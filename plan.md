# Session A5 — Tree-walking interpreter (debug backend)

Spec refs: spec.md §9.1. Roadmap: roadmap.md "A5".

## Plan

1. `crates/calc-ir/src/interp.rs` (new module, re-exported from `calc-ir/src/lib.rs`
   alongside `ast_to_ir`/`ir`):
   - `pub enum Value { Number(f64) }` — single-variant today (calc-lang has exactly
     one runtime type), but an enum because the roadmap's Rust-learning goal for this
     session is explicitly "enum-based runtime values," and unlike A2's deferred
     `Stmt`, this type is exercised by every instruction, not unused scaffolding.
   - A `Temp`-indexed store (`Vec<Option<Value>>`, grown on write) instead of a
     name-keyed `HashMap`, since `ast_to_ir::lower`'s `next_temp` counter makes every
     `Temp` in a program globally unique and densely numbered, and every temp is
     written before it's read (the "roughly SSA-ish" property from A4).
   - `pub fn interpret(program: &Program) -> Value`, with recursive `exec_block`/
     `exec_instr`: `Const` writes a literal; `BinOp` applies `calc_syntax::BinOp` via
     `f64` arithmetic; `Copy` propagates a value; `If` reads `cond`, treats nonzero as
     true, and recurses into whichever nested `Block` is selected.
   - Unit tests: the same `{ let x = 1; if x { x + 1 } else { 2 } }` program A4's
     `ast_to_ir` test already lowers, a zero-condition `if` (falsiness), and plain
     arithmetic precedence (`2 + 3 * 4`).

2. `crates/calc-compiler/src/main.rs` (was a stub print): hand-rolled `env::args()`
   parsing for `calcc run --interpret <path>` (no `clap` yet — that's A13's job).
   Reads the file, runs `LalrpopFrontend::parse` → `calc_syntax::resolve` →
   `calc_ir::lower` → `calc_ir::interpret`, printing diagnostics/errors to stderr and
   returning `ExitCode::FAILURE` on failure at any stage, or the result number on
   success.

3. `DECISIONS.md`: new A5 entries for (a) the `Temp`-indexed `Vec` store instead of a
   name-keyed `HashMap`, and (b) nonzero-is-truthy semantics for `if`.

4. `calc-lang/docs/a5-tree-walking-interpreter.md` (new teaching-doc page) + a new
   line in `calc-lang/docs/README.md`'s reading-order list.

## Outcome

Implemented as planned, with one small departure: the store is `Vec<Option<Value>>`
rather than a plain `Vec<Value>` with a default fill value, so that reading an
unwritten `Temp` panics with a clear "malformed IR" message instead of silently
returning a wrong number — matching how `ast_to_ir::lookup` already panics on an
unresolved identifier rather than returning a bogus value.

`cargo build && cargo test` from `calc-lang/`: all 14 tests pass (5 new in
`calc_ir::interp` — including a dedicated nonzero-condition/then-branch case added
alongside the zero-condition one, so both sides of `If`'s branch are covered by their
own test — all prior tests unaffected). Manually verified `calcc run --interpret`
end-to-end against a sample program (`{ let x = 3; if x { x * 2 + 1 } else { 0 } }` →
`7`, exit 0) and a bad program (`y + 1` → resolve error on stderr, exit 1) and bad CLI
usage (usage line on stderr, exit 1).
