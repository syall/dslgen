//! calcc — the calc-lang compiler CLI (spec.md §11). `run --interpret` (session A5)
//! and `build --backend=cranelift|llvm` (sessions A6/A7) are the only subcommands so
//! far: both share the parse → resolve → lower prefix, then either interpret the IR
//! directly or hand it to a backend (`cranelift_backend`, or `llvm_backend` when
//! built with `--features backend-llvm`) and `link_stub`. A real CLI-argument crate
//! (`clap`), trait-based `--backend` dispatch, and a `check` subcommand land in
//! A8/A13 — hand-rolled `env::args()` parsing is enough for two subcommands.

mod cranelift_backend;
mod link_stub;
#[cfg(feature = "backend-llvm")]
mod llvm_backend;

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
        [cmd, backend, path, out_flag, out]
            if cmd == "build" && backend == "--backend=cranelift" && out_flag == "-o" =>
        {
            build(path, out, cranelift_backend::compile_to_object)
        }
        #[cfg(feature = "backend-llvm")]
        [cmd, backend, path, out_flag, out]
            if cmd == "build" && backend == "--backend=llvm" && out_flag == "-o" =>
        {
            build(path, out, llvm_backend::compile_to_object)
        }
        #[cfg(not(feature = "backend-llvm"))]
        [cmd, backend, ..] if cmd == "build" && backend == "--backend=llvm" => {
            eprintln!(
                "calcc: this build has no LLVM backend; rebuild with `--features backend-llvm`"
            );
            ExitCode::FAILURE
        }
        _ => {
            eprintln!(
                "usage: calcc run --interpret <path>\n       calcc build --backend=<cranelift|llvm> <path> -o <output>"
            );
            ExitCode::FAILURE
        }
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

/// `compile_to_object` is whichever backend's entry point `--backend` selected; A8
/// replaces this function-pointer stand-in with a real `Backend` trait.
fn build(path: &str, out: &str, compile_to_object: fn(&Program) -> Vec<u8>) -> ExitCode {
    let Some(program) = compile_to_ir(path) else {
        return ExitCode::FAILURE;
    };

    let object_bytes = compile_to_object(&program);

    match link_stub::link(&object_bytes, Path::new(out)) {
        Ok(exe_path) => {
            println!("wrote {}", exe_path.display());
            ExitCode::SUCCESS
        }
        Err(err) => {
            eprintln!("calcc: link failed: {err}");
            ExitCode::FAILURE
        }
    }
}
