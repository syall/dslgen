# A10 — Built-ins, kind 2: C-ABI FFI

**Session code**:
[`crates/calc-runtime/native/calc_ffi.c`](../crates/calc-runtime/native/calc_ffi.c),
[`crates/calc-runtime/build.rs`](../crates/calc-runtime/build.rs),
[`crates/calc-runtime/src/lib.rs`](../crates/calc-runtime/src/lib.rs),
[`crates/calc-compiler/build.rs`](../crates/calc-compiler/build.rs),
[`crates/calc-compiler/src/link_stub.rs`](../crates/calc-compiler/src/link_stub.rs),
[`crates/calc-ir/src/ast_to_ir.rs`](../crates/calc-ir/src/ast_to_ir.rs).
**Spec refs**: spec.md §7 (FFI half of built-in bindings). **Prereqs**:
[A9](a9-native-rust-builtins.md).

A9 gave calc-lang its first built-in kind: `add`/`mul`, written in Rust, called by
compiled code and by the interpreter alike. A10 adds the second kind spec.md §7
describes — **C-ABI FFI**: a built-in whose implementation is written in some other
language and linked in as its own library. This page assumes A9's page for the basics
(ABI, symbols, linkers, `is_pic`) and focuses on what's actually different here: a real
C file, and a design mistake caught and fixed before it shipped.

