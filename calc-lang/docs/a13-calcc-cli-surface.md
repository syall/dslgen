# A13 — The `calcc` CLI surface

**Session code**:
[`crates/calc-compiler/src/main.rs`](../crates/calc-compiler/src/main.rs),
[`crates/calc-compiler/src/link.rs`](../crates/calc-compiler/src/link.rs) (`LinkOptions`),
[`crates/calc-compiler/src/runtime_deps.rs`](../crates/calc-compiler/src/runtime_deps.rs),
[`crates/calc-compiler/tests/cli.rs`](../crates/calc-compiler/tests/cli.rs).
**Spec refs**: spec.md §11 (generated-compiler CLI), §9.1 (the interpreter is not the
default). **Prereqs**: [A5](a5-tree-walking-interpreter.md),
[A12](a12-the-link-driver.md).

By A12, every stage of a real compiler exists: parsing, name resolution, an IR, an
interpreter, two codegen backends, a link driver. What's left is the part a user
actually touches: the command line. Until now, `calcc` read its arguments with two
hand-written patterns, which meant `-o` had to come last, `--backend` had to come
first, `--help` didn't exist, and neither did `calcc check`. A13 replaces that with a
real CLI crate (`clap`) and adds the third subcommand spec.md §11 asks for:

```text
calcc check <path>                                   parse and check only
calcc run --interpret <path>                         run with the debug interpreter
calcc build [--backend=<name>] <path> -o <output>    compile to a native executable
```

## What a compiler driver is

`calcc` isn't really "the compiler". It's a **driver**: a small program that decides
which stages of the compiler to run, in what order, and what to do with each stage's
output. `gcc`, `clang`, `rustc` and `cargo` are all drivers in this sense. `gcc` on
its own runs the preprocessor, the compiler proper, the assembler and the linker as
separate programs, and flags like `-fsyntax-only`, `-S` or `-c` just tell the driver
where to stop.

So a compiler CLI is a set of choices about **how far down the pipeline to go**.
calc-lang's three subcommands are exactly that: each one runs a longer prefix of the
same pipeline.

```text
                  ┌──────── check ────────┐
                  │                       │
source ──► read ──► parse ──► resolve ──┬──► lower ──► interpret ──► print result
                                        │                              (run --interpret)
                                        │
                                        └──► lower ──► codegen ──► runtime deps ──► link ──► executable
                                                       (backend)                            (build)
```

- `check` stops after `resolve`. If the program has a syntax error or uses a name
  that isn't bound, you find out without running or compiling anything.
- `run --interpret` goes on to lower the program to IR and interpret it (A5).
- `build` lowers it, then hands the IR to a codegen backend (A6–A8), checks and
  packs the run-time dependencies (A11/A12), and links an executable (A12).

In the code, this shows up as one shared function every subcommand starts with:

```rust
/// The front end every subcommand shares (all of `check`): read the source file,
/// then parse → resolve it, printing `calcc`-style diagnostics and returning `None`
/// on the first failing stage.
fn parse_and_resolve(path: &Path) -> Option<Expr> { … }
```

Before A13 this function also lowered to IR (`compile_to_ir`), since both
subcommands needed IR. `check` doesn't, so lowering moved out to the two callers
that do. That's the whole change the new subcommand needed from the pipeline.

**Why `check` matters.** It's the fastest useful answer a compiler can give. Editors
and CI jobs run it constantly because it skips all the expensive stages. `cargo check`
is the same idea for Rust. In calc-lang, spec.md §11 says "parse/typecheck", but
calc-lang has only one type of value (`f64`), so there's nothing to typecheck yet.
Name resolution is the only semantic check that exists, so that's what `check` runs.

## Parsing arguments with `clap`

### What a CLI crate does for you

