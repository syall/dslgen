# Session A3 — Bindings in the AST, scopes, symbol table, and role-annotation concepts (by hand)

Spec refs: spec.md §5 (result types, for the new AST node), §6.2, §6.3. Roadmap: roadmap.md "A3".

## Plan

1. Add `crates/calc-syntax/src/ast.rs`: `Stmt::Let { name, value }` (a declaration,
   not an expression) and `Expr::Block { stmts: Vec<Stmt>, result: Box<Expr> }` (a
   sequence of declarations sharing one scope, followed by a result expression) —
   chosen over an ML-style `let ... in ...` expression because that design makes
   "duplicate binding" structurally unreachable (every `let` would open its own
   fresh single-name scope). See `DECISIONS.md`'s A3 entry.
2. Update `calc.lalrpop`: add `Block`/`Stmt` grammar rules; change `if`/`else`'s
   branches from bare `"{" <t:Expr> "}"` to `<t:Block>` (backward-compatible
   superset — `if x { 1 } else { 2 }` still parses the same, with empty-`stmts`
   `Block`s); add `Block` as a `Term` alternative so blocks can appear as any
   expression.
3. Update `lalrpop_frontend.rs`'s `role_model()`: add `"let"` to `keywords`, and
   populate `scopes: vec!["Block"]` / `bindings: vec!["Stmt"]` — the first time
   either field has been non-empty since A1.
4. Add `crates/calc-syntax/src/resolve.rs`: `resolve(expr: &Expr) -> Result<(),
   Vec<ResolveError>>` (`UnresolvedIdentifier`/`DuplicateBinding`), walking `Expr`
   with a `Vec<HashMap<String, ()>>` scope stack pushed/popped per `Expr::Block`;
   accumulates every error rather than stopping at the first.
5. Export `resolve`/`ResolveError`/`Stmt` from `calc-syntax`'s `lib.rs`.
6. Update A2's existing `If`-shape tests (branches are now `Expr::Block`s with empty
   `stmts`) and add tests: parsing a `Block` with `let` statements, `resolve`'s two
   error cases (unresolved identifier, duplicate binding), `resolve` succeeding on a
   valid block, and cross-scope shadowing being allowed; extend the `role_model()`
   test for the new `scopes`/`bindings`/`"let"` entries.
7. Log the `let ... in ...`-vs-block-statements choice (and deferring the §7.3
   built-in refactor to optional session A17) in `DECISIONS.md`.
8. Write `calc-lang/docs/a3-scopes-bindings-and-resolution.md` (scopes/bindings/
   symbol-table concepts, the design choice, `resolve`'s walk, and why resolution
   isn't routed through built-ins yet) and add it to `docs/README.md`'s index.

## Outcome

Implemented as planned; no departures. `cargo build && cargo test` from `calc-lang/`
passes: 9 tests in `calc-syntax` (the 4 existing A1/A2 tests — 2 updated for the new
`Expr::Block`-wrapped `If` branches, 1 extended for the new `RoleModel` fields — plus
5 new: a `Block`/`Stmt` parse-shape test and 4 `resolve` tests covering both error
cases, a successful resolution, and cross-scope shadowing), all others unaffected.
