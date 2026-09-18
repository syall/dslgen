# A1 — The `ParserFrontend` trait, and a LALRPOP implementation of it

**Session code**: [`crates/calc-syntax/src/frontend.rs`](../crates/calc-syntax/src/frontend.rs),
[`crates/calc-syntax/src/lalrpop_frontend.rs`](../crates/calc-syntax/src/lalrpop_frontend.rs),
[`crates/calc-syntax/src/calc.lalrpop`](../crates/calc-syntax/src/calc.lalrpop),
[`crates/calc-syntax/build.rs`](../crates/calc-syntax/build.rs).
**Spec refs**: spec.md §5. **Prereqs**: [A0](a0-workspace-and-parser-decision.md).

A0 decided *that* parsing would sit behind a trait, with LALRPOP as the first
implementation. A1 builds both: the trait itself, and enough of a LALRPOP grammar to
parse real `calc-lang` source — arithmetic with `+ - * /`, variables, and an
`if`/`else` expression.

## What a grammar-generator library actually does

Parsing text has two jobs that are conceptually separate even though they usually
happen together: **lexing** (splitting `"if x - 1 { 2 }"` into tokens — `if`, `x`,
`-`, `1`, `{`, `2`, `}` — deciding where one token ends and the next begins) and
**parsing** (deciding whether a sequence of tokens matches the language's grammar,
and building a tree out of it). Writing either by hand is tedious and easy to get
subtly wrong — operator precedence, ambiguous grammars, and good error messages are
all places a first attempt tends to have bugs a mature library already fixed years
ago.

LALRPOP is an **LR(1) parser generator**: you write a grammar file
([`calc.lalrpop`](../crates/calc-syntax/src/calc.lalrpop)), a `build.rs` step
compiles it into real Rust parsing code before your crate even gets to `src/`, and
you call the generated parser like any other Rust function. "LR(1)" describes the
parsing algorithm's shape (left-to-right scan, rightmost derivation, one token of
lookahead) — the practical upshot for someone writing a grammar is that **left-
recursive rules just work**, which matters immediately below.

## The `ParserFrontend` trait

[`frontend.rs`](../crates/calc-syntax/src/frontend.rs) defines the contract every
parsing strategy satisfies (spec.md §5), independent of which one is behind it:

```rust
pub trait ParserFrontend {
    type Ast;
    fn parse(&self, src: &str) -> Result<Self::Ast, Vec<ParseDiagnostic>>;
    fn role_model(&self) -> RoleModel;
}
```

Two things fall out of this shape on purpose. First, `Ast` is an **associated
type**, not a fixed one — each frontend gets to say what its own parse result looks
like, and nothing that calls `parse()` through the trait needs to know or care.
Second, `role_model()` exists *alongside* `parse()`, not folded into the AST itself
— it reports the spec.md §6.2 structural vocabulary (`#[identifier]`,
`#[keyword]`, `#[control_flow]`, ...) as its own separate piece of data, because
later infrastructure (the resolver in A3, IR lowering in A4 — both later
generalized in B4/B5 — and the generated LSP in A14) wants to ask "which rules are
keywords?" without caring about — or being coupled to — the specific shape of any
one frontend's AST.

## Building the grammar

[`calc.lalrpop`](../crates/calc-syntax/src/calc.lalrpop) has three rules layered
by precedence, which is the standard trick for encoding "`*`/`/` bind tighter than
`+`/`-`" directly into a grammar's structure rather than as a separate precedence
table:

```
Expr   := Expr ('+'|'-') Factor | Factor
Factor := Factor ('*'|'/') Term | Term
Term   := Num | Ident | If-expression | '(' Expr ')'
```

