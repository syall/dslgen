# calc-lang decision log

## A0 — Parser frontend

Parsing will be built from the start behind a `ParserFrontend` trait (spec.md §5),
mirroring the pluggable `Backend` trait from §8.1 one layer earlier. No stage
downstream of parsing (role-driven lowering, codegen, the LSP) will depend on which
concrete frontend produced the AST — only on the trait's typed AST + role model
output.

The first concrete implementation is **LALRPOP**: it supports inline Rust actions
attached directly to grammar alternatives, which sidesteps designing a separate
action language (§6.1, §14.2) until/unless that's actually needed later. It also
handles left-recursive expression grammars naturally, which matters for calc-lang's
arithmetic expressions.

**pest** (grammar/actions kept separate) and a **hand-written recursive-descent**
frontend are planned as alternate implementations of the same trait, in sessions
A1-pest and A1-custom respectively, once A1 exists to write a shared test suite to
compare them against.

## A2 — Typed AST, and deferring `Stmt` to A3

A1's throwaway `RawAst` is promoted to a real `calc-syntax::ast::Expr`, with the
grammar's actions building it directly. `If`'s three sub-expressions moved from
positional fields (`If(Box<RawAst>, Box<RawAst>, Box<RawAst>)`) to named ones (`If {
cond, then_branch, else_branch }`), since this is meant to be the AST's permanent
shape and named fields make call sites and pattern matches self-documenting.

roadmap.md's session template describes A2's deliverable as an "`Expr`/`Stmt`"
module, but `Stmt` is deliberately not added yet: calc-lang has no construct that
isn't itself an expression-with-a-value — `if`/`else` evaluates to a branch's value
exactly like Rust's own `if` expression (A1's doc) — so there is nothing for a
statement type to represent, and no grammar rule that would produce one. An empty,
untested `Stmt` enum would be unused scaffolding, which cuts against this project's
own "don't restructure or generalize ahead of need" convention (CLAUDE.md). A3
("Scopes, symbol table") is the session that actually introduces a binding/
declaration construct to test "duplicate binding" errors against — that's the
natural, need-driven point to add `Stmt`, backed by real grammar and tests.
