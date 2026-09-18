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

## A3 — Block-scoped `let` statements over ML-style `let ... in ...`

calc-lang's first binding construct is `Stmt::Let { name, value }`, usable only
inside `Expr::Block { stmts: Vec<Stmt>, result: Box<Expr> }` — `{ let x = 1; let y
= 2; x + y }` — rather than an ML-style `let x = v in body` *expression*.

The reason is that ML-style `let` makes "duplicate binding" (one of the two error
cases the roadmap names for this session) structurally unreachable: every `let`
would open its own brand-new scope containing exactly one name, so two bindings can
never land in the same scope no matter how they're nested — nested `let`s are
always shadowing, never a same-scope redeclaration. A block holding a *sequence* of
`let` statements sharing one scope is what makes redeclaring a name within that same
scope a real, testable case, distinct from shadowing across nested blocks (allowed).

This is also a backward-compatible grammar change, not a rewrite: `if`/`else`
branches already required literal `{ ... }` braces since A1. Giving `{ ... }` real
block syntax (`"{" Stmt* Expr "}"`) and using it for the `if`/`else` branches is a
strict superset — `if x { 1 } else { 2 }` still parses to the same `Expr::If` shape,
just with an empty `stmts` list on each branch's `Block`. `Block` is also added as a
plain `Term` alternative, so a block can appear as any expression (`2 + { let x = 1;
x }`), not just inside `if`/`else`.

Resolution (`calc-syntax::resolve`) is hand-written directly over `Expr`/`Stmt` — a
`Vec<HashMap<String, ()>>` scope stack pushed/popped at each `Expr::Block` — rather
than routed through the `scope_enter`/`scope_exit`/`symbol_declare`/`symbol_lookup`
built-in indirection spec.md §7.3 describes. That refactor is deliberately deferred
to the optional A17 session, once Part B's generic role-driven lowering exists to
plug a pluggable implementation into; building it by hand first here (per roadmap
A3's own framing, "role-annotation concepts... by hand") gives A17 a concrete,
tested implementation to extract behind that interface later, mirroring how A0–A2
built LALRPOP by hand before any generalization was attempted.
