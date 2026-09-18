# A3 — Scopes, bindings, and name resolution

**Session code**: [`crates/calc-syntax/src/ast.rs`](../crates/calc-syntax/src/ast.rs),
[`crates/calc-syntax/src/calc.lalrpop`](../crates/calc-syntax/src/calc.lalrpop),
[`crates/calc-syntax/src/resolve.rs`](../crates/calc-syntax/src/resolve.rs),
[`crates/calc-syntax/src/lalrpop_frontend.rs`](../crates/calc-syntax/src/lalrpop_frontend.rs).
**Spec refs**: spec.md §5 (result types), §6.2 (structural role annotations), §6.3
(how actions and roles combine). **Prereqs**:
[A2](a2-typed-ast-and-semantic-actions.md).

A2 ended with `calc-lang`'s AST able to represent arithmetic and `if`/`else`, but
nothing that introduces a name — every identifier `x` a program used had to already
mean something to a human reader, because nothing in the compiler checked it meant
anything at all. A3 gives `calc-lang` its first binding construct and a real
`resolve` pass that checks names are used correctly, which is also the first session
to actually populate two fields of `RoleModel` (`scopes` and `bindings`) that have
sat empty since A1.

## What a binding and a scope actually are

A **binding** introduces a name (`x`) and associates it with a value, for some
region of the program. A **scope** is that region — the span of code where the
binding is visible. Outside its scope, the same name might mean nothing at all, or
might mean something completely different (a *shadowing* binding in an enclosing or
sibling scope). A **symbol table** is just the data structure that tracks "which
names are currently bound, and to what" as a compiler walks through a program's
scopes — in the simplest form, exactly what this session builds: a stack of
name-sets, one per currently-open scope.

## Why `let` needed a block, not just an expression

The obvious first design for `let` is the one many small languages use: an
expression, `let x = value in body`, evaluating `body` with `x` bound. `calc-lang`
almost went this way — it would have kept every construct an expression, matching
A1/A2's running theme. But it has a real problem for this session's actual goal:
`calc-lang` needs to detect **duplicate bindings** (redeclaring the same name twice
in one scope) as an error, and with `let ... in ...`, that error can never happen.
Every `let` opens a fresh scope holding exactly one name; nesting `let`s (`let x = 1
in let x = 2 in x`) never puts two bindings in the *same* scope — the inner `x` just
shadows the outer one, which is a different, legal thing.

So A3 adds two AST pieces together instead:

```rust
pub enum Stmt {
    Let { name: String, value: Expr },
}

pub enum Expr {
    // ...Number, Var, BinOp, If, unchanged from A2...
    Block {
        stmts: Vec<Stmt>,
        result: Box<Expr>,
    },
}
```

`Stmt::Let` is deliberately *not* an `Expr` variant — a declaration doesn't produce
a value the way `1 + 2` does, it just has an effect (introducing a name) on the
scope it's declared in. `Expr::Block` is what actually creates that scope: a
sequence of `Stmt`s sharing one set of bindings, followed by a final `Expr` whose
value is the block's value. Two `Stmt::Let`s in the same `Block` naming the same
identifier is now a real, reachable case — the actual duplicate-binding error this
session needs to test.

This also turns out to cost almost nothing in the grammar, because `if`/`else`
branches already required literal `{ ... }` braces since A1:

```
Term: Expr = {
    // ...
    "if" <c:Expr> <t:Block> "else" <e:Block> => ...,
    "(" <Expr> ")",
    Block,
};

Block: Expr = {
    "{" <stmts:Stmt*> <result:Expr> "}" => Expr::Block { stmts, result: Box::new(result) },
};

Stmt: Stmt = {
    "let" <name:Ident> "=" <value:Expr> ";" => Stmt::Let { name, value },
};
```

Giving `{ ... }` a real grammar rule (`Block`, zero-or-more `Stmt`s then an `Expr`)
and using it for `if`/`else`'s branches is a strict superset of A2's grammar:
`if x { 1 } else { 2 }` still parses exactly as before, just with an empty `stmts`
list on each branch. `Block` is also added directly to `Term`, so a block can appear
anywhere any other expression can (`2 + { let x = 1; x }`), not only inside
`if`/`else`.