Every program receives its arguments as a list of strings: `["build", "-o", "prog",
"prog.calc"]`. Turning that into "the user wants `build`, with output `prog` and input
`prog.calc`" means handling flags in any order, `--flag=value` and `--flag value`
both, missing and unknown arguments, and help text. Doing all of that by hand is a
lot of fiddly code. `clap` is the Rust ecosystem's standard crate for it.

### The derive API

Rust's **derive macros** generate code from a type definition. You've seen
`#[derive(Debug)]` generate a `Debug` implementation from a struct's fields. `clap`
does the same thing for command lines: you describe the arguments as a struct or an
enum, and `#[derive(Parser)]` generates the parsing code, the error messages and the
`--help` output. Here's `calcc`'s entire CLI definition, slightly trimmed:

```rust
/// The calc-lang compiler.
#[derive(Parser)]
#[command(name = "calcc", bin_name = "calcc", version)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Compile a program to a native executable.
    Build {
        /// Codegen backend: `cranelift` or `llvm`, among those compiled into this
        /// calcc. Optional when only one is.
        #[arg(long, value_name = "NAME")]
        backend: Option<String>,
        /// The `.calc` source file.
        path: PathBuf,
        /// Where to write the executable (`.exe` is added on Windows if missing).
        #[arg(short = 'o', value_name = "OUTPUT")]
        output: PathBuf,
        #[arg(long)]
        verbose: bool,
        #[arg(long)]
        keep_object: bool,
    },
    /// Run a program without compiling it to an executable.
    Run {
        #[arg(long, required = true)]
        interpret: bool,
        path: PathBuf,
    },
    /// Parse and check a program without running or compiling it.
    Check { path: PathBuf },
}
```

How to read it:

- **Each enum variant is a subcommand.** `#[derive(Subcommand)]` turns `Build` into
  `calcc build`, and so on (the name is lowercased).
- **A field with no `#[arg]` is a positional argument.** `path` is whatever
  non-flag argument appears, wherever it appears.
- **`#[arg(long)]` makes a `--name` flag** from the field name (`keep_object` becomes
  `--keep-object`). `#[arg(short = 'o')]` makes `-o`.
- **The field's type decides the rest.** `bool` is a switch that's present or not.
  `Option<String>` is an optional flag with a value. `PathBuf` is required, and gets
  parsed as a path.
- **Doc comments (`///`) become the help text.** The comment that documents the code
  is the same text a user sees in `calcc build --help`, so the two can't drift apart.

`main` then asks clap to parse, and `match`es on which variant came back:

```rust
fn main() -> ExitCode {
    match Cli::parse().command {
        Command::Build { backend, path, output, verbose, keep_object } => build(…),
        Command::Run { interpret: _, path } => run_interpret(&path),
        Command::Check { path } => match parse_and_resolve(&path) {
            Some(_) => ExitCode::SUCCESS,
            None => ExitCode::FAILURE,
        },
    }
}
```

Because `Command` is an enum, the compiler checks that `main` handles every
subcommand. Add a fourth variant and forget to handle it, and the build fails.

This is what `calcc build --help` prints, all generated from the definition above:

```text
Compile a program to a native executable

Usage: calcc build [OPTIONS] -o <OUTPUT> <PATH>

Arguments:
  <PATH>  The `.calc` source file

Options:
      --backend <NAME>  Codegen backend: `cranelift` or `llvm`, among those compiled into this calcc. Optional when only one is
  -o <OUTPUT>           Where to write the executable (`.exe` is added on Windows if missing)
      --verbose         Print each step's choices and external commands (backend, run-time dependency probes, bundles, the linker command line) to stderr
      --keep-object     Keep the intermediate object files next to the output
  -h, --help            Print help
```

(`bin_name = "calcc"` is there so this says `calcc` rather than `calcc.exe` on
Windows. By default clap uses whatever name the program was started with.)

### Where validation lives: `--backend` is still `select`'s job

clap can check values too. `#[arg(value_parser = ["cranelift", "llvm"])]` would reject
`--backend=nope` before `main` even runs. `calcc` deliberately doesn't do this.
A8's `backend::select` already gives better errors than a generic "invalid value"
could:

```text
$ calcc build --backend=nope prog.calc -o prog
calcc: unknown backend `nope` (enabled: cranelift)

$ calcc build --backend=llvm prog.calc -o prog     # in a Cranelift-only build
calcc: this build has no llvm backend; rebuild with `--features backend-llvm` (enabled: cranelift)
```

The second message tells "this backend doesn't exist" apart from "it exists but
this `calcc` was built without it", and says how to fix it. That knowledge lives in
`backend.rs`, next to the list of backends. A clap value list would be a second copy
of that list, and a worse error. The rule of thumb: let the CLI crate check the
*shape* of the command line (which flags, how many arguments), and let the code that
owns a concept check its *values*.

## Exit codes and output streams

A CLI's output is also read by other programs: shells, scripts, CI, editors. Two
conventions make that work.

**Exit codes say what kind of failure it was.**

| Code | Meaning | Example |
|---|---|---|
| 0 | success | `calcc check ok.calc` |
| 1 | the *program being compiled* has a problem | an unresolved name, a link failure |
| 2 | the *command line* has a problem | `calcc run prog.calc` (no `--interpret`) |

clap exits with 2 on its own when it can't parse the arguments. `calcc` returns
`ExitCode::FAILURE` (1) for everything past that point. A script can tell "I called
calcc wrong" apart from "the program is broken" without reading any output.

**stdout carries results, stderr carries everything else.** `run --interpret` prints
the result on stdout. `build` prints `wrote <path>`, its `note: needs at run time`
lines and (with `--keep-object`) `kept <path>` lines on stdout. Diagnostics and
`--verbose` narration go to stderr. So `calcc run --interpret prog.calc > answer.txt`
captures only the answer, and `check` prints nothing at all when the program is fine.
That's the Unix convention: silence means success.

## `run`: why `--interpret` is required

It would be easy to make `calcc run prog.calc` interpret by default. spec.md §9.1
says the opposite: the interpreter is a development aid, "explicitly not the default
execution path". The real product is `calcc build`. So `--interpret` is required:

```text
$ calcc run prog.calc
error: the following required arguments were not provided:
  --interpret

Usage: calcc run --interpret <PATH>
```

A15 adds a second way to run a program: `calcc run --hot-reload`, which JIT-compiles
it with Cranelift and patches in changes while it runs (§9.2). At that point the two
flags become a clap **argument group**: exactly one of `--interpret` and
`--hot-reload` is required, and giving both is a usage error. A group with one member
would just be noise, so it waits for A15. (That's also why `Run` keeps an
`interpret: bool` field that's always `true` today: it's the slot the choice goes in.)
`run` takes no `--backend`: the interpreter has no backend, and hot reload is
Cranelift-only by design.

## `--verbose` and `--keep-object`: seeing inside `build`

`build` is the only subcommand that runs other programs (probe commands, the linker)
and writes intermediate files (object files). Two flags make that visible. A12's
decision log listed them as work for this session.

**`--verbose`** prints, on stderr, every point where `calcc` makes a choice or runs
another program:

```text
$ calcc build --verbose prog.calc -o prog
backend: cranelift
probe: python --version (built-in `print`)
bundle: print (2 files, 1023 bytes)
link: "C:\…\cl.exe" prog.obj prog.bundles.obj …\calc_runtime.lib …\calc_ffi.lib kernel32.lib ntdll.lib userenv.lib ws2_32.lib dbghelp.lib /nologo /Fe:prog.exe /link /defaultlib:libcmt
wrote prog.exe
```

The `link:` line is the most useful one. It's the exact command the link driver runs,
and you can copy it and run it yourself. Everything A12's teaching page describes
(link units, link order, system libraries) shows up in it. Before A13, you only saw
this line when linking *failed*.

The flag goes to three places, because three pieces of code make those decisions:
`main.rs` (which backend was selected), `runtime_deps::prepare` (probes and bundles),
and `link::link`, through a small options struct:

