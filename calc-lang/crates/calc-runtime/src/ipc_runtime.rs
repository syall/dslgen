//! The subprocess/IPC built-in kind's real implementation (spec.md §7 kind 3,
//! session A11): `print`'s real work is delegated to `native/print.py`, a small
//! external Python script, spawned fresh per call (spec.md §14.6's simplest
//! lifecycle choice — a long-lived worker process is deferred to Part C).
//!
//! **Protocol (resolving spec.md §14.5)**: the numeric argument is passed as a
//! plain command-line argument, and the script's stdout is captured via
//! `std::process::Command`'s piped output — not a JSON request/response over
//! stdin, despite roadmap.md's A11 entry suggesting `serde_json`. This file must
//! stay dependency-free like the rest of `calc-runtime/src/lib.rs` (see that
//! file's module doc comment and `calc-lang/DECISIONS.md`'s A11 entry for why);
//! a single scalar argument doesn't need a serialization library to marshal.
//!
//! **Failure semantics (resolving spec.md §14.7)**: a failed spawn or a nonzero
//! exit is a `panic!` — a process-level abort, matching this codebase's existing
//! style for invariants with no error-handling story yet (no `calcc`-level
//! diagnostics infrastructure exists for a runtime built-in failure).

use std::process::{Command, Output};

/// `print.py`'s source, embedded at compile time via a path relative to *this*
/// source file — not an environment variable or a runtime-relative path, so it
/// resolves identically regardless of which build produced the calling binary
/// (an ordinary `cargo build`, or `calc-compiler/build.rs`'s separate bare-`rustc`
/// build of this same crate's `src/lib.rs`, session A9).
const PRINT_SCRIPT: &str = include_str!("../native/print.py");

/// The built-in's real implementation: relays `print.py`'s output to this
/// process's own stdout — calc-lang's first real program output, beyond a final
/// numeric result — and returns `x` unchanged, exactly like the other two kinds'
/// implementations return a value with no side effect of their own. `print` is
/// called as a statement (`calc-syntax::Stmt::Print`), so calc-lang itself never
/// reads this return value — see `calc-lang/DECISIONS.md`'s A11 entry for why it's
/// kept anyway rather than making the call genuinely `void`.
pub fn call(x: f64) -> f64 {
    print!("{}", run_print_script(x));
    x
}

/// Runs `print.py` with `x` as its one command-line argument (`sys.argv[1]`,
/// since `-c <script>` occupies `sys.argv[0]`) and returns whatever it wrote to
/// its own stdout. Split out from `call` so a test can assert on the captured
/// text directly, rather than needing to capture this process's own stdout.
fn run_print_script(x: f64) -> String {
    // Try `python3` first — the conventional name on Unix, and what roadmap.md's
    // own illustrative script uses — falling back to `python`. Found empirically:
    // some Windows installs (including this project's own dev machine) only
    // register a working `python` on PATH; `python3` resolves to a non-functional
    // "app execution alias" stub that spawns successfully but exits 9009 without
    // running anything, rather than failing to spawn at all, so the fallback has
    // to check exit status, not just spawn success.
    let output = try_python("python3", x)
        .or_else(|| try_python("python", x))
        .unwrap_or_else(|| {
            panic!(
                "calc-runtime: couldn't run python3 or python for the `print` \
                 built-in (is a working Python 3 on PATH?)"
            )
        });
    String::from_utf8(output.stdout).expect("print.py's output should be valid UTF-8")
}

/// Spawns `interpreter -c <PRINT_SCRIPT> <x>`, returning its output only if the
/// interpreter actually ran the script — `None` if spawning failed outright, or
/// it exited non-zero (covers both a real failure and the Windows alias-stub case
/// above).
fn try_python(interpreter: &str, x: f64) -> Option<Output> {
    let output = Command::new(interpreter)
        .arg("-c")
        .arg(PRINT_SCRIPT)
        .arg(x.to_string())
        .output()
        .ok()?;
    output.status.success().then_some(output)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Calls the real `print.py`, not a Rust reimplementation of its f-string
    /// formatting — the same "`eval` must call the one real implementation"
    /// principle `calc-lang/DECISIONS.md`'s A10 entry established for kind 2.
    /// `\r\n` vs `\n` is normalized away: Python's stdout is text-mode, so it
    /// emits the platform's own line ending (`\r\n` on Windows) — that's real,
    /// faithfully-relayed subprocess output, not something worth asserting on.
    #[test]
    fn runs_the_real_print_script_and_captures_its_output() {
        assert_eq!(
            run_print_script(12.3456).replace("\r\n", "\n"),
            "result = 12.35\n"
        );
    }

    #[test]
    fn call_returns_its_argument_unchanged() {
        assert_eq!(call(2.5), 2.5);
    }
}