## The `resolve` pass

[`resolve.rs`](../crates/calc-syntax/src/resolve.rs) walks an `Expr` tree with a
scope stack, `Vec<HashMap<String, ()>>` — a `Vec` used as a stack (push on entering
a scope, pop on leaving), and a `HashMap` per scope recording which names are bound
there. (The map's value is `()`, the empty tuple — the map's job here is purely "is
this name present," a set, but a set is the same data structure with the value type
erased.)

```rust
Expr::Block { stmts, result } => {
    scopes.push(HashMap::new());
    for stmt in stmts {
        let Stmt::Let { name, value } = stmt;
        resolve_expr(value, scopes, errors);          // resolve RHS first
        let scope = scopes.last_mut().expect("scope just pushed");
        if scope.contains_key(name) {
            errors.push(ResolveError::DuplicateBinding { name: name.clone() });
        } else {
            scope.insert(name.clone(), ());
        }
    }
    resolve_expr(result, scopes, errors);
    scopes.pop();
}
```

Two details worth calling out:

- **A `let` statement's value is resolved *before* its name is inserted.** This
  means `{ let x = x; x }` fails to resolve (the right-hand `x` isn't in scope yet)
  unless an outer scope already has an `x` — there's no accidental self-reference.
- **Duplicate-checking only looks at the current (innermost) scope**, via
  `scopes.last_mut()`. A name bound in an *outer* scope is invisible to this check —
  that's what makes shadowing across nested blocks legal while same-scope
  redeclaration isn't.

`Expr::Var(name)` does the opposite walk — search every open scope frame from
innermost to outermost (`scopes.iter().rev()`), and if none contains the name, it's
unresolved:

```rust
Expr::Var(name) => {
    if !scopes.iter().rev().any(|scope| scope.contains_key(name)) {
        errors.push(ResolveError::UnresolvedIdentifier { name: name.clone() });
    }
}
```

`resolve` accumulates every error it finds rather than stopping at the first one
(the same `Result<T, Vec<_>>` shape `ParserFrontend::parse` already uses), so a
program with several unrelated mistakes gets several diagnostics from one pass —
better for an author (and, later, the LSP) than a single crash-on-first-error.

## Filling in `RoleModel` for the first time

`RoleModel::scopes` and `RoleModel::bindings` (spec.md §6.2) have been empty
`Vec`s since A1 — there was nothing that opened a scope or declared a binding for
them to name. `LalrpopFrontend::role_model()` now reports both, using LALRPOP grammar
rule names exactly the way `control_flow` already names `"Term"` for `if`/`else`:

```rust
scopes: vec!["Block".to_string()],
bindings: vec!["Stmt".to_string()],
```

This is the concrete referent spec.md §6.3 describes: "any `#[scope_*]`-tagged rule
opens/closes a scope the same way, regardless of DSL" only makes sense once at least
one real DSL has a rule that actually does that. `calc-lang`'s `Block` and `Stmt`
rules are that first example — when Part B (`B4`/`B5`) generalizes role-driven
lowering across many DSLs, this is the concrete case its design gets checked
against.

## Why resolution isn't a built-in call yet

spec.md §7.3 describes `scope_enter`/`scope_exit`/`symbol_declare`/`symbol_lookup`
as built-ins like any other — swappable to FFI or even IPC, same as `add`/`mul` from
a later session. `resolve.rs` doesn't do that yet; it manipulates the `HashMap`
scope stack directly. That's intentional, not an oversight: roadmap.md's own framing
for this session is "role-annotation concepts... **by hand**," and the optional A17
session is specifically where these four operations get pulled behind a pluggable
interface, once Part B's generic role-driven lowering exists for that interface to
plug into. Building the concrete, hand-written version first — the same order A0–A2
built LALRPOP by hand before anything was generalized — gives A17 a tested
implementation to extract, rather than an abstraction designed before there's
anything real behind it.

See `DECISIONS.md`'s A3 entry for this same reasoning recorded in the project's
permanent decision log.
