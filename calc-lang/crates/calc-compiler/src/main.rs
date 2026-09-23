//! calcc — the calc-lang compiler CLI (spec.md §11). `run --interpret` (session A5)
//! and `build [--backend=<name>]` (sessions A6–A8) are the only subcommands so far:
//! both share the parse → resolve → lower prefix, then either interpret the IR
//! directly or hand it to whichever `backend::Backend` `--backend` selected, then
//! `runtime_deps` and `link` (session A12). A real CLI-argument crate (`clap`) and a
//! `check` subcommand land in A13 — hand-rolled `env::args()` parsing is enough for
//! two subcommands.

mod backend;
#[cfg(feature = "backend-cranelift")]
mod cranelift_backend;
mod link;
#[cfg(feature = "backend-llvm")]
mod llvm_backend;
mod runtime_deps;

use std::env;
use std::fs;
use std::path::Path;
use std::process::ExitCode;

use calc_ir::{interpret, lower, Program, Value};
use calc_syntax::lalrpop_frontend::LalrpopFrontend;
use calc_syntax::{resolve, ParserFrontend};

fn main() -> ExitCode {
    let args: Vec<String> = env::args().skip(1).collect();
    match args.as_slice() {
        [cmd, flag, path] if cmd == "run" && flag == "--interpret" => run_interpret(path),
        [cmd, rest @ ..] if cmd == "build" => match parse_build_args(rest) {
            Some((backend_name, path, out)) => build(backend_name, path, out),
            None => usage(),
        },
        _ => usage(),
    }
}

fn usage() -> ExitCode {
    let names: Vec<&str> = backend::enabled().iter().map(|b| b.name()).collect();
    eprintln!(
        "usage: calcc run --interpret <path>\n       calcc build [--backend=<{}>] <path> -o <output>",
        names.join("|")
    );
    ExitCode::FAILURE
}

/// Splits `build`'s arguments into (optional backend name, source path, output path).
fn parse_build_args(args: &[String]) -> Option<(Option<&str>, &str, &str)> {
    match args {
        [path, o, out] if o == "-o" => Some((None, path, out)),
        [backend, path, o, out] if o == "-o" => {
            Some((Some(backend.strip_prefix("--backend=")?), path, out))
        }
        _ => None,
    }
}

/// Shared front end for both subcommands: read the source file, then parse →
/// resolve → lower it into `calc-ir`'s `Program`, printing `calcc`-style
/// diagnostics and returning `None` on the first failure.
fn compile_to_ir(path: &str) -> Option<Program> {
    let src = match fs::read_to_string(path) {
        Ok(src) => src,
        Err(err) => {
            eprintln!("calcc: couldn't read {path}: {err}");
            return None;
        }
    };

    let ast = match LalrpopFrontend.parse(&src) {
        Ok(ast) => ast,
        Err(diagnostics) => {
            for d in diagnostics {
                eprintln!("calcc: parse error at byte {}: {}", d.offset, d.message);
            }
            return None;
        }
    };

    if let Err(errors) = resolve(&ast) {
        for e in errors {
            eprintln!("calcc: resolve error: {e:?}");
        }
        return None;
    }

    Some(lower(&ast))
}

fn run_interpret(path: &str) -> ExitCode {
    let Some(program) = compile_to_ir(path) else {
        return ExitCode::FAILURE;
    };
    let Value::Number(result) = interpret(&program);
    println!("{result}");
    ExitCode::SUCCESS
}

/// `backend_name` is `--backend`'s value, if given; `backend::select` resolves it (or
/// the implicit default) among the backends compiled into this build.
fn build(backend_name: Option<&str>, path: &str, out: &str) -> ExitCode {
    let backend = match backend::select(backend_name) {
        Ok(backend) => backend,
        Err(err) => {
            eprintln!("calcc: {err}");
            return ExitCode::FAILURE;
        }
    };
    let Some(program) = compile_to_ir(path) else {
        return ExitCode::FAILURE;
    };

    let object_bytes = match backend.compile(&program) {
        Ok(bytes) => bytes,
        Err(err) => {
            eprintln!("calcc: {} backend failed: {err}", backend.name());
            return ExitCode::FAILURE;
        }
    };

    let prepared = match runtime_deps::prepare(&program) {
        Ok(prepared) => prepared,
        Err(err) => {
            eprintln!("calcc: {err}");
            return ExitCode::FAILURE;
        }
    };

    match link::link(&object_bytes, &prepared.bundles, Path::new(out)) {
        Ok(exe_path) => {
            println!("wrote {}", exe_path.display());
            for note in &prepared.notes {
                println!("note: needs at run time: {note}");
            }
            ExitCode::SUCCESS
        }
        Err(err) => {
            eprintln!("calcc: link failed: {err}");
            ExitCode::FAILURE
        }
    }
}