- [What changed, in one paragraph](#what-changed-in-one-paragraph)
- [The C file, and declaring it from Rust](#the-c-file-and-declaring-it-from-rust)
- [The mistake: a duplicated `eval`, and why it matters](#the-mistake-a-duplicated-eval-and-why-it-matters)
- [Why `calc-runtime` needs its own `build.rs` now](#why-calc-runtime-needs-its-own-buildrs-now)
- [Two archives, two build scripts, one source file](#two-archives-two-build-scripts-one-source-file)
- [A real linker warning: the MSVC CRT mismatch](#a-real-linker-warning-the-msvc-crt-mismatch)
- [What it costs, and what's deliberately not here](#what-it-costs-and-whats-deliberately-not-here)
- [Try it](#try-it)

## What changed, in one paragraph

Same trick as A9: calc-lang still has no call syntax, so `-` becomes the new built-in's
surface. `ast_to_ir::builtin_for` now maps `Sub` to `"sub"` alongside `Add`→`"add"` and
`Mul`→`"mul"`; only `/` (`Div`) is left as an inline `BinOp`. The only calc-lang-specific
line that changed is that one `match` arm:

```rust
fn builtin_for(op: BinOp) -> Option<&'static str> {
    match op {
        BinOp::Add => Some("add"),
        BinOp::Mul => Some("mul"),
        BinOp::Sub => Some("sub"),   // was `None` through A9
        BinOp::Div => None,
    }
}
```

Everything downstream of that — the `CallBuiltin` IR node, both backends' codegen, the
linker's symbol resolution — is unchanged from A9, because none of it was ever specific
to *how* a built-in is implemented. That genericity is the entire point of A9's design,
and this session is the first real test of it: `sub`'s implementation lives in a
different language, in a different file, built by a different tool, and nothing outside
`calc-runtime` needs to know.

## The C file, and declaring it from Rust

[`native/calc_ffi.c`](../crates/calc-runtime/native/calc_ffi.c) is three lines:

```c
double calc_sub(double a, double b) {
    return a - b;
}
```

No `extern "C"`, no attribute — in C, this *is* the normal way to write a function, and
C's calling convention on a given platform is simply *the* C ABI, the same one A9's
`extern "C"` asked `rustc` to opt into. There's nothing to opt into from the C side.

For Rust code to call it, `calc-runtime/src/lib.rs` declares it without defining it:

```rust
extern "C" {
    fn calc_sub(a: f64, b: f64) -> f64;
}
```

This is `extern "C"` used in the *other* direction from A9: there, it told `rustc` "make
`calc_add` callable from non-Rust code that expects the C ABI." Here, it tells `rustc`
"there exists a function elsewhere, following the C ABI, named `calc_sub` — trust me,
and let the linker find it." Calling it is `unsafe`, because `rustc` has no way to check
that a linked C function actually behaves the way its Rust signature claims:

```rust
eval: |args| unsafe { calc_sub(args[0], args[1]) },
```

If the C function didn't exist at all, or its return type didn't match, this would be a
link-time error, not a compile-time one — the entire reason FFI exists is to cross a
boundary the compiler can't see across.

## The mistake: a duplicated `eval`, and why it matters

The first version of this session's plan gave `sub` this `eval` instead:

```rust
eval: |args| args[0] - args[1],   // rejected before it was written
```

This compiles, passes every test, and is *wrong* in a way tests wouldn't have caught,
because for subtraction specifically, hand-writing the same one-line formula twice is
never going to disagree with itself. But it's a second implementation of what `sub`
means, sitting next to the real one, and A9's own justification for `calc-runtime`
existing at all was "when two implementations exist, they can disagree; when there is
one source, they can't." A duplicated `eval` throws that away for exactly the built-ins
where it matters most — not `sub`, but whatever comes next. A11's `print` built-in will
be backed by a small Python script; there's no faithful, safe way to hand-write "what
running `python3 print.py 3.14` produces" as a pure Rust closure.

So the rule this session settles on, for every built-in kind from here forward:
**`eval` always calls the one real implementation. Never a rewritten copy of it.** For a
native-Rust built-in this was already true by construction (A9's `eval` calls the same
`calc_add` compiled code calls). For an FFI built-in it takes a bit more machinery —
the rest of this page is that machinery. For A11's IPC built-ins it will turn out to be
*less* machinery than this, not more: a subprocess shim is plain Rust, with no ABI
boundary to cross, so `eval` can call it directly. The one new cost there is inherent to
IPC itself, not to wiring the interpreter to it — `calcc run --interpret` will need
`python3` on PATH too, once that built-in exists.

## Why `calc-runtime` needs its own `build.rs` now

Here's the part that isn't just "repeat A9's static-library trick for C." In A9, three
things happen to `calc-add`, and only one of them was special:

1. Cargo compiles `calc-runtime` normally, into an `.rlib`. `calc-runtime`'s own `eval`
   closure calls `calc_add(a, b)` as a plain Rust function call, and the interpreter
   reaches it through one generic `(builtin.eval)(&args)` call — **no linking step, no
   archive, nothing new**. This is just what "one crate depends on another" already
   means in Rust; A9 didn't have to do anything for it to work.
2. `calc-compiler/build.rs` *additionally* compiles `calc-runtime/src/lib.rs` a second
   time, with bare `rustc --crate-type=staticlib`, into an archive with a real
   `calc_add` symbol a system linker can read.
3. `link_stub.rs` hands that archive to the linker when building a `.calc` program.

Step 1 is free. Steps 2 and 3 are the one thing A9 actually built, needed *only*
because the generated `.calc` program is assembled by a completely separate, non-Cargo
link.

`calc_sub` breaks step 1. It's C, not Rust — even the interpreter, running in the exact
same process as everything else, cannot call a C function without an actual link step.
There is no free path here the way there was for `calc_add`. So `calc-runtime` needs
its own `build.rs` for the first time in this project, to do for its *ordinary* Cargo
consumers what A9 never had to do for anyone:

```rust
// calc-runtime/build.rs
fn main() {
    println!("cargo:rerun-if-changed=native/calc_ffi.c");
    cc::Build::new().file("native/calc_ffi.c").compile("calc_ffi");
}
```

`cc::Build::compile` does three things: invoke the platform's C compiler, archive the
result into `libcalc_ffi.a` (`calc_ffi.lib` on MSVC), and print
`cargo:rustc-link-lib=static=calc_ffi` plus a matching `cargo:rustc-link-search`.

**What `cc` actually is.** [`cc`](https://crates.io/crates/cc) is a small, widely-used
crate — not part of Rust itself — whose whole job is finding and driving a system C
toolchain from a build script: locating `gcc`/`clang` on Unix-like systems, or `cl.exe`
plus the right `vcvars` environment on MSVC, so individual crates don't each have to
reimplement that discovery. This project already uses it a second way, at run time
rather than build time: `link_stub.rs`'s `cc::Build::new()...get_compiler()` call (A6)
uses the exact same toolchain-discovery logic purely to find a linker to drive, without
compiling anything.

**How printing text becomes a linker flag.** Called from inside a build script,
`cc::Build::compile` reads the environment variables Cargo sets before running any
`build.rs` (`TARGET`, `HOST`, `OUT_DIR`, `CARGO_CFG_TARGET_ENV`, …), uses them to invoke
the right compiler on `native/calc_ffi.c`, archives the result into `OUT_DIR` — and then
prints two plain lines to its own stdout:

```text
cargo:rustc-link-lib=static=calc_ffi
cargo:rustc-link-search=native=/path/to/OUT_DIR
```

Cargo defines a protocol for exactly this: any build-script stdout line starting with
`cargo:` isn't ordinary output — Cargo intercepts and parses it as an instruction (this
project's own `build.rs` already used one instance of the same protocol,
`cargo:rustc-env=CALC_RUNTIME_LIB=...`, since A9). `rustc-link-lib` means "pass this
library to the linker" (`-lstatic=calc_ffi` on Unix-style linkers,
`/DEFAULTLIB:calc_ffi.lib` on MSVC); `rustc-link-search` means "add this directory to
the linker's search path" (`-L`, or `LIB` on MSVC) — together: "link `calc_ffi`, and
here's where to find it." Crucially, Cargo doesn't apply these only to the crate whose
script printed them — it collects `rustc-link-lib`/`rustc-link-search` from *every*
crate's build script across a binary's whole dependency graph, and feeds them all into
that one binary's final link command. That's the propagation: any binary that depends
on `calc-runtime`, at any depth — the interpreter's own test binary, `calc-ir`'s tests,
`calcc` itself — automatically gets `calc_ffi` linked in when it's built, because
`calc-runtime`'s `build.rs` runs once but its two printed lines end up on all of their
link commands. That propagation is exactly what makes
`extern "C" { fn calc_sub(...); }` resolve for all of them, with nothing written in any
of those other crates.

![Left: calc-runtime's calc_add is compiled once, by Cargo; the interpreter reaches it through the same generic (builtin.eval)(&args) call every built-in goes through, and that eval closure is a plain Rust call needing no linking step. Right: calc-runtime declares calc_sub in Rust but its only definition is in C; the interpreter's call site is identical to the left panel's, but eval's closure now depends on calc-runtime's new build.rs to link that C function in before it can resolve.](images/a10-buildrs-need.svg)

## Two archives, two build scripts, one source file

`calc-compiler/build.rs` *still* needs its own copy, for the same reason as A9's step 2:
Cargo's propagation above only affects binaries built *through Cargo's own linking*.
`link_stub.rs` never goes through that — it hands raw file paths straight to a system
linker to assemble the *generated* `.calc` program, entirely outside Cargo. So
`calc-compiler/build.rs` compiles `../calc-runtime/native/calc_ffi.c` a second,
independent time:

```rust
cc::Build::new()
    .file(&ffi_src)
    .out_dir(&out_dir)
    .static_crt(true)   // see below
    .compile("calc_compiler_calc_ffi");
```

named `calc_compiler_calc_ffi` rather than `calc_ffi` — a deliberate difference from
`calc-runtime`'s own archive, so `calcc`'s own link never has two identically-named
static libraries both offering a `calc_sub` definition. (A linker would very likely
handle that fine regardless — it only pulls in whichever one first resolves an
outstanding symbol — but there's no reason to depend on that when a distinct name
avoids the question outright.) Its path is exposed the same way `CALC_RUNTIME_LIB` is:

```rust
println!("cargo:rustc-env=CALC_FFI_LIB={}", ffi_archive.display());
```

and `link_stub.rs` passes it alongside `RUNTIME_LIB` on every platform:

```rust
cmd.arg(&object_path).arg(RUNTIME_LIB).arg(FFI_LIB).arg("-o").arg(&exe_path);
```

So `calc_ffi.c` is compiled twice — once by each build script — but it's the *build
output* that's duplicated, not the logic: one three-line source file, read by both.
This mirrors A9's own "calc-runtime.rs compiled by Cargo, and again by raw `rustc`"
shape; what's new here is that A10 needed a *third* build (`calc-runtime`'s own) that
A9's design never required at all.

![One calc_ffi.c source file flows into two independent build scripts. calc-runtime's own build.rs compiles it to a "calc_ffi" archive that Cargo propagates automatically into calcc, every crate's tests, and the interpreter. calc-compiler's build.rs separately compiles the same source into a distinctly-named "calc_compiler_calc_ffi" archive, whose path is handed directly to link_stub.rs's raw, non-Cargo link of the generated program.](images/a10-two-archives.svg)

## A real linker warning: the MSVC CRT mismatch

The first working version of this session's code passed every test, but Windows CI
would have printed a linker warning on it:

```text
libcmt.lib(initializers.obj) : warning LNK4098: defaultlib 'msvcrt.lib' conflicts
with use of other libs; use /NODEFAULTLIB:library
```

### Static vs. dynamic libraries, in general

Every executable on Windows needs a **C runtime** (the CRT): the library behind
`malloc`/`free`, `printf`, and the start-up code that runs before `main` and calls it
with the right arguments. Every Rust program needs one too, since `rustc` compiles
down to native code that still relies on it. A program can get the CRT's code into
itself two different ways, and this distinction isn't specific to the CRT — it's the
same choice for *any* library:

- **Static linking**: the library's object code is *copied* into the executable at
  link time, the same way `link_stub.rs` copies `calc_add`'s machine code out of
  `calc_runtime.lib` and into `prog.exe` (A9's page walks through exactly this). The
  result is one self-contained file — no other file needs to exist for the program to
  run — at the cost of a bigger executable, and a separate copy of the library's code
  in every program that links it. On Windows, the static CRT's import library is
  named `libcmt.lib`.
- **Dynamic linking**: the executable stores only the *name* of a library it needs
  (e.g. `ucrtbase.dll`), plus which symbols it uses from it. The actual code lives in
  a separate `.dll` file, loaded into memory once and shared by every running program
  that needs it — smaller executables, one shared copy in memory, but the `.dll` must
  actually be present on the machine that runs the program. The library a program
  links against to get *this* behavior for the CRT is `msvcrt.lib` — despite the name,
  it is not the DLL itself, but a small stub telling the linker "resolve these symbols
  from whatever CRT DLL is present at run time," the dynamic-linking equivalent of
  `libcmt.lib`.

A single executable has to pick **one** of these for the CRT, program-wide — every
object file and every static library that goes into it has to agree, because the two
forms use incompatible internal layouts for some CRT-managed state (e.g. each has its
own separate heap; mixing them can produce a `malloc` in one and a `free` in the other
without them agreeing that memory is even valid). `LNK4098` is exactly this
disagreement made visible: some input to the link said "I need the dynamic CRT," while
`libcmt.lib` — the *static* CRT — was also on the command line.

### Where the conflict came from, and the fix that actually worked

`link_stub.rs` has always picked static, explicitly (`libcmt.lib`, `libvcruntime.lib`,
`libucrt.lib` — see A9's page); `cc`, left to its defaults, picks dynamic. So
`calc-compiler/build.rs`'s first version of the FFI archive — built with no CRT option
set — was announcing "link me with the dynamic CRT" right into a link that had already
committed to the static one: exactly the `LNK4098` above.

The obvious fix — the `cc` crate has a `.static_crt(true)` option for exactly this —
made that one warning disappear, and immediately created the mirror-image problem
somewhere else. Setting `.static_crt(true)` on **`calc-runtime`'s own** build fixed the
archive `link_stub.rs` uses, but that same setting also applies to the *other* archive
`calc-runtime/build.rs` produces — the one Cargo auto-links into ordinary binaries
(`calcc`, every crate's test harness). Those binaries are built by plain `cargo
build`/`cargo test`, which use `rustc`'s own default on `*-msvc` targets: the
**dynamic** CRT. Forcing static there produced the exact same warning, just aimed at a
different binary — and this one would have actually broken the build, not merely
looked untidy: a sufficiently recent `rustc` reports a linker's warning output as its
own lint, `linker_messages`, and `cargo clippy --all-targets -- -D warnings`
(CLAUDE.md's pre-commit checklist) turns every warning into a hard error.

The actual fix: the two archives never had to agree with *each other* — only each with
its own, different consumer. `calc-runtime/build.rs`, feeding ordinary Rust binaries,
stays on `cc`'s default (dynamic). `calc-compiler/build.rs`, feeding
`link_stub.rs`'s explicitly-static link, sets `.static_crt(true)`. Same three-line
source file, compiled to two archives, each carrying the CRT setting its *one*
consumer actually needs:

![Two panels. Before the fix, both the calc_ffi archive and the calc_compiler_calc_ffi archive default to the dynamic CRT: this matches calcc.exe (an ordinary, dynamic-CRT Rust binary) but conflicts with prog.exe, which link_stub.rs links against the static CRT — producing LNK4098. After the fix, calc-runtime's build.rs stays on the dynamic-CRT default (matching calcc.exe), while calc-compiler's build.rs sets .static_crt(true) (matching prog.exe's static-CRT link) — no warning either way.](images/a10-crt-mismatch.svg)

## What it costs, and what's deliberately not here

- **Zero new external dependencies.** A system C toolchain is needed only to *build*
  `calc-runtime`/`calc-compiler` — not to run a compiled `.calc` program, since
  `calc_sub` is statically linked — and it isn't even a new requirement:
  `link_stub.rs` has needed a system C toolchain since A6, to find a linker at all.
- **No dependency enumeration.** There's nothing today that records "`sub` needs a C
  toolchain" or (once A11 lands) "`print` needs `python3` on PATH" anywhere machine-
  readable. spec.md §7.2's override chain (`bindings.toml` → `calcc.toml` → CLI flag) is
  where that eventually belongs, once `bindings.toml` (Part B) is real data instead of
  this hardcoded table. Adding a field for it now, with nothing to read it, would be
  scaffolding for a session that doesn't exist yet.
- **`calc-runtime`'s stated scope widens, temporarily.** It used to describe itself as
  "native Rust built-ins." It's now "the built-in manifest, plus every kind's real
  implementation that's expressible here" — a native Rust function directly, an FFI
  library via its own tiny build step. That's accepted only because Part B's
  `bindings.toml` (B6) deletes this whole hardcoded table outright; the crate's real,
  permanent job is narrower than what it does today.
- **Still not here**: call syntax, a data-driven manifest, generation-time signature
  validation, dynamic (vs. static) FFI linking, the real link driver (A12) — all
  unchanged from A9's own list.

## Try it

```bash
# bash / zsh / Git Bash
cd calc-lang
cargo build
printf '(1 + 2) * 4 - 6 / 2' > prog.calc
./target/debug/calcc run --interpret prog.calc                          # 9
./target/debug/calcc build --backend=cranelift prog.calc -o prog && ./prog; echo $?   # 9
```

```powershell
# Windows PowerShell
cd calc-lang
cargo build
Set-Content -NoNewline -Encoding ascii -Path prog.calc -Value '(1 + 2) * 4 - 6 / 2'
.\target\debug\calcc.exe run --interpret prog.calc                          # 9
.\target\debug\calcc.exe build --backend=cranelift prog.calc -o prog
if ($?) { .\prog.exe }
$LASTEXITCODE   # 9
```

The PowerShell version differs from the bash one in two places, not just the
`&&`/`echo $?` swap. `if ($?) { ... }` plus `$LASTEXITCODE` is PowerShell's equivalent
of bash's "run the next command only on success, then print whichever command's exit
code." And `-Encoding ascii` isn't cosmetic: Windows PowerShell 5.1's default UTF-8
`Set-Content` writes a byte-order mark, `fs::read_to_string` (`main.rs`) doesn't strip
one, and the lexer would then choke on a stray character before the first token —
calc-lang source is always plain ASCII, so ASCII encoding sidesteps the question
entirely rather than fighting UTF-8's BOM.

`(1 + 2) * 4 - 6 / 2` exercises every operator: `add`/`mul`/`sub` as built-in calls
(two different archives, `add`/`mul` from `calc-runtime`'s Rust and `sub` from its C
file) and `/` as the one remaining inline `BinOp`. Try temporarily removing
`.arg(FFI_LIB)` from `link_stub.rs` and rebuilding: the linker will report `calc_sub`
as an unresolved external, the same way removing `RUNTIME_LIB` breaks `calc_add` in A9.
