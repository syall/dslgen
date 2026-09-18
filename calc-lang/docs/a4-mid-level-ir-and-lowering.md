# A4 — Mid-level IR and the lowering pass

**Session code**: [`crates/calc-ir/src/ir.rs`](../crates/calc-ir/src/ir.rs),
[`crates/calc-ir/src/ast_to_ir.rs`](../crates/calc-ir/src/ast_to_ir.rs).
**Spec refs**: spec.md §8.1 (mid-level IR). **Prereqs**:
[A3](a3-scopes-bindings-and-resolution.md).

By A3, `calc-lang` has a typed AST and a `resolve` pass that checks every name makes
sense. What it doesn't have yet is anything that looks like what a codegen backend
(a later session) actually wants to consume. A4 adds a second data model — the
**mid-level IR** — and a `lower()` function that turns a resolved AST into it.

## Why not generate code straight from the AST?

You *could* write a codegen backend that walks `Expr` directly and emits machine
code as it goes — many toy interpreters do exactly that. It gets painful fast once
you have more than one backend (Cranelift, LLVM, an interpreter — all planned for
`calc-lang`): each one would have to re-solve the same problems (how to turn a
nested `if`-expression into control flow, how to name intermediate values) in its
own way, and any AST shape change would ripple into every backend at once.

A **mid-level IR** is a second, deliberately simpler tree/list-shaped
representation that sits between the two: one lowering pass turns AST into IR, and
every backend (plus the interpreter, plus a future differential-testing pass)
consumes the same IR. The AST can stay expressive and close to the grammar; the IR
can stay uniform and close to what a backend actually needs.

## Three-address code

`calc-ir`'s IR is **three-address code**: every instruction has at most one
destination and reads only already-computed values as operands — no nested
sub-expressions like the AST's `BinOp(Box<Expr>, BinOp, Box<Expr>)`.

```rust
pub struct Temp(pub u32);

pub enum Instr {
    Const { dst: Temp, value: f64 },
    BinOp { dst: Temp, op: BinOp, lhs: Temp, rhs: Temp },
    Copy { dst: Temp, src: Temp },
    If { dst: Temp, cond: Temp, then_block: Block, else_block: Block },
}
```

A `Temp` is a **temporary** — three-address-code terminology for a
compiler-introduced value with no source-level name, as opposed to a variable a DSL
author actually wrote. Lowering `{ let x = 5; x + 1 }` doesn't produce one
instruction per sub-expression the way the AST nests them; it produces a flat
sequence, each instruction naming the temp it computes:

```
t0 = const 5.0    // let x = 5 — x's binding lives in t0
t1 = const 1.0    // the literal 1
t2 = t0 + t1      // x + 1, reading x back as t0
```

This "flatten nested expressions into a sequence of named steps" shape is what
"three-address code" means, and it's the same shape almost every real compiler's
mid-level IR takes (LLVM IR, Cranelift IR, and rustc's own MIR are all variations on
it) — it's simple enough that a lowering pass and a codegen pass can each be written
without having to think about the other's concerns.

### Locals are just temps

`calc-lang`'s `let x = 1;` doesn't lower to a separate "store into local `x`"
instruction. Since A3's `resolve` pass already guarantees `x` is bound before any
use, `ast_to_ir::lower` just remembers, in a scope stack shaped exactly like
`resolve.rs`'s (`Vec<HashMap<String, Temp>>`), *which temp* computed `x`'s value —
and every later `Var("x")` reuses that same temp directly. There's no load/store
instruction pair to write or optimize away later; the AST's own binding structure
already did that job.

## Structured `If`, not jump-based basic blocks

Many compiler IRs represent control flow as a flat list of **basic blocks** —
straight-line instruction sequences — connected by explicit branch/jump
instructions, forming a control-flow graph (CFG) the backend then has to interpret.
`calc-ir` doesn't do that (yet — spec.md §8.1 calls this "native control-flow
nodes... vs. raw basic-block jumps"). Instead, `Instr::If` is one instruction that
directly nests two `Block`s:

