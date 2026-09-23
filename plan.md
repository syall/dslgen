# Session A11 — Built-ins, kind 3: subprocess/IPC bridge

Spec refs: spec.md §7 (IPC half), §14.5, §14.6, §14.7. Roadmap: roadmap.md "A11".
Prereqs: A9.

## Plan

1. **Grammar/AST**: add a single fixed `"print" "(" <Expr> ")"` production (a `Term`
   alternative in `calc-syntax/src/calc.lalrpop`), and `Expr::Print(Box<Expr>)` in
   `calc-syntax/src/ast.rs`. Deliberately *not* general call syntax (no arbitrary
   callee name, no argument-count generality) — same "operators are the surface"
   principle A9's DECISIONS.md entry established, extended to one fixed unary
   keyword. Requiring parens sidesteps picking an arbitrary unary-operator
   precedence tier.
2. **Resolver**: add `Expr::Print(inner) => resolve_expr(inner, ...)` to
   `calc-syntax/src/resolve.rs`'s exhaustive match.
3. **Lowering**: in `calc-ir/src/ast_to_ir.rs`, lower `Expr::Print(inner)` to the
   existing generic `Instr::CallBuiltin { dst, name: "print", args: vec![arg] }` —
   no IR changes needed, `CallBuiltin` already supports arbitrary arity.
4. **`calc-runtime`**: new `src/ipc_runtime.rs` (`std`-only — see Outcome for why),
   spawning `python3 -c <embedded print.py> <x>`, capturing stdout via
   `Command::output()`, relaying it to this process's own stdout, and returning `x`
   unchanged. New `native/print.py`. `lib.rs` gains `mod ipc_runtime;`, a real
   `#[no_mangle] extern "C" fn calc_print`, a new `BindingKind::Ipc` variant, and a
   `print` `BUILTINS` entry whose `eval` calls the real `calc_print` — never a
   reimplementation, per A10's established principle.
5. **Tests**: `calc-runtime` unit tests on `ipc_runtime`/`calc_print` directly;
   `calc-ir` interpreter test running `print(...)`; `calc-compiler` end-to-end
   link+run tests through both backends if the minimal-CRT compiled-executable
   environment turns out to support spawning a subprocess cleanly, otherwise
   recorded as a gap instead of forced.
6. **plan.md** (this file), **DECISIONS.md**, and a new teaching-doc page
   (`calc-lang/docs/a11-subprocess-ipc-builtins.md`, added to `docs/README.md`'s
   reading order).

A key design question surfaced during planning: `calc-runtime/src/lib.rs` must
compile standalone via a bare `rustc --crate-type=staticlib` invocation with zero
`--extern` flags (established A9/A10), so any file it `mod`-declares must stay
dependency-free — ruling out roadmap.md's own suggested `serde_json` request/
response protocol unless the build mechanism itself changed first. Discussed with
the user: kept a hand-rolled, dependency-free protocol for this session (lowest
complexity, and — as it turned out — lets `print` link into compiled programs too,
not just the interpreter), while recording as a deliberate, standing project
direction (not scoped to any one future session) that none of the three binding
kinds should be architecturally forced to stay dependency-free long-term; once the
build mechanism is properly fixed (naturally A12's territory), `ipc_runtime.rs`
should switch to a real `serde_json` protocol to demonstrate the fix works.

## Outcome

Implemented as planned, with two real findings during implementation that changed
what was originally hedged as a "stretch goal":

- **Backend/link support worked, contradicting A10's own forecast that A11 would
  stay interpreter-only.** Because `ipc_runtime.rs` stayed `std`-only, `calc_print`
  compiled into the same static archive as `calc_add`/`calc_mul` with zero backend
  code changes (`cranelift_backend.rs`/`llvm_backend.rs` already dispatch built-ins
  generically by name/arity). Actually *linking* a compiled program that calls
  `print` failed on MSVC with over thirty `LNK2019` errors, all resolving to three
  missing import libraries — `ws2_32.lib`, `ntdll.lib`, `userenv.lib` — pulled in by
  `std::process::Command`'s Windows implementation (named-pipe I/O for piping
  stdio, plus unused-but-present networking/profile-lookup code) that a hand-built
  object file's absent `/DEFAULTLIB` directives don't supply. Added all three to
  `link_stub.rs`'s existing MSVC link line; verified end-to-end on both backends
  (`links_and_runs_a_program_that_calls_the_ipc_builtin` in `link_stub.rs`, plus
  manual `calcc build --backend=cranelift`/`--backend=llvm` runs). Consequence for
  A12: `link_stub.rs` already links all three built-in kinds together, so A12's own
  stated deliverable is already true — its real remaining scope is graduating the
  ad hoc bare-`rustc`/`cc` build mechanism to a durable one (and, per the
  discussion above, switching `ipc_runtime.rs` to `serde_json` once that lands),
  not making IPC linkable.
- **A genuine Windows portability bug, found and fixed, not anticipated in the
  plan.** `Command::new("python3")` resolves to a non-functional "app execution
  alias" stub on this project's own dev machine (spawns successfully, exits 9009,
  never runs anything) even though a real interpreter is on `PATH` as plain
  `python`. `run_print_script` now tries `python3` first, falling back to `python`
  on any non-success exit (not just a spawn failure, since the stub spawns fine).
