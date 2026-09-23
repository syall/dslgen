//! `calcc`'s command-line surface (spec.md §11, session A13), tested by running the
//! real binary the way a user would. Cargo builds a package's binaries before its
//! integration tests and tells each test where they are via `CARGO_BIN_EXE_<name>`.
//!
//! Exit codes are part of the contract: 0 for success, 1 for a problem with the
//! program being compiled, 2 for a problem with the command line itself (clap's
//! convention).

use std::path::PathBuf;
use std::process::{Command, Output};

fn calcc(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_calcc"))
        .args(args)
        .output()
        .expect("calcc should start")
}

/// Writes `src` to a fresh `.calc` file in a per-test temp directory and returns its
/// path as a string, ready to pass to `calcc`.
fn source(test: &str, src: &str) -> String {
    let dir = temp_dir(test);
    let path = dir.join("prog.calc");
    std::fs::write(&path, src).unwrap();
    path.to_string_lossy().into_owned()
}

fn temp_dir(test: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("calc_a13_{test}"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

#[test]
fn help_lists_every_subcommand() {
    let output = calcc(&["--help"]);
    assert!(output.status.success());
    let help = String::from_utf8_lossy(&output.stdout);
    for sub in ["build", "run", "check"] {
        assert!(help.contains(sub), "{help}");
    }
}

#[test]
fn check_is_silent_on_a_valid_program() {
    let path = source("check_ok", "{ let x = 2; x * 21 }");
    let output = calcc(&["check", &path]);
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    assert!(output.stdout.is_empty());
}

#[test]
fn check_reports_resolve_and_parse_errors() {
    let path = source("check_unresolved", "y + 1");
    let output = calcc(&["check", &path]);
    assert_eq!(output.status.code(), Some(1));
    assert!(
        stderr(&output).contains("resolve error"),
        "{}",
        stderr(&output)
    );

    let path = source("check_parse", "let (");
    let output = calcc(&["check", &path]);
    assert_eq!(output.status.code(), Some(1));
    assert!(
        stderr(&output).contains("parse error"),
        "{}",
        stderr(&output)
    );
}

#[test]
fn run_interpret_prints_the_result() {
    let path = source("run", "{ let x = 2; x * 21 }");
    let output = calcc(&["run", "--interpret", &path]);
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    assert_eq!(String::from_utf8_lossy(&output.stdout).trim(), "42");
}

/// spec.md §9.1: the interpreter isn't the default way to run a program, so `run`
/// with no mode is a usage error rather than a silent interpret.
#[test]
fn run_without_a_mode_is_a_usage_error() {
    let path = source("run_no_mode", "1");
    let output = calcc(&["run", &path]);
    assert_eq!(output.status.code(), Some(2));
    assert!(
        stderr(&output).contains("--interpret"),
        "{}",
        stderr(&output)
    );
}

/// `select`'s own message, not clap's: it lists the backends actually compiled in.
#[test]
fn an_unknown_backend_is_reported_by_backend_selection() {
    let path = source("unknown_backend", "1");
    let out = temp_dir("unknown_backend").join("prog");
    let output = calcc(&[
        "build",
        "--backend=nope",
        &path,
        "-o",
        &out.to_string_lossy(),
    ]);
    assert_eq!(output.status.code(), Some(1));
    let err = stderr(&output);
    assert!(err.contains("unknown backend `nope`"), "{err}");
    assert!(err.contains("enabled:"), "{err}");
}

/// These builds rely on the implicit backend, which exists only when exactly one is
/// compiled in (the default Cranelift-only build, or LLVM-only).
#[cfg(not(all(feature = "backend-cranelift", feature = "backend-llvm")))]
mod build {
    use super::*;

    /// `-o` before the path, flags in any order: clap doesn't care about order the
    /// way A5–A12's hand-written slice patterns did. The executable's exit code is
    /// the program's answer (see either backend's `main` wrapper).
    #[test]
    fn builds_a_runnable_executable_with_flags_in_any_order() {
        let path = source("build", "{ let x = 2; x * 21 }");
        let out = temp_dir("build_out").join("prog");
        let output = calcc(&["build", "-o", &out.to_string_lossy(), &path]);
        assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
        let stdout = String::from_utf8_lossy(&output.stdout);
        let exe = stdout
            .lines()
            .find_map(|line| line.strip_prefix("wrote "))
            .expect("calcc reports the executable's path");
        let status = Command::new(exe).status().expect("executable should run");
        assert_eq!(status.code(), Some(42));
    }

    #[test]
    fn verbose_shows_the_backend_and_the_link_command() {
        let path = source("verbose", "1");
        let out = temp_dir("verbose_out").join("prog");
        let output = calcc(&["build", "--verbose", &path, "-o", &out.to_string_lossy()]);
        assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
        let err = stderr(&output);
        let expected = if cfg!(feature = "backend-cranelift") {
            "backend: cranelift"
        } else {
            "backend: llvm"
        };
        assert!(err.contains(expected), "{err}");
        assert!(err.contains("link: "), "{err}");
    }

    #[test]
    fn keep_object_leaves_the_intermediate_objects() {
        let path = source("keep", "1");
        let dir = temp_dir("keep_out");
        let out = dir.join("prog");
        let output = calcc(&[
            "build",
            "--keep-object",
            &path,
            "-o",
            &out.to_string_lossy(),
        ]);
        assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
        let kept: Vec<&str> = std::str::from_utf8(&output.stdout)
            .unwrap()
            .lines()
            .filter_map(|line| line.strip_prefix("kept "))
            .collect();
        assert_eq!(kept.len(), 2, "{kept:?}");
        for path in kept {
            assert!(std::path::Path::new(path).is_file(), "{path}");
        }
    }

    #[test]
    fn without_keep_object_nothing_but_the_executable_is_left() {
        let path = source("no_keep", "1");
        let dir = temp_dir("no_keep_out");
        let output = calcc(&["build", &path, "-o", &dir.join("prog").to_string_lossy()]);
        assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
        let left: Vec<_> = std::fs::read_dir(&dir).unwrap().collect();
        assert_eq!(left.len(), 1, "{left:?}");
    }
}
