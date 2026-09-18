# A5 — Tree-walking interpreter (debug backend)

**Session code**: [`crates/calc-ir/src/interp.rs`](../crates/calc-ir/src/interp.rs),
[`crates/calc-compiler/src/main.rs`](../crates/calc-compiler/src/main.rs).
**Spec refs**: spec.md §9.1 (debug interpreter). **Prereqs**:
[A4](a4-mid-level-ir-and-lowering.md).

By A4, `calc-lang` source can be turned into IR — but nothing runs it yet. A5 adds
the simplest possible thing that can: a **tree-walking interpreter**, plus enough of
a `calcc` CLI to drive the whole pipeline from a `.calc` file on disk to a printed
number.

## Why build an interpreter before a real codegen backend?

`calc-lang` will eventually have two "real" codegen backends (Cranelift in A6, LLVM
in A7) that turn IR into machine code. Those are complex enough — register
allocation, calling conventions, object-file emission — that bugs in them are easy to
introduce and hard to spot by inspection.

An interpreter sidesteps all of that: it just walks the IR's instructions directly in
a loop, using ordinary Rust values instead of generating any code. That simplicity is
exactly what makes it useful as a **semantics oracle** — a trusted reference for
"what should this program actually compute" that later sessions (concretely, C4's
differential testing) can run every codegen backend's output against and diff. If the
interpreter and a backend disagree, the interpreter is the one you trust, because
there's so much less that could have gone wrong in it.

It's also just the fastest way to run a `calc-lang` program today: no object file, no
linker, no `--backend` flag — `calcc run --interpret program.calc` parses, resolves,
lowers, and evaluates in one process.

## Walking three-address code, not a tree

"Tree-walking interpreter" is a slight misnomer for what's actually being walked
here. A tree-walking interpreter in the classic sense recurses directly over an AST's
nested expression nodes. `calc-ir::interp` instead walks A4's IR, which is **flat**
three-address code (a `Block(Vec<Instr>)`), not a tree — so most of `interpret` is an
ordinary `for` loop over instructions, running each one in order:

```rust
fn exec_block(block: &Block, store: &mut TempStore) {
    for instr in &block.0 {
        exec_instr(instr, store);
    }
}
```

The recursion shows up in exactly one place: `Instr::If` nests two `Block`s (A4's
"structured control flow" design), so evaluating an `If` means recursing into
`exec_block` on whichever branch's `Block` gets selected. That's the one spot this
interpreter is genuinely tree-shaped — everywhere else, it's a straight-line
instruction loop.

## The runtime environment is `Temp -> Value`, not `name -> Value`

A classic textbook interpreter keeps a `HashMap<String, Value>` mapping source
variable names to their current values. `calc_ir::interp` has no such map, and
doesn't need one — by A4, variable *names* no longer exist anywhere in the IR at all.
Two earlier passes already did that work:

- **A3's `resolve()`** walks the *AST* and checks every name makes sense (no
  unresolved identifiers, no duplicate bindings within a scope) — before any IR
  exists.
- **A4's `ast_to_ir::lower()`** rewrites every `Expr::Var(name)` into a direct
  `Temp` reference, using its own scope stack (`Vec<HashMap<String, Temp>>`) that's
  thrown away once lowering finishes. That map never reaches the IR — `Instr` has no
  name-bearing variant at all.

So by the time `interp::interpret` runs, "look up a variable's current value" and
"look up a `Temp`'s current value" are the same operation, and every `Temp` is just a
small dense integer. That makes a growable array the natural store, not a hash map:

```rust
struct TempStore(Vec<Option<Value>>);
```

