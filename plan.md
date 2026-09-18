# Session A2 — Typed AST and semantic actions

Spec refs: spec.md §5 (result types), §6.1 (actions). Roadmap: roadmap.md "A2".

## Plan

1. Add `crates/calc-syntax/src/ast.rs`: the real typed AST, replacing A1's throwaway
   `RawAst`. `Expr` (`Number`, `Var`, `BinOp`, and `If` with named fields `cond`/
   `then_branch`/`else_branch`) and `BinOp` (`Add`/`Sub`/`Mul`/`Div`), both deriving
   `Debug, Clone, PartialEq` (`BinOp` also `Copy, Eq`).
2. Update `calc.lalrpop`'s actions to construct `crate::ast::Expr` directly instead of
   the frontend-local `RawAst`; grammar shape (the `Expr`/`Factor`/`Term` precedence
   ladder) is unchanged.
3. Update `lalrpop_frontend.rs`: delete the `RawAst`/`BinOp` definitions (now in
   `ast.rs`), set `LalrpopFrontend::Ast = crate::ast::Expr`. `parse()`/`role_model()`
   unchanged. Rewrite the existing `Debug`-string-comparison test to build the
   expected `Expr` value and use `assert_eq!` (now that `Expr: PartialEq`); add the
   roadmap's named test parsing `"if x { 1 } else { 2 }"` and asserting on the typed
   shape.
4. Export the new module from `calc-syntax`'s `lib.rs` (`pub mod ast;` + re-export).
5. Log the `RawAst` → `Expr` promotion and, explicitly, the decision to **not** add a
   `Stmt` type yet in `DECISIONS.md` — calc-lang has no non-expression construct for
   one to represent until A3 adds a binding/declaration form; an empty, untested
   `Stmt` enum would be ahead-of-need scaffolding.
6. Write `calc-lang/docs/a2-typed-ast-and-semantic-actions.md` (what an AST is and why
   it's typed per-rule, the semantic-action mechanism, and the `Stmt` deferral
   rationale) and add it to `docs/README.md`'s reading-order index.

## Outcome

Implemented as planned; no departures. `cargo build && cargo test` from `calc-lang/`
passes: 4 tests in `calc-syntax` (the 3 from A1, updated, plus the new named-shape
test), all others unaffected. No remaining references to `RawAst` in `calc-syntax`
code (only in A1's doc, describing history accurately).