```rust
pub struct Block(pub Vec<Instr>);   // straight-line, no internal branches

Instr::If {
    dst: Temp,
    cond: Temp,
    then_block: Block,
    else_block: Block,
}
```

A `Block` here is deliberately simple: just a `Vec<Instr>` with no branches inside
it — the "basic block" part of "structured control flow, not raw basic-block
jumps." What makes it *structured* is that instead of the caller having to follow
jump targets through a flat block list, the two branches are nested directly inside
the one `Instr::If` that owns them, and the pass that walks this IR (a backend, an
interpreter) can recurse into `then_block`/`else_block` the same way it would
recurse into any other nested data.

## Merging branches: "phi via copies"

Here's the part that's specific to `calc-lang`: `if`/`else` is an **expression** —
`{ let x = if cond { 1 } else { 2 }; x }` needs *some* single temp holding "whichever
branch ran actually computed." A textbook SSA-form IR (LLVM's, for instance) solves
this with a **phi node**: a special instruction at the point where two control-flow
paths merge, saying "this value is `a` if we arrived from block A, `b` if from block
B." `calc-ir` doesn't have a `Phi` instruction — with a structured `If` node instead
of a flat CFG, there's no separate "merge point" block for a phi to live in.

Instead, `ast_to_ir::lower` gives `Instr::If` its own `dst` temp, and makes each
branch end with an explicit copy into it:

```rust
let dst = fresh(next_temp);

let mut then_instrs = Vec::new();
let then_result = lower_expr(then_branch, env, &mut then_instrs, next_temp);
then_instrs.push(Instr::Copy { dst, src: then_result });   // then branch → dst

let mut else_instrs = Vec::new();
let else_result = lower_expr(else_branch, env, &mut else_instrs, next_temp);
else_instrs.push(Instr::Copy { dst, src: else_result });   // else branch → dst
```

This is a standard, simple technique sometimes called "phi elimination via copies":
whichever branch actually executes is the only one whose `Copy` actually runs, so
`dst` ends up holding the right value either way, without needing a `Phi` node that
would only ever show up in exactly one place in this IR (a structured `If`'s merge
point). It's also the one spot where this IR isn't strictly SSA — `dst` has two
different instructions that could define it, just never both in the same run —
which is exactly what spec.md §8.1 means by "**roughly** SSA-ish."

## Worked example

Lowering `{ let x = 1; if x { x + 1 } else { 2 } }` (the test in `ast_to_ir.rs`)
produces:

```
t0 = const 1.0                    // let x = 1
if t0:                             // cond = x (t0); t1 is allocated here as the
                                    // If's dst, before either branch is lowered —
                                    // that's why it's numbered ahead of t2/t3/t4
                                    // despite only being written by the two copies
                                    // below
  then:
    t2 = const 1.0
    t3 = t0 + t2                  // x + 1
    t1 = t3                       // copy into the If's dst
  else:
    t4 = const 2.0
    t1 = t4                       // copy into the If's dst
result: t1
```

Every step traces directly back to a piece of the source: `t0` is `x`'s binding,
`t1` is the `if`-expression's shared result temp, and the two branches' `Copy`
instructions are the "phi via copies" merge described above.

## What's deliberately not here yet

spec.md §8.1 lists `Loop`, `Break`/`Continue`, and `Return` alongside `If` as the
IR's eventual control-flow vocabulary. `calc-ir::ir::Instr` only has `If`, because
`calc-lang`'s AST has no loop or early-return construct to lower from — adding empty
IR variants nothing constructs would be scaffolding for a future session, not this
one. See `DECISIONS.md`'s A4 entry for the same reasoning spelled out alongside A2's
precedent (deferring `Stmt` until A3 actually needed it).
