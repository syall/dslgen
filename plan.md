# Session A4 — Mid-level IR and the lowering pass

Spec refs: spec.md §8.1. Roadmap: roadmap.md "A4".

## Plan

1. Add `crates/calc-ir/src/ir.rs`: the mid-level IR types — `Temp` (a
   three-address-code temporary, doc-commented since the term isn't self-evident),
   `Instr` (`Const`/`BinOp`/`Copy`/`If`, reusing `calc_syntax::BinOp` as-is), `Block`
   (`Vec<Instr>`, straight-line, no internal branches), and `Program { body, result
   }`. `If` nests two `Block`s directly (structured control flow) rather than using
   jump-connected basic blocks.
2. Add `crates/calc-ir/src/ast_to_ir.rs`: `lower(expr: &Expr) -> Program`, a
   recursive `lower_expr` mirroring `resolve.rs`'s scope-stack walk
   (`Vec<HashMap<String, Temp>>`, pushed/popped per `Expr::Block`). Variables reuse
   the temp their binding computed into (no load/store instructions needed, since
   A3's `resolve` already guarantees bind-before-use). `If` lowers each branch into
   its own instruction sequence, ending each with an `Instr::Copy` into one shared
   `dst` temp ("phi via copies", since a structured `If` has no natural point for an
   SSA `Phi` node). Assumes its input already passed `calc_syntax::resolve()`; an
   unresolved lookup panics rather than returning a `Result` (lowering isn't a
   validation boundary).
3. Wire `calc-ir`'s `Cargo.toml` to depend on `calc-syntax` (path dependency); update
   `lib.rs` to `pub mod ir; pub mod ast_to_ir;` plus re-exports.
4. Test in `ast_to_ir.rs`: lower `"{ let x = 1; if x { x + 1 } else { 2 } }"` (parsed
   via `LalrpopFrontend`) and assert the resulting `Program` equals a literal
   expected IR value, checking every instruction and temp number.
5. Log two choices in `DECISIONS.md`: deferring `Loop`/`Break`/`Continue`/`Return`
   IR variants (spec.md §8.1 lists them, but `calc-lang`'s AST has no construct to
   lower from yet — same reasoning as A2's `Stmt` deferral) and the "phi via copies"
   technique for `If`'s branch merge.
6. Write `calc-lang/docs/a4-mid-level-ir-and-lowering.md` (why compilers use a
   mid-level IR, three-address code, structured control-flow nodes vs. basic-block
   jumps, the "phi via copies" merge, a worked trace of the test program) and add it
   to `docs/README.md`'s index.

## Outcome

Implemented as planned; no departures. `cargo build && cargo test` from `calc-lang/`
passes: 1 new test in `calc-ir` (`ast_to_ir::tests::lowers_a_let_bound_if_expression`)
plus all 9 existing `calc-syntax` tests (A1–A3) unaffected.
