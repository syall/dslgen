# A2 — Typed AST and semantic actions

**Session code**: [`crates/calc-syntax/src/ast.rs`](../crates/calc-syntax/src/ast.rs),
[`crates/calc-syntax/src/calc.lalrpop`](../crates/calc-syntax/src/calc.lalrpop),
[`crates/calc-syntax/src/lalrpop_frontend.rs`](../crates/calc-syntax/src/lalrpop_frontend.rs).
**Spec refs**: spec.md §5 (result types), §6.1 (actions). **Prereqs**:
[A1](a1-parserfrontend-trait-and-lalrpop.md).

A1 proved the `ParserFrontend` trait boundary end to end, but its `Ast` type
(`RawAst`) was explicitly a placeholder — "enough to prove parsing works ... but not
yet the properly-designed typed AST the rest of the pipeline will build on." A2 does
that redesign: a real `calc-syntax::ast` module, with the grammar's actions building
it directly.

## What an AST is, and why it's typed per-rule

A parser's job is really two jobs. First, decide whether the input matches the
grammar at all (A1's territory). Second, if it does, build some in-memory value that
represents *what the program means*, structurally — an **abstract syntax tree**. It's
"abstract" precisely because it throws away everything about the concrete syntax that
doesn't matter to meaning: `2 + 3 * 4` and `2+3*4` parse to the exact same tree, even
though their token streams differ in whitespace.

The "per-rule" part of "typed per-rule" matters too. It would be possible to have
every grammar rule just return some generic `ParseTree` node holding a rule name and
a list of children — but then every consumer of the tree (the resolver in A3, the
lowering pass in A4, an interpreter in A5) would need to re-discover, at runtime,
"is this a number or a variable reference?" A **typed** AST puts that decision in
Rust's type system instead: `Expr::Number(f64)` and `Expr::Var(String)` are
different `enum` variants the compiler can check `match` arms against exhaustively.
Getting this wrong (forgetting to handle a variant somewhere downstream) becomes a
compile error, not a runtime surprise.

## `ast.rs`: the real shape

```rust
#[derive(Debug, Clone, PartialEq)]
pub enum Expr {
    Number(f64),
    Var(String),
    BinOp(Box<Expr>, BinOp, Box<Expr>),
    If {
        cond: Box<Expr>,
        then_branch: Box<Expr>,
        else_branch: Box<Expr>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinOp { Add, Sub, Mul, Div }
```

Two Rust details worth calling out:

- **`Box<Expr>` for recursion.** `Expr` refers to itself (an `If`'s condition is
  itself an `Expr`), and Rust needs to know a type's size at compile time. A
  self-referential enum with no indirection would have unbounded size — `Box` puts
  the recursive part on the heap, so `Expr` itself has a fixed size (a pointer plus a
  discriminant) no matter how deep a program's expression tree gets.
- **Named fields on `If`.** `RawAst::If` was positional —
  `If(Box<RawAst>, Box<RawAst>, Box<RawAst>)` — which meant every construction and
  pattern match had to remember "condition, then, else" by position alone. Now that
  this is meant to be the AST's permanent shape (not a throwaway), `If { cond,
  then_branch, else_branch }` makes that order self-documenting at every call site.

`PartialEq` is derived too, which A1's `RawAst` didn't have — it lets tests compare a
parsed `Expr` against an expected value directly (`assert_eq!`) instead of comparing
`Debug`-formatted strings, which is both clearer to read and a stronger check (string
comparison can't tell you *why* two trees differ; a derived `PartialEq` failure prints
both full values).

## Semantic actions: from concrete syntax to `Expr`

[`calc.lalrpop`](../crates/calc-syntax/src/calc.lalrpop)'s rules are unchanged in
shape from A1 — same three-rule precedence ladder (`Expr` → `Factor` → `Term`) — but
every action now constructs `crate::ast::Expr` directly instead of the frontend-local
`RawAst`:

```
"if" <c:Expr> "{" <t:Expr> "}" "else" "{" <e:Expr> "}" =>
    Expr::If { cond: Box::new(c), then_branch: Box::new(t), else_branch: Box::new(e) },
```

This is spec.md §6.1's "semantic action" concept made concrete: the grammar rule
matches *syntax* (the literal tokens `if`, `{`, `}`, `else`), and its action is the
Rust expression that turns the pieces LALRPOP captured (`c`, `t`, `e`) into the
*meaning* — a typed `Expr::If` node. Because LALRPOP lets actions be real inline Rust
(A0's reason for picking it first), this action is just a plain enum-constructor call
— no separate action language needed, exactly as A0 anticipated.

`lalrpop_frontend.rs` shrinks correspondingly: `RawAst`/`BinOp` are deleted from it
entirely (they live in `ast.rs` now), and `LalrpopFrontend`'s `Ast` associated type
becomes `crate::ast::Expr`. Nothing else about the trait impl changes — `parse()` and
`role_model()` are exactly as A1 left them, which is the point of having designed the
trait around an associated type in the first place: swapping out *what* a frontend
produces doesn't touch *how* it's called.

## Testing the typed shape, not just "it parsed"

A1's test compared a `Debug`-formatted string. A2 replaces that with a real value
comparison, and adds the case the roadmap names directly:

```rust
let ast = frontend.parse("if x { 1 } else { 2 }").expect("should parse");
assert_eq!(
    ast,
    Expr::If {
        cond: Box::new(Expr::Var("x".to_string())),
        then_branch: Box::new(Expr::Number(1.0)),
        else_branch: Box::new(Expr::Number(2.0)),
    }
);
```

This is a stronger test than A1's: it asserts on the tree's actual *structure*
(which variant, in what nesting), not on a formatted string of it.

## Why there's no `Stmt` yet

Some descriptions of this stage of a compiler talk about "`Expr`/`Stmt`" types
together, and it's worth explaining why calc-lang only gets the former right now.
A **statement** is generally a construct that doesn't itself produce a value — a
variable declaration, an assignment, a loop. calc-lang doesn't have one of those yet:
every construct so far, including `if`/`else`, evaluates to a value (per A1: "exactly
like Rust's own `if` expressions"). Adding a `Stmt` enum today would mean adding a
type with no grammar rule producing it and no test able to exercise it — dead code
whose only justification would be "a future session will probably want this," which
is exactly the kind of ahead-of-need design this project's conventions (CLAUDE.md)
call out to avoid.

A3 ("Scopes, symbol table and role-annotation concepts", not yet built) is the
session that actually needs a non-expression construct — a binding/declaration, to
have something concrete to resolve names against and to trigger "duplicate binding"
errors — so that's the natural, need-driven point for `Stmt` to enter the AST, backed
by real syntax and real tests. See `DECISIONS.md`'s A2 entry for the same reasoning
in the project's permanent decision log.
