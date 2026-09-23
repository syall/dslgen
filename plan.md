# Session A12 — The link driver

Spec refs: spec.md §4 (`src/link.rs`), §7 (linking model, run-time deps), §3/§8.1
(platform scope). Roadmap: roadmap.md "A12". Prereqs A6/A7, A10, A11 landed.
Baseline: `main@26e85bb` (spec/roadmap now schedule dynamic FFI → C7, wasm32 →
C1-wasm (optional), cross-compilation → C8 (required); skill counts 35 required).

## Plan

### Context

A12's literal deliverable ("`calcc build` → runnable executable, all three kinds at
once") already works since A11, through ad hoc machinery: `calc-compiler/build.rs`
reaches into `calc-runtime`'s files (bare-`rustc` build of `src/lib.rs` — forcing
every built-in to be dependency-free — plus a second `cc` build of `calc_ffi.c`);
manifest and implementations are fused in one crate; `link_stub.rs` hardcodes MSVC
system/CRT libs found by trial and error.

Goals agreed in planning: one concrete, simple dependency story for all three kinds
so a user of the compiler can swap in their own implementation; linking = object +
each implementation's archive + its explicitly listed deps, nothing hardcoded in
`link.rs`; run-time deps and platform tiers stated; shaped for B6's
`bindings.toml`; implementations and packaging not tied into compiler
infrastructure; a strong linker story (pre-link symbol check, documented link
order, actionable failures).

### The dependency model (one rule, three kinds)

Every built-in = a C-ABI `symbol` provided by a **link unit** (static archive +
explicit link-dep list), plus optional **run-time requirements**. `link.rs`:
`object + every link unit + their deps` → executable.

| Kind | Link unit | Link deps | Run-time deps | Swap in your own |
|---|---|---|---|---|
| Native Rust | runtime archive (Cargo-built `calc-runtime` staticlib) | **derived** by rustc (`native-static-libs`) | none | edit the fn / add a Cargo dep in `calc-runtime` |
| C-ABI FFI | its **own** archive, named in the manifest (`calc_ffi`) | **declared** in the manifest | none (static; dynamic is C7) | a library exporting the symbol + its listed deps |
| IPC | runtime archive (the shim) | derived (same list) | **declared**: commands (`python3`, `python`) | a command speaking the JSON protocol |

A prebuilt *Rust* staticlib counts as FFI (two Rust staticlibs in one executable
duplicate `std`), so native Rust means "compiled into the one runtime archive".

### Architecture (shipped as SVGs in the A12 teaching doc)

- **Current**: `calc-compiler/build.rs` reads `calc-runtime`'s files; manifest +
  impls fused in `calc-runtime`; `link_stub.rs` hardcodes system libs.
- **Proposed**: `calc-compiler` → `calc-ir`, `calc-runtime-artifacts`,
  `calc-builtins`; `calc-ir`/`calc-runtime-artifacts` → `calc-runtime` →
  `calc-builtins`. Link inputs: `calc_runtime` archive + rustc-derived deps file +
  `calc_ffi` archive (deps from manifest) → `link.rs` → program (run-time needs
  reported).

### Crate layout after A12