```rust
#[derive(Debug, Default, Clone, Copy)]
pub struct LinkOptions {
    pub verbose: bool,
    pub keep_object: bool,
}
```

`Default` makes every field `false`, so the tests that call `link` directly pass
`&LinkOptions::default()` and behave exactly as before.

**`--keep-object`** keeps the two object files the link driver writes next to the
output: the program's own code, and the object holding the embedded IPC bundles.
Normally they're deleted after linking. Keeping them lets you inspect what a backend
produced (`objdump -d prog.o`, or `dumpbin /disasm prog.obj` on Windows), or rerun
the `link:` command from `--verbose` by hand:

```text
$ calcc build --keep-object prog.calc -o prog
kept prog.obj
kept prog.bundles.obj
wrote prog.exe
```

Two places deliberately don't get these flags. `check` runs nothing and writes
nothing. `run --interpret`'s only subprocesses are IPC built-ins, started from inside
`calc-runtime`. Compiled executables share that code, and it has no way to receive a
flag without an environment variable or a global.

**Other link options, and why they aren't flags.** A link step could take many more
options. Each one below either already works another way or belongs to a later
session:
- **which linker to use**: set the `CC` environment variable (the `cc` crate reads it);
- **target platform**: C8 (cross-compilation), because it changes codegen too;
- **static vs. dynamic FFI libraries**: C7, declared per built-in in the manifest;
- **your own library locations**: C2 (external overrides);
- **extra raw linker arguments**: nothing needs them yet.

## Testing a CLI

`tests/cli.rs` tests `calcc` the way a user uses it: by running the binary. Cargo
builds a package's binaries before its integration tests, and tells each test where
they are through an environment variable set at compile time:

```rust
fn calcc(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_calcc"))
        .args(args)
        .output()
        .expect("calcc should start")
}
```

The tests then check the three parts of a CLI's contract: exit code, stdout and
stderr. For example, `run` without a mode must exit with 2 and mention
`--interpret`. `check` on a valid program must exit with 0 and print nothing. `build`
with `-o` *before* the source path must still work, which A5–A12's hand-written
patterns couldn't do.

The `build` tests rely on `calcc` picking a backend on its own, which it only does
when exactly one is compiled in. So they're gated on that:

```rust
#[cfg(not(all(feature = "backend-cranelift", feature = "backend-llvm")))]
mod build { … }
```

That covers both CI jobs that build one backend (Cranelift-only and LLVM-only).

## Try it

```bash
# bash / zsh / Git Bash
cd calc-lang
cargo build
printf '{ let x = 2; x * 21 }' > prog.calc
./target/debug/calcc --help
./target/debug/calcc check prog.calc; echo $?          # (nothing) / 0
printf 'y + 1' > bad.calc
./target/debug/calcc check bad.calc; echo $?           # resolve error / 1
./target/debug/calcc run prog.calc; echo $?            # usage error / 2
./target/debug/calcc run --interpret prog.calc         # 42
./target/debug/calcc build --verbose --keep-object -o prog prog.calc
./prog; echo $?                                        # 42
```

```powershell
# Windows PowerShell
cd calc-lang
cargo build
Set-Content -Encoding ascii -Path prog.calc -Value "{ let x = 2; x * 21 }"
.\target\debug\calcc.exe --help
.\target\debug\calcc.exe check prog.calc; $LASTEXITCODE     # 0
.\target\debug\calcc.exe run --interpret prog.calc          # 42
.\target\debug\calcc.exe build --verbose --keep-object -o prog prog.calc
.\prog.exe; $LASTEXITCODE                                   # 42
```

Try running the `link:` line from `--verbose` yourself after a `--keep-object`
build (on Windows, from a Developer PowerShell, so `cl.exe` finds the system
libraries that `calcc` otherwise locates for it). It produces the same executable, which shows there's nothing in `calcc build`
beyond the stages this series has walked through.