`Expr` can only ever add/subtract things that are already fully-multiplied/divided
`Factor`s, so `2 + 3 * 4` parses as `2 + (3 * 4)` without any explicit precedence
declaration. Both `Expr` and `Factor` are **left-recursive** (`Expr := Expr '+' ...`
refers to itself as its own first symbol) — this is the natural way to write "one or
more, left-to-right" in a grammar, and it's exactly what LALRPOP's LR(1) algorithm
handles natively. (A hand-written recursive-descent parser, in contrast, would
recurse infinitely on a rule shaped like that without extra work — precedence-
climbing — to rewrite the recursion away. That cost is exactly what session
A1-custom would make you feel directly.)

`if`/`else` is written as a `Term` — an *expression*, not a statement — because
`calc-lang` at this stage has no separate statement/expression distinction yet
(that arrives in A2): `if x - 1 { 2 + 3 * 4 } else { y / 2 }` evaluates to whichever
branch's value, exactly like Rust's own `if` expressions.

One detail worth noticing because it's a preview of spec.md §6.2's later concerns:
the grammar never has to specially handle "what if a variable is named `if`?"
LALRPOP's generated lexer prioritizes literal string tokens (`"if"`, `"else"`) over
a regex terminal (`Ident`'s `[a-zA-Z_][a-zA-Z0-9_]*`) whenever both could match the
same text. That's precisely the ambiguity spec.md §6.2 says DSL-Generator should be
able to "cross-check `#[keyword]` tokens against `#[identifier]` token patterns" to
catch — here, the parser-generator library resolves it for free, by construction.

## Wiring the grammar into Rust

[`build.rs`](../crates/calc-syntax/build.rs) runs `lalrpop::process_root()`, which
finds `calc.lalrpop` and compiles it into generated Rust source under `OUT_DIR` at
build time. [`lalrpop_frontend.rs`](../crates/calc-syntax/src/lalrpop_frontend.rs)
pulls that generated code in with `lalrpop_util::lalrpop_mod!(pub calc);` — after
that macro, `calc::ExprParser` is a real, usable Rust type. `LalrpopFrontend::parse`
does nothing more than call it and convert LALRPOP's own error type into this
crate's `ParseDiagnostic`:

```rust
fn parse(&self, src: &str) -> Result<Self::Ast, Vec<ParseDiagnostic>> {
    calc::ExprParser::new()
        .parse(src)
        .map_err(|err| vec![convert_error(err)])
}
```

That one line is the entire point of the trait: every later session that wants to
parse `calc-lang` source calls `LalrpopFrontend.parse(src)` — or, once A1-pest/
A1-custom exist, a different frontend's `.parse(src)` — through the exact same
method signature.

## `RawAst`: a deliberately throwaway shape

`RawAst` (also in `lalrpop_frontend.rs`) is the `Ast` this frontend's `parse()`
returns — enough to prove parsing works (`Number`, `Var`, `BinOp`, `If`), but not
yet the properly-designed typed AST the rest of the pipeline will build on. Session
A2 replaces it with a real `calc-syntax::ast` module — an `Expr` type; `Stmt` turned
out to wait for A3, once there was actually something worth binding (see A2's own
doc page and `DECISIONS.md` for that reasoning) — once there's a firmer sense of
what semantic actions and IR lowering actually need from it. This is a working
example of `spec.md`/`roadmap.md`'s "prefer small, focused, incremental additions"
philosophy in practice: get the trait boundary proven end to end first, redesign the
payload once, deliberately, rather than guessing at the final shape up front.

## Testing through the trait, not the generated type

The session's test —

```rust
let frontend = LalrpopFrontend;
let ast = frontend.parse("if x - 1 { 2 + 3 * 4 } else { y / 2 }").expect("should parse");
```

— calls `parse()` on `LalrpopFrontend` through its inherent method, which is also
its trait method (Rust doesn't distinguish the two at the call site once the trait
is in scope). It never references `calc::ExprParser` directly. That's intentional:
it's the same test that A1-pest and A1-custom will run, unmodified, against their
own frontends — the proof that downstream code genuinely doesn't know or care which
`ParserFrontend` produced a given AST.