- **`calc-builtins`** (new) — manifest data only; no deps, no build script:
  `Builtin { name, symbol, arity, kind }`, `BindingKind::{NativeRust, Ffi { library,
  link_deps }, Ipc { commands }}`, `LinkDep { name, only_on: Option<&str> }`
  (`"windows"`/`"unix"`, Rust's `target_family` vocabulary), `BUILTINS`, `lookup`,
  `Builtin::runtime_requirements()` (kind-generic; IPC → its commands). The
  in-memory form B6 will parse `bindings.toml` into; C7 adds a `link` field to
  `Ffi` and a dynamic-library requirement to the same method.
- **`calc-runtime`** — implementations only: `calc_add`/`calc_mul`, `extern "C"
  calc_sub`, `calc_print`, `ipc_runtime.rs` (now `serde_json`), `native/`.
  `pub fn eval(name) -> Option<fn(&[f64]) -> f64>` calls the real implementations
  (never copies); a test asserts its names equal `calc_builtins::BUILTINS`'s.
  `Cargo.toml`: `links = "calc_runtime"`, default feature `link-ffi`. `build.rs`
  (only with `link-ffi`): compiles/links `calc_ffi.c` for Rust consumers (dynamic
  CRT, as today) **and** a static-CRT copy into `$OUT_DIR/link-units/`, announced
  as `cargo:link_units_dir=…` → `DEP_CALC_RUNTIME_LINK_UNITS_DIR`. The single place
  that knows how `calc_ffi.c` is built.
- **`calc-runtime-artifacts`** (new) — depends on `calc-runtime` (for metadata).
  `build.rs`: nested `$CARGO rustc -p calc-runtime --lib --crate-type staticlib
  --release --locked --no-default-features --target $TARGET --target-dir
  $OUT_DIR/runtime-target -- --print native-static-libs=$OUT_DIR/calc_runtime.link-deps`
  (separate target dir → no lock deadlock; `--no-default-features` → archive only
  *references* `calc_sub`; env hygiene: `CARGO_PROFILE_RELEASE_PANIC=abort`, remove
  `RUSTC_WORKSPACE_WRAPPER`/`RUSTFLAGS`/`CARGO_TARGET_DIR`, set
  `CARGO_ENCODED_RUSTFLAGS=-Ctarget-feature=+crt-static` on `*-msvc` so rustc's
  list names the static CRT itself; fallback if `--print …=path` misbehaves: parse
  the `native-static-libs:` note from stderr). `src/lib.rs`: `RUNTIME_LIB`,
  `RUNTIME_LINK_DEPS` (file contents via `include_str!`), `LINK_UNITS_DIR`,
  `TARGET`. Generic — Part B lifts it into `dslgen-backend`.
- **`calc-compiler`** — **no `build.rs`**, no `cc` build-dependency.
  `link_stub.rs` → **`link.rs`**:
  - Inputs in documented order: object → runtime archive → every manifest FFI
    `library` (dedup, from `LINK_UNITS_DIR`) → derived runtime deps + FFI
    `link_deps` filtered by `only_on`. MSVC: `.lib` inputs, `/…` options after
    `/link`; Unix: `-L` + `-l`. **No library names in `link.rs`.**
  - `enum LinkFlavor { Msvc, Unix }` from `cc`'s classification; anything else →
    `unsupported toolchain` error (C1-wasm adds `WasmLd`).
  - Target triple chosen in exactly one place (`calc_runtime_artifacts::TARGET`,
    = host today); C8 threads `--target` through it.
  - **Pre-link symbol check**: for each built-in the program uses, its symbol must
    be exported by its own link unit and by no other (promote the test-only
    `archive_symbols` parser; BSD/Mach-O tables skipped with a note). Diagnostics
    like `built-in "sub" needs calc_sub from calc_ffi (<path>): not exported`.
  - **Actionable failures**: linker error includes the full command line and the
    linker's captured output.
  - `calcc build` prints run-time requirements of built-ins the program uses (IR
    walk), e.g. `note: needs at run time: python3 or python on PATH (built-in "print")`.

### Platform tiers (in `link.rs` docs + teaching doc)

- **Tested**: x86_64 Windows MSVC (local), x86_64 Linux GNU (CI).
- **Supported by construction, untested**: macOS, Windows MinGW, clang-cl, *BSD,
  aarch64 Linux (Cranelift only there — LLVM is `target-x86`-only until C8).
- **Not in A12**: cross-compilation (→ C8), wasm32 (→ C1-wasm, optional), dynamic
  FFI linking (→ C7), hosts with no C toolchain. Linker driver override: `CC` /
  `CC_<target>` (honored by `cc`), documented.

### Implementation order

1. `calc-builtins`: move manifest data out of `calc-runtime/src/lib.rs`; switch
   backends (`cranelift_backend.rs`, `llvm_backend.rs`) and `calc-ir`
   (`interp.rs`, `ir.rs` doc refs) to it; add to workspace members.
2. `calc-runtime`: `eval` table; `links` + `link-ffi` feature + link-units output;
   `serde_json` IPC (`{"args":[x]}` on stdin → `{"output","result"}` on stdout;
   `print.py` uses stdlib `json`; interpreter candidates from the manifest's
   `Ipc { commands }`); drop "must stay self-contained" docs.
3. `calc-runtime-artifacts`: nested build + derived deps + exported consts.
4. `link.rs` (order, flavors, pre-link check, failures) + `calcc` wiring (run-time
   note); delete `calc-compiler/build.rs` and its `cc` build-dep; update
   `link_stub` references in code/comments.
5. Tests: every built-in's symbol exported by its own link unit; runtime archive
   does *not* define `calc_sub`; deps file non-empty; pre-link check rejects a
   missing and a duplicated symbol (hand-built archives); `only_on` filtering;
   `eval` ↔ manifest names; run-time-requirement collection; end-to-end
   `{ print(2 * 3 + 4 - 1); 7 }` → stdout `result = 9.00`, exit 7, per backend.
6. Docs (A12 only — **no edits to a0–a11 pages or earlier DECISIONS entries**):
   `calc-lang/docs/a12-the-link-driver.md` (object → exe: undefined symbols,
   archive member extraction, CRT startup, system libs; section on archive symbol
   tables + the pre-link check; section on link order in single-pass linkers and
   what breaks if FFI precedes the runtime archive; the model table + per-kind
   "swap in your own"; crate layout; platform tiers; a note that earlier pages'
   `link_stub.rs` is now `link.rs`) with `docs/images/a12-architecture-{current,
   proposed}.svg`; README.md entry; CLAUDE.md workspace-layout lines; DECISIONS.md
   A12 entry (layout alternatives A–E → B+C+E; derived vs declared deps; FFI as
   its own unit; serde_json proves the constraint is gone; deferred: C7, C8,
   C1-wasm, C2 overrides, embedding the archive so `calcc` works outside its build
   tree, A13's `--verbose`/`--keep-object`; A7's x86-only deferral now owned by
   C8); `plan.md` overwritten.
7. At sync time (step 9): the dashboard and the `docs` branch already show the
   35-session required path with a C8 row (synced to `main@26e85bb` separately), so
   only A12 → done and A13 → next remain.

Open, not A12's to resolve: B10 is required but its prereqs need A1-pest and/or
A1-custom (optional).

### Verification

`cargo build && cargo test` from `calc-lang/` while iterating; then the full
CLAUDE.md §4 sequence (fmt, check, clippy `--all-targets -D warnings`, doc
`-D warnings`, audit, build, test) plus build/test with `--features backend-llvm`.
Manual: inspect `calc_runtime.link-deps`; `calcc build` with both backends on an
all-three-kinds program → run it, check stdout, exit code and the run-time note;
swap test (point `calc_sub` at a different C file, confirm the executable uses it);
remove `calc_ffi` from the link to see the pre-link diagnostic; confirm `cargo
clippy`/`cargo doc` don't deadlock on the nested build and `cargo test -p calc-ir`
doesn't trigger it.

### Post-review addition: IPC run-time dependencies and bundling

Approved after the plan above was implemented and reviewed.

#### Context

Review of the implemented A12 surfaced two flaws in the IPC kind's dependency story:
(1) it only works for a single-file implementation — `print.py` is embedded with
`include_str!` and run via `python -c`, so a multi-file project (several modules, its
own packages) has nowhere to live at run time, and "python on PATH" under-reports
what it needs; (2) `commands = ["python3", "python"]` tried in order at run time is a
guess: which one runs is decided on the target machine, a broken dependency (the
Windows `python3` alias stub) is masked instead of reported, and spec.md §7 wants an
unreachable IPC executable surfaced at `calcc build` time. User chose to fix both in
A12, including bundling.

#### Model

An IPC implementation = **one executable invocation speaking the JSON protocol**,
plus optionally a **bundle**: a directory of files the invocation needs, shipped
inside the produced executable (embedded by the link driver). The program's own
package dependencies stay the responsibility of whatever it is (a venv, `pipx`, a packaged binary); calcc installs
nothing (spec §3's stance for toolchains).

Manifest (`calc-builtins`), replacing `Ipc { commands }`. Nothing in it is
Python-specific; everything that varies by language *or* platform lives in one
`IpcCommand` per platform:

```rust
Ipc {
    // Exactly one applies per target family (validated).
    commands: &[
        IpcCommand { program: "python3", args: &["{bundle}"], env: PYTHON_ENV,
                     probe: Some(&["--version"]), only_on: Some("unix") },
        IpcCommand { program: "python",  args: &["{bundle}"], env: PYTHON_ENV,
                     probe: Some(&["--version"]), only_on: Some("windows") },
    ],
    bundle: Some("print"),        // directory name under calc-runtime's `ipc/`
}
```

- `program`, `args` and `env` values may contain `{bundle}`, replaced with the
  bundle directory at run time. `program` may point *into* the bundle, so a
  compiled helper needs no interpreter at all.
- `args`/`env` are per command, not shared, because they legitimately differ by
  platform (Java's classpath separator is `:` on Unix and `;` on Windows; a native
  helper is `print` vs `print.exe`).
- `env` covers runtimes that locate code by environment (`PYTHONPATH={bundle}/vendor`,
  `NODE_PATH`, ...).
- `probe: Option<args>` runs at `calcc build`; `None` for a program inside the bundle,
  whose check is "the file exists in the bundle".
- `PYTHON_ENV` is `[("PYTHONDONTWRITEBYTECODE", "1")]`: running the bundle from its
  source directory otherwise writes `__pycache__/` into it, which would get packed
  (found during implementation; see Outcome).

How other languages fit, with no new fields (documented in the teaching doc, not
tested — CI shouldn't need a JDK or Node):

| Implementation | `program` | `args` | `probe` | bundle contains |
|---|---|---|---|---|
| Python project | `python3` / `python` | `{bundle}` | `--version` | `__main__.py`, modules, optional `vendor/` (+ `env PYTHONPATH`) |
| Node.js | `node` | `{bundle}/index.js` | `--version` | `index.js`, `node_modules/` |
| Java | `java` | `-cp`, `{bundle}/print.jar`, `Main` | `-version` | `print.jar` (deps shaded in, or `lib/*` with a per-platform separator) |
| Native helper (Go, C, Rust...) | `{bundle}/print` / `{bundle}/print.exe` | — | none | the binary |

The language-neutral contract (written down in the doc): read one UTF-8 JSON request
`{"args": [...]}` from stdin until EOF, write one UTF-8 JSON response `{"output":
"...", "result": ...}` to stdout, exit 0; non-finite numbers as strings; stderr is
free for diagnostics. Deliberately *not* calcc's job: producing the bundle (`javac`,
`npm ci`, `pip install --target vendor`) — the bundle is copied as-is, so it must
already be built — and version constraints beyond what a probe checks (an author
wanting "Java 17+" writes a probe that fails otherwise). Considered and left out: a
per-command working directory (runtimes locate their own files relative to the
script/jar; changing cwd would break programs reading user-relative paths). Bundle
copying follows symlinks (e.g. `node_modules/.bin`), since creating symlinks on
Windows needs privileges. Bundles are packed file by file; symlinks are followed.

- `print` becomes a real **multi-file project**: `calc-runtime/ipc/print/__main__.py`
  (protocol handling) + `formatting.py` (the f-string), run as `python3 <dir>` (Python
  runs a directory's `__main__.py` with that directory on `sys.path`). Replaces
  `native/print.py` and the `include_str!`.
- `Builtin::runtime_requirements(target_family) -> Vec<RuntimeRequirement>` with
  `Command(program)` ("python3 on PATH"; omitted when `program` is inside the bundle)
  and `BundleCache(name)` ("a writable per-user cache directory to unpack into");
  FFI/native → none. Exact, no alternatives.

#### Where the bundle lives: embedded in the executable (user's choice)

- **Compiled program**: the bundle is **linked into the executable as data**.
  `calcc build` packs each used IPC built-in's bundle into one blob; the link driver
  generates a small **data-only object file** (via the `object` crate: COFF/ELF/Mach-O
  picked from the target triple) defining two symbols, `calc_ipc_bundles` (the bytes)
  and `calc_ipc_bundles_len`, and links it like any other input. No relocations
  needed (a flat blob). Code signing is unaffected, since the data is part of the
  image rather than appended.
- **At run time** the shim reads the blob through those symbols and, on first use,
  **unpacks** the bundle to a per-user, content-addressed cache directory —
  `<cache>/calc-bundles/<name>-<hash>/`, where `<cache>` is `%LOCALAPPDATA%` on
  Windows, `$XDG_CACHE_HOME` or `~/.cache` on Unix, or `CALC_BUNDLE_CACHE` if set
  (an explicit, documented override, e.g. for `noexec` home dirs). Unpacking goes to
  a temporary sibling then `rename`s into place, so concurrent first runs don't
  clash; a present directory is reused (its name is the content hash, so it's never
  stale). Unix file modes are restored so a bundled native helper stays executable.
  Per-user (not shared `/tmp`) so no other user can pre-plant files at the
  predictable path.
- **Interpreter** (`calcc run --interpret`, tests): no packing — `eval("print")`
  runs the same shim on the source directory (`calc-runtime/ipc/print`, baked in via
  `CARGO_MANIFEST_DIR`). Same shim, same Python project, only the location differs,
  by explicit rule.
- **Rust consumers** (`calcc`, test harnesses) also link `calc_print`, which
  references `calc_ipc_bundles`. `calc-runtime`'s build script (feature renamed
  `link-ffi` → `link-natives`) compiles a tiny `native/no_ipc_bundles.c` defining an
  empty table for them, next to `calc_ffi`. The nested staticlib build turns the
  feature off, so compiled programs get the table only from the generated object.
- **One pack format, one place**: a small documented binary format (magic, bundle
  count; per bundle: name, content hash, files as path/mode/bytes; little-endian)
  implemented once in `calc-builtins::bundle_format` (pure functions, no deps), used
  by `calcc` to pack and by the shim to unpack. Hash: FNV-1a 64 over the packed
  bundle, computed at pack time and stored — only a cache key, not a security check.

#### `calcc build` flow (new module `calc-compiler/src/runtime_deps.rs`)

Before linking, for each IPC built-in the program actually uses (existing IR walk):
pick the command for the target family; **probe** it (error naming the built-in,
command, and exit status/output — the Windows `python3` stub now fails the build
instead of being skipped), or, if `program` is inside the bundle, check that file
exists; **pack** its bundle from `calc_runtime_artifacts::IPC_BUNDLES_DIR/<name>`.
Then `link::link(object, &packed_bundles, out)` — link.rs gains only the generic
"emit a data object for this blob and add it to the inputs" step. Finally print the
exact requirements, e.g.

```text
note: needs at run time: `python3` on PATH (built-in `print`)
note: needs at run time: a writable per-user cache directory (or CALC_BUNDLE_CACHE) to unpack bundle `print` into on first run: 2 files, 1023 bytes (built-in `print`)
```

#### Tests

- `calc-builtins`: exactly one `IpcCommand` applies per family; requirement lists;
  `bundle_format` pack → parse round trip, including modes and an empty blob.
- `calc-runtime`: the shim runs the multi-file project from the source root (JSON
  round trip, non-finite values); `{bundle}` substitution in program/args/env;
  unpacking into a temp cache (`CALC_BUNDLE_CACHE`) creates the files once and
  reuses them on the second call.
- `calc-compiler`: the generated data object parses (via `object::read`) and defines
  both symbols for this target; probe failure is reported by built-in (fake command);
  a program without IPC built-ins embeds an empty table; the end-to-end
  all-three-kinds test runs the executable **with no files next to it**, using a test
  cache dir, and checks the bundle landed there.
- Backend tests (`cranelift_backend.rs`, `llvm_backend.rs`) pass an empty bundle blob.

#### Docs / bookkeeping

A12 teaching doc: new section "IPC implementations bigger than one file": the model,
the language-neutral protocol contract, the Python/Node/Java/native table, why one
command per platform plus a build-time probe instead of a run-time fallback, what
calcc deliberately doesn't install, and embedding — the pack format, the generated
data object as a link input, unpacking to a per-user content-addressed cache, and the
alternatives (appending: breaks code signing; a directory beside the executable: two
things to ship). Protocol, run-time-requirements and Try-it sections updated; the
"after" SVG gains the generated bundle object as a link input. README entry line
updated. DECISIONS.md A12 entry gains an IPC paragraph (alternatives: embedded string
only / zipapp / reference an installed launcher / directory beside the executable /
append / linked data object; chose linked data object + one command per platform +
probe). `plan.md` Outcome notes the post-review change.

#### Verification

Full CI command set again (all four jobs' commands). Manual: `calcc build` the
all-kinds program; copy **only** `prog.exe` to another directory and run it (works;
the cache dir gains `calc-bundles/print-<hash>/{__main__,formatting}.py`); run again
(reuses it); change `formatting.py`, rebuild, run (new hash dir); set
`CALC_BUNDLE_CACHE` and confirm it's honored; temporarily set the Windows command to
`python3` to confirm the probe fails `calcc build` with a clear message; `cargo test
-p calc-ir` still doesn't trigger the nested build.

## Outcome

Implemented as planned. Every command in `.github/workflows/agent-evals.yml` was run
locally and passes: the `checks` job (`fmt --check`, `check`, `audit`, `build`),
`test-cranelift` (clippy `-D warnings`, doc `-D warnings`, test), `test-llvm`
(`-p calc-compiler --no-default-features --features backend-llvm`: clippy, doc, test)
and `test-all-features` (clippy, doc, test with `--all-features`). Only on Windows,
though: no Linux environment was available locally, so the Linux path (GNU archive
names, rustc's `-l...` dependency list, `cc`-style link line) is exercised for the
first time by CI. No deadlock under `cargo clippy`/`cargo doc`. On this machine rustc derived
`kernel32.lib ntdll.lib userenv.lib ws2_32.lib dbghelp.lib /defaultlib:libcmt` for the
runtime archive, and `cl.exe` links with that list alone: `libvcruntime`/`libucrt`,
previously hardcoded, come in through `libcmt`'s own default-library directives.
Manual checks: `calcc build` on an all-three-kinds program (both backends) prints the
run-time note and runs correctly; replacing the `calc_ffi` archive with the runtime
archive produces the pre-link diagnostics (three duplicated symbols, one missing);
changing `calc_sub` in `calc_ffi.c` changes the compiled program's output with no
`calc-compiler` change.

Departures, all small:

- **The pre-link check covers every manifest built-in, not just the ones a program
  calls.** While implementing, it turned out the runtime archive's interpreter table
  (`calc_runtime::eval`) references every built-in's symbol, so every link unit must
  be present and consistent whichever built-ins a program uses. Checking all of them
  is the accurate check, and it kept `link::link`'s signature unchanged (no program
  or used-built-in list needed). The run-time-requirements note does use the
  program's actual built-ins, via an IR walk in `main.rs`.
- **Post-review: IPC built-ins reworked** per the addition above — one program
  speaking the JSON protocol plus an optional bundle, one command per platform
  probed at build time, bundles embedded as a linker-generated data object and
  unpacked to a per-user cache. `Builtin::runtime_requirements(family)` returns a
  list (`CommandOnPath`, `BundleCache`). One real finding while implementing it:
  running the bundle from its source directory made Python write `__pycache__/`
  there, which then got packed into executables; fixed by declaring
  `PYTHONDONTWRITEBYTECODE=1` in `print`'s manifest `env`, not by teaching `calcc`
  Python-specific excludes. Verified manually: building, copying only `prog.exe` to
  an empty directory and running it works; a second run reuses the unpacked bundle;
  changing `formatting.py` produces a new `print-<hash>` directory; the default cache
  is `%LOCALAPPDATA%\calc-bundles`; pointing the Windows command at `python3` makes
  `calcc build` fail with the alias stub's own message.
- **Doc images are named `a12-architecture-{before,after}.svg`**, not
  `{current,proposed}`, since the doc describes the change after it landed.
- **The target family for `LinkDep::only_on`** comes from parsing
  `calc_runtime_artifacts::TARGET` with `target-lexicon`, not from `cfg!`. That keeps
  the "target chosen in one place" seam for C8.
- **Added a `manifest_dir` key** to `calc-runtime`'s published `links` metadata, so
  `calc-runtime-artifacts` needs no relative path to it at all. The plan only named
  `link_units_dir`.
- **The artifacts crate also re-runs when the workspace `Cargo.lock` changes**: the
  nested build's dep-info lists only path dependencies' sources, not registry crates.
- **The nested build removes the outer build's `CARGO_ENCODED_RUSTFLAGS`** instead of
  setting it to an empty string on non-MSVC targets, so an inherited value can never
  leak in and an empty value never has to be interpreted.
- **spec.md and roadmap.md brought in line with what A12 built**, at the user's request:
  spec §4's workspace layout gains `calc-builtins`, `calc-runtime-artifacts` and
  `runtime_deps.rs`, and its run-time note mentions the bundle cache; §7 describes the
  IPC model (one program speaking the protocol, an optional bundle embedded in the
  executable, one command per platform with a build-time probe); §7.1's example
  `bindings.toml` uses that shape and explains `target` (which implementation) vs.
  `only_on` (how it's invoked or linked per platform); §14 #5 records calc-lang's
  working protocol while leaving it open for generated compilers. roadmap.md's B2
  deliverable now also moves `link.rs`, `runtime_deps.rs`, the bundle format and the
  archive building into `dslgen-backend`.
- **`{bundle}` consistency, added after review**: `IpcCommand::uses_bundle()` (does
  `program`, `args` or `env` refer to the bundle) must match whether the built-in
  declares a bundle, checked by a manifest test and by `calcc build`; a program inside
  the bundle is now also probed when it declares a probe, after the file-exists check.
  `runs_from_bundle()` deliberately still looks only at `program`, since it decides
  whether the executable must be on `PATH`.
- **Tests:** A11's separate IPC link test was folded into the new all-three-kinds
  end-to-end test, and a test was added for the linker-failure message
  (`a_linker_failure_shows_the_command_and_its_output`).
