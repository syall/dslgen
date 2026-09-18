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