`Temp` itself is a **tuple struct** — `pub struct Temp(pub u32)` — a struct whose
single field has no name, only a position, so it's accessed as `.0` instead of
`.some_field` (the same way you'd index a plain tuple like `(1, 2).0`). `TempStore`
uses that to go straight from a `Temp` to an array slot: `temp.0 as usize` unwraps
the `Temp` back to its underlying `u32` and casts it to `usize` (the type `Vec`
indexing requires), so `Temp(3)` always means "slot 3." A `Temp` is otherwise just a
type-safe label — wrapping the number in its own type means the compiler stops you
from accidentally passing a `BinOp`'s `f64` or some other `u32` where a `Temp` is
expected, even though under the hood it's nothing more than an integer.

This array-slot design is safe, not just convenient, because of how A4 assigns `Temp` numbers:
`ast_to_ir::lower`'s `next_temp` counter is a single `&mut u32` threaded through the
*entire* recursive lowering, including into both the `then` and `else` arms of every
`If`. That means every `Temp` in a whole program is globally unique and densely
numbered from 0 — no two instructions, anywhere, ever share a slot — and (per A4's
"roughly SSA-ish" design) every `Temp` is written before it's ever read as an
operand. `TempStore::read` panics if that invariant is ever violated (a `Temp` read
before it was written), the same way `ast_to_ir`'s own `lookup()` already panics on
an unresolved identifier — both are "this should be impossible if earlier passes did
their job" checks, not user-facing error paths.

## Runtime values

```rust
pub enum Value {
    Number(f64),
}
```

`calc-lang` has exactly one runtime type today, so `Value` has exactly one variant.
It's an enum rather than a bare `f64` on purpose: every instruction in `exec_instr`
produces and consumes a `Value`, so this is the real shape of "what an instruction
computes," not speculative room for a type `calc-lang` doesn't have yet (contrast
with `Loop`/`Return` IR variants, which A4's docs explain were *not* added because
nothing constructs them — this enum, by contrast, is exercised by every single test).

## Truthiness: there's no boolean type

`calc-lang`'s grammar makes `if <cond> { ... } else { ... }` accept *any* numeric
expression as `<cond>` — there's no separate boolean type or comparison operator
anywhere in the AST. So the interpreter has to define what counts as "true" for a
number, and picks the simplest, most familiar option: nonzero is true, zero is
false, mirroring C's convention.

```rust
if store.read(*cond).as_number() != 0.0 {
    exec_block(then_block, store);
} else {
    exec_block(else_block, store);
}
```

## Worked example

Interpreting `{ let x = 3; if x { x * 2 + 1 } else { 0 } }`:

1. `t0 = const 3.0` — `let x = 3`.
2. `If` reads `t0` (3.0, nonzero → truthy) and takes the `then` branch.
3. Inside `then`: `t2 = const 2.0`, `t3 = t0 * t2` (6.0), `t4 = const 1.0`,
   `t5 = t3 + t4` (7.0), then a `Copy` writes 7.0 into the `If`'s shared `dst` temp.
4. The `else` branch's instructions never run — its temps stay unwritten in the
   store, which is fine, since nothing downstream ever reads them.
5. The program's result temp holds `7.0`.

Running this through `calcc run --interpret` end-to-end (parse → resolve → lower →
interpret) prints `7`.

## The CLI so far

`calcc run --interpret <path>` is hand-parsed from `env::args()` — no `clap` yet.
A13 is the session that gives `calcc` a real CLI-argument crate and a full
`build`/`run`/`check` subcommand surface; pulling that dependency in now, for one
subcommand, would be scaffolding ahead of the session actually scoped to justify it.
For now, `main.rs` just chains the four passes built so far and reports the first
failure it hits, at whichever stage it happens:

```
parse (LalrpopFrontend) → resolve → lower (ast_to_ir) → interpret
```

## What's deliberately not here yet

No bytecode compilation step (spec.md §9.1 allows "tree-walking (or simple
bytecode)" — this session picks the simpler of the two), no `build`/`check`
subcommands (A13), and no `--backend` selection (there's only one execution path so
far). Those all layer on in later sessions without changing anything built here —
the interpreter, once written, stays exactly what A6/A7's codegen backends and C4's
differential tests get checked against.
