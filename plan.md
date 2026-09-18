# Session A1 — The `ParserFrontend` trait, and a LALRPOP implementation of it

Spec refs: spec.md §5. Roadmap: roadmap.md "A1".

## Plan

1. Add a `ParserFrontend` trait to `calc-syntax` (`frontend.rs`): an associated `Ast`
   type, `parse(&self, src: &str) -> Result<Self::Ast, Vec<ParseDiagnostic>>`, and
   `role_model(&self) -> RoleModel` reporting the `#[identifier]`/`#[keyword]`/
   `#[control_flow]` categories from spec.md §6.2 (only the categories calc-lang's
   grammar uses so far; `scopes`/`bindings` fill in at A3).
2. Add a LALRPOP grammar (`calc.lalrpop`) for arithmetic expressions (`+ - * /` with
   standard precedence/associativity), numeric literals, variables, and `if`/`else`,
   wired up via `build.rs` + `lalrpop_util::lalrpop_mod!`.
3. Implement `LalrpopFrontend: ParserFrontend` (`lalrpop_frontend.rs`) over a
   throwaway `RawAst` enum — intentionally not the real typed AST (that's A2) — just
   enough to prove the trait boundary end to end.
4. Test only through the trait (never the LALRPOP-generated `calc::ExprParser` type
   directly), so later sessions can swap frontends without the test caring: one test
   parsing `if x - 1 { 2 + 3 * 4 } else { y / 2 }` and asserting on the `RawAst` shape,
   one asserting a parse error surfaces as a `ParseDiagnostic`, one asserting
   `role_model()` reports `if`/`else` as keywords and one `control_flow` entry.

## Outcome

Implemented as planned; no departures. `cargo build && cargo test` from `calc-lang/`
passes (3 new tests in `calc-syntax`, all others unaffected). No new entry needed in
`DECISIONS.md` — A1 executes A0's already-logged LALRPOP choice rather than making a
new one.
