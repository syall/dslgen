# Session A13 — `calcc` CLI surface

Spec refs: spec.md §11 (generated-compiler CLI), §9.1 (interpreter is not the default
path). Roadmap: roadmap.md "A13". Prereqs A5, A12 landed (`87b8c2e`, `da3e0d3`).

## Context

`calcc` today (`crates/calc-compiler/src/main.rs`) hand-parses `env::args()` with two
slice patterns: `run --interpret <path>` and `build [--backend=<n>] <path> -o <out>`.
Argument order is rigid (`-o` must come last, `--backend` must come first), there's
no `--help`, and there's no `check`. A13's deliverable: `calcc build`,
`calcc run --interpret`, `calcc check` all working per §11, with `--backend=<name>`
threaded through, using a real CLI crate. A12's DECISIONS entry also explicitly
scheduled `--verbose`/`--keep-object` "→ A13's CLI".

## Plan

1. **Add `clap` (derive feature)** to `calc-compiler/Cargo.toml` (4.x; already in the
   local registry cache). Derive over builder: the CLI is a static shape, and doc
   comments become `--help` text for free.

2. **Rewrite `main.rs`'s argument handling** as a `#[derive(Parser)] struct Cli` with
   a `#[derive(Subcommand)] enum Command`:
   - `Build { #[arg(long)] backend: Option<String>, path: PathBuf, #[arg(short='o')] output: PathBuf, #[arg(long)] verbose: bool, #[arg(long)] keep_object: bool }`
   - `Run { #[arg(long, required = true)] interpret: bool, path: PathBuf }` —
     `--interpret` stays mandatory: §9.1 says the interpreter is explicitly *not*
     the default execution path, and A15 will add `--hot-reload` as the second mode
     (then an `ArgGroup`, not before).
   - `Check { path: PathBuf }`.
   - `--backend` stays a plain `Option<String>` passed to the existing
     `backend::select` (`src/backend.rs`), not a clap `value_parser`/`ValueEnum`:
     `select` already distinguishes "unknown backend" from "known but compiled out,
     rebuild with `--features backend-llvm`" (A8), which clap's generic
     possible-values error would lose. Its `--help` text is a static doc comment
     ("cranelift or llvm, among those compiled in; optional when only one is");
     a wrong value gets `select`'s message, which lists what's actually enabled.
   - Delete `usage()` and `parse_build_args()`; clap handles usage errors (exit code
     2, distinct from compile failures' exit code 1 — the standard convention).

3. **Split the shared front end** in `main.rs`: `compile_to_ir(path)` becomes
   `parse_and_resolve(path) -> Option<Expr>` (read, parse, resolve, diagnostics as
   today) plus `lower` at the two call sites that need IR. `check` = 
   `parse_and_resolve` only; silent with exit 0 on success, diagnostics + exit 1 on
   failure (Unix convention, like `cc -fsyntax-only`). "Typecheck" in §11 has
   nothing to do in calc-lang yet (one value type, `f64`) — resolve is the only
   semantic check that exists; noted, not invented.

4. **`--verbose` / `--keep-object`** — `build`-only flags (the only subcommand that
   runs external programs or writes intermediate files). `--verbose` narrates, on
   stderr (stdout keeps `wrote …` / `note: …`), every step where `calcc` makes a
   choice or runs another program:
   - `main.rs`: which backend `select` picked (matters when it was implicit).
   - `runtime_deps::prepare(program, verbose)`: each probe command it runs
     (`python3 --version`) and each bundle it packs (name, file count, bytes).
   - `link::link`: new `pub struct LinkOptions { pub verbose: bool, pub keep_object: bool }`
     (`Default`) as a 4th parameter; `verbose` prints the existing
     `command_line(&cmd)` before running the linker.
   `--keep-object` only touches `link.rs`: skip the two `TempFile` cleanups (program
   object + bundles object) and print their paths. Other `link`/`prepare` callers
   (backend and link.rs tests) pass defaults.
   Not threaded: `check` (runs nothing), `run --interpret` (its only subprocesses
   are IPC built-ins spawned inside `calc-runtime`, shared with compiled
   executables, with no channel for a flag — deferred).

5. **Tests** — new integration test `crates/calc-compiler/tests/cli.rs` driving the
   real binary via `env!("CARGO_BIN_EXE_calcc")` against temp `.calc` files:
   - `check` on a valid program → exit 0, no stdout; on an unresolved identifier →
     exit 1, `resolve error` on stderr; on a parse error → exit 1.
   - `run --interpret` prints the result; `run` without `--interpret` → exit 2.
   - `build` with `-o` before the path (order independence) produces a runnable
     executable whose exit/output matches; `--backend=nope` → exit 1 with
     `unknown backend`; `--keep-object` leaves the `.o`/`.obj` behind; `--verbose` puts the
     backend name and link command on stderr.
   - `--help` lists all three subcommands.
   (Build tests gated `#[cfg(feature = "backend-cranelift")]` like existing recipes,
   since with several backends enabled `build` needs an explicit `--backend`.)

6. **Docs**: `main.rs` module doc updated; new
   `calc-lang/docs/a13-calcc-cli-surface.md` (what a compiler driver is; subcommands
   as composable pipeline prefixes — `check` ⊂ `run --interpret` ⊂ `build`, a
   diagram of which stages each runs; clap derive explained from scratch; exit-code
   conventions; why `--backend` validation stays in `backend::select`; `--verbose`
   showing the real link line; "try it" section); numbered entry 14 in
   `docs/README.md`.

7. **DECISIONS.md A13 entry**: clap derive vs builder vs keep hand-rolling;
   `--backend` validated by `backend::select`, not clap; `--interpret` mandatory per
   §9.1 with `--hot-reload` left to A15; `check` = parse + resolve (no type system
   to check yet); `-o` stays required (spec's shape; no default output name
   invented); `--verbose`/`--keep-object` picked up from A12's deferral.

8. **plan.md** overwritten with this plan + Outcome.

## `run`'s shape, now and later

| Session | Command | Engine | Flags |
|---|---|---|---|
| A13 | `run --interpret <path>` | `calc_ir::interpret` over the IR; prints the result | `--interpret` required |
| A15 | `run --hot-reload <path>` | Cranelift JIT + file watcher | `--interpret` / `--hot-reload` become a required, exclusive clap `ArgGroup` |

Bare `run` stays a usage error (exit 2), per §9.1 — and it leaves room for a
possible future default (e.g. `cargo run`-style build-to-temp-and-execute) without
breaking anyone. `run` takes no `--backend`: the interpreter has none, and
hot-reload is Cranelift-only by spec (§9.2), so A15 errors if Cranelift is compiled
out rather than accepting a flag with one legal value.

## Other `LinkOptions` fields considered and left out

`LinkOptions` gets exactly `verbose` + `keep_object`. Each of these either already
works another way or belongs to a scheduled session (DECISIONS entry records them):
- choice of linker: already set by the `CC` env var through the `cc` crate
  (`link.rs`'s error already says "point CC at one"); no `--linker` flag.
- target triple: C8 (cross-compilation) adds it, since it also changes codegen.
- static vs. dynamic FFI libraries: C7, set per built-in in the manifest, not a CLI flag.
- extra library search paths / swapping implementations: C2 (external overrides).
- raw extra linker arguments (`--link-arg`): not in §11 and no use for it yet.
- debug info / strip: the backends don't produce debug info yet.
- where temporary files go: stays next to the output file, where `--keep-object`
  leaves them and a user would look for them.

## Deliberately not done

- `--hot-reload` (A15), line:column diagnostics (not in §11; byte offsets stay),
  default `-o` name, `dslgen` CLI (B8).

## Verification

`cargo build && cargo test` while iterating; then the full CLAUDE.md §4 sequence
(`cargo fmt`, `check`, `clippy --all-targets -D warnings`, `doc -D warnings`,
`audit`, `build`, `test`) from `calc-lang/`. Manual smoke: `calcc --help`,
`calcc check`, `calcc run --interpret`, `calcc build --verbose -o out prog.calc`
then run `out`.

## Outcome

Implemented as planned. Small departures, none changing behavior:
- `#[command(bin_name = "calcc")]` added so usage lines read `calcc`, not `calcc.exe`,
  on Windows.
- `--keep-object` prints `kept <path>` on stdout for both object files, and does so
  before the linker runs, so the paths are shown even when linking fails.
- The `build` tests in `tests/cli.rs` are gated on "exactly one backend enabled"
  (`not(all(cranelift, llvm))`) rather than Cranelift-only, so CI's LLVM-only job
  runs them too.

`cargo test -p calc-compiler` passes for all three feature combinations (default
Cranelift-only, `--no-default-features --features backend-llvm`, and
`--features backend-llvm`); see the full pre-commit run below.
