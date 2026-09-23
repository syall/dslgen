//! calcc — the calc-lang compiler CLI (spec.md §11; session A13). Three subcommands,
//! each running a longer prefix of one pipeline:
//!
//! * `check <path>` — parse → resolve, then stop (diagnostics only);
//! * `run --interpret <path>` — … → lower → interpret the IR (session A5);
//! * `build [--backend=<name>] <path> -o <output>` — … → lower → codegen via the
//!   selected `backend::Backend` (A6–A8) → `runtime_deps` → `link` (A12).
//!
//! Arguments are parsed by `clap`'s derive API: the `Cli`/`Command` types below *are*
//! the CLI's definition, and their doc comments are its `--help` text. Usage errors
//! exit with code 2 (clap's convention), compile errors with code 1. See
//! `calc-lang/docs/a13-calcc-cli-surface.md`.

mod backend;
#[cfg(feature = "backend-cranelift")]
mod cranelift_backend;
mod link;
#[cfg(feature = "backend-llvm")]
mod llvm_backend;
mod runtime_deps;

use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use calc_ir::{interpret, lower, Value};
use calc_syntax::lalrpop_frontend::LalrpopFrontend;
use calc_syntax::{resolve, Expr, ParserFrontend};
use clap::{Parser, Subcommand};

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
        /// Print each step's choices and external commands (backend, run-time
        /// dependency probes, bundles, the linker command line) to stderr.
        #[arg(long)]
        verbose: bool,
        /// Keep the intermediate object files next to the output.
        #[arg(long)]
        keep_object: bool,
    },
    /// Run a program without compiling it to an executable.
    Run {
        /// Run it with the debug interpreter (spec.md §9.1). Required: the
        /// interpreter is a development aid, not the default way to run a program.
        #[arg(long, required = true)]
        interpret: bool,
        /// The `.calc` source file.
        path: PathBuf,
    },
    /// Parse and check a program without running or compiling it.
    Check {
        /// The `.calc` source file.
        path: PathBuf,
    },
}

fn main() -> ExitCode {
    match Cli::parse().command {
        Command::Build {
            backend,
            path,
            output,
            verbose,
            keep_object,
        } => build(
            backend.as_deref(),
            &path,
            &output,
            link::LinkOptions {
                verbose,
                keep_object,
            },
        ),
        // `interpret` is always true here (clap requires it); it's a field so A16's
        // `--hot-reload` can become the other choice of one required mode.
        Command::Run { interpret: _, path } => run_interpret(&path),
        Command::Check { path } => match parse_and_resolve(&path) {
            Some(_) => ExitCode::SUCCESS,
            None => ExitCode::FAILURE,
        },
    }
}

/// The front end every subcommand shares (all of `check`): read the source file,
/// then parse → resolve it, printing `calcc`-style diagnostics and returning `None`
/// on the first failing stage.
fn parse_and_resolve(path: &Path) -> Option<Expr> {
    let src = match fs::read_to_string(path) {
        Ok(src) => src,
        Err(err) => {
            eprintln!("calcc: couldn't read {}: {err}", path.display());
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

    Some(ast)
}

fn run_interpret(path: &Path) -> ExitCode {
    let Some(ast) = parse_and_resolve(path) else {
        return ExitCode::FAILURE;
    };
    let program = lower(&ast);
    let Value::Number(result) = interpret(&program);
    println!("{result}");
    ExitCode::SUCCESS
}

/// `backend_name` is `--backend`'s value, if given; `backend::select` resolves it (or
/// the implicit default) among the backends compiled into this build.
fn build(
    backend_name: Option<&str>,
    path: &Path,
    out: &Path,
    options: link::LinkOptions,
) -> ExitCode {
    let backend = match backend::select(backend_name) {
        Ok(backend) => backend,
        Err(err) => {
            eprintln!("calcc: {err}");
            return ExitCode::FAILURE;
        }
    };
    if options.verbose {
        eprintln!("backend: {}", backend.name());
    }
    let Some(ast) = parse_and_resolve(path) else {
        return ExitCode::FAILURE;
    };
    let program = lower(&ast);

    let object_bytes = match backend.compile(&program) {
        Ok(bytes) => bytes,
        Err(err) => {
            eprintln!("calcc: {} backend failed: {err}", backend.name());
            return ExitCode::FAILURE;
        }
    };

    let prepared = match runtime_deps::prepare(&program, options.verbose) {
        Ok(prepared) => prepared,
        Err(err) => {
            eprintln!("calcc: {err}");
            return ExitCode::FAILURE;
        }
    };

    match link::link(&object_bytes, &prepared.bundles, out, &options) {
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