- Minor: a clippy `approx_constant` lint fired on a test using `3.14159` as an
  argument (close enough to `f64::consts::PI`); changed to `12.3456`.
- **Post-review pivot: `print` moved from `Expr::Print` to `Stmt::Print`.** After
  the session was otherwise complete, revisited on request: `print` is run for a
  side effect, and `calc-syntax/ast.rs`'s own doc comment already states the test
  this codebase uses for `Stmt` vs. `Expr` (`Stmt` is for constructs that aren't
  themselves value-producing). Moved the AST variant, the grammar production (now
  requires a trailing `;`, like `let`), and lowering (now in the block statement
  loop, leaving the call's `dst` intentionally unbound) accordingly; updated every
  test and the "Try it" example that assumed a bare `print(<expr>)` was a complete
  program, since `Stmt` only appears inside `{ ... }` now. See DECISIONS.md's A11
  entry and the teaching doc's new "Why `print` is a statement, not an expression"
  section for the full reasoning. Full re-verification (`cargo test --workspace`,
  both feature sets) passes after the change.
- **Second post-review pivot, then reverted in full: `print` made genuinely `void`.**
  Asked directly, after the `Stmt` pivot above, whether `print`'s call should return
  nothing at all rather than an `f64` calc-lang's grammar happens not to bind: this
  was built out completely (`calc_ir::Instr::CallBuiltin.dst` → `Option<Temp>`;
  `calc_runtime::Builtin::eval` → `fn(&[f64]) -> Option<f64>`; `calc_print` → `extern
  "C" fn(f64)` with no return; both backends declaring a return-type-less callee
  signature when `dst` was `None`) and verified working end-to-end on both backends.
  Asked next whether the pre-`void` return value was ever actually *usable*: no —
  `Stmt::Print` never inserted its result into `env` the way `Stmt::Let` does, so it
  was unreachable from calc-lang either way. A change with zero observable behavior,
  touching the shared IR node every built-in lowers to, both backends' codegen, and
  the interpreter's dispatch, wasn't worth keeping — reverted in full back to the
  post-`Stmt`-pivot state (`dst: Temp` always allocated, `eval: fn(&[f64]) -> f64`,
  `calc_print` returns `x` unchanged). See DECISIONS.md's A11 entry ("Considered,
  implemented, then reverted...") and the teaching doc's note under "Why `print` is
  a statement, not an expression" for the full reasoning. Full re-verification
  (`cargo build`/`test --workspace`, both feature sets) passes after the revert.
- **Third post-review addition: a new top-level `Program` grammar rule, distinct
  from `Expr`.** Making `print` a `Stmt` (above) meant every program using it needed
  a wrapping `{ ... }`, even a one-liner. Requested explicitly: a top-level program
  should act like an implicit block, so `print(1); 2` is valid on its own, while
  still requiring a final result expression. `calc.lalrpop` gains `pub Program:
  Expr = <stmts:Stmt*> <result:Expr> => ...`, parsed via the new
  `calc::ProgramParser` (`LalrpopFrontend::parse` calls this now; `Expr` drops
  `pub`). Backward compatibility handled in the action itself: an empty `stmts`
  produces the bare `result` `Expr` directly, not `Expr::Block { stmts: vec![], ..
  }` — every existing statement-free program keeps its exact original AST shape;
  only a program that actually uses top-level statements gets wrapped, and
  `resolve`/`lower`/both backends already handle that shape correctly with zero
  further changes. Added grammar-level tests (`calc-syntax`) for both the new
  capability and the backward-compatibility guarantee, plus an interpreter
  end-to-end test and a manual `calcc build --backend=cranelift` smoke test. See
  DECISIONS.md's A11 entry and the teaching doc's new "The top-level `Program`
  rule: an implicit block" section. Full re-verification (`cargo fmt`, `clippy`,
  `doc`, `audit`, `build`, `test`, both feature sets) passes.
- **Documentation-only polish, requested after the code was otherwise done**: added
  a "What `include_str!` actually does" / "what's needed at run time, and what
  isn't" blurb to the teaching doc's protocol section, plus a standalone diagram
  (`docs/images/a11-print-script-embedding.svg`, hand-translated from an initial
  visualize-tool sketch into real inline SVG styles/colors so it renders correctly
  as a plain file on GitHub, not just inside that tool's own host) showing
  `native/print.py` → `include_str!` → `PRINT_SCRIPT` → both build artifacts → both
  binaries → the `python3 -c` spawn, with the two-column section recentered after
  an initial off-center mistake and a label clarifying `PRINT_SCRIPT` holds the
  file's full text, not a path. While double-checking this pass, found and fixed a
  genuinely broken sentence in `docs/README.md`'s A11 entry (a dropped word from an
  earlier edit — "...linker errors (...) that took to make that work") left over
  from a previous revision.

`cargo fmt`, `cargo check --workspace`, `cargo clippy --workspace --all-targets --
-D warnings`, `RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps`,
`cargo audit`, `cargo build --workspace`, and `cargo test --workspace` all pass
with default (Cranelift) features; the same four build/test commands were also run
locally with `--features backend-llvm` (not covered by CI) and pass.
