//! calcc — the calc-lang compiler CLI (spec.md §11). `run --interpret` is the only
//! subcommand so far (session A5): parse → resolve → lower → interpret → print. A
//! real CLI-argument crate (`clap`) and `build`/`check` subcommands land in A12/A13 —
//! hand-rolled `env::args()` parsing is enough for this one subcommand.

use std::env;
use std::fs;
use std::process::ExitCode;

use calc_ir::{interpret, lower, Value};
use calc_syntax::lalrpop_frontend::LalrpopFrontend;
use calc_syntax::{resolve, ParserFrontend};

fn main() -> ExitCode {
    let args: Vec<String> = env::args().skip(1).collect();
    match args.as_slice() {
        [cmd, flag, path] if cmd == "run" && flag == "--interpret" => run_interpret(path),
        _ => {
            eprintln!("usage: calcc run --interpret <path>");
            ExitCode::FAILURE
        }
    }
}

fn run_interpret(path: &str) -> ExitCode {
    let src = match fs::read_to_string(path) {
        Ok(src) => src,
        Err(err) => {
            eprintln!("calcc: couldn't read {path}: {err}");
            return ExitCode::FAILURE;
        }
    };

    let ast = match LalrpopFrontend.parse(&src) {
        Ok(ast) => ast,
        Err(diagnostics) => {
            for d in diagnostics {
                eprintln!("calcc: parse error at byte {}: {}", d.offset, d.message);
            }
            return ExitCode::FAILURE;
        }
    };

    if let Err(errors) = resolve(&ast) {
        for e in errors {
            eprintln!("calcc: resolve error: {e:?}");
        }
        return ExitCode::FAILURE;
    }

    let program = lower(&ast);
    let Value::Number(result) = interpret(&program);
    println!("{result}");
    ExitCode::SUCCESS
}
