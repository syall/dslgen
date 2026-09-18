# A6 — Cranelift codegen backend

**Session code**:
[`crates/calc-compiler/src/cranelift_backend.rs`](../crates/calc-compiler/src/cranelift_backend.rs),
[`crates/calc-compiler/src/link_stub.rs`](../crates/calc-compiler/src/link_stub.rs),
[`crates/calc-compiler/src/main.rs`](../crates/calc-compiler/src/main.rs).
**Spec refs**: spec.md §8.1 (pluggable codegen backends). **Prereqs**:
[A4](a4-mid-level-ir-and-lowering.md), [A5](a5-tree-walking-interpreter.md).

A5 gave `calc-lang` a way to *run* a program without generating any machine code.
A6 adds the other kind of backend spec.md §8.1 calls for: one that actually
compiles the IR — `calcc build --backend=cranelift program.calc -o program` now
produces a real, standalone, runnable executable.

## What a codegen backend does that an interpreter doesn't

A5's interpreter reads `calc-ir::Instr`s and immediately *does* what they say —
add two numbers, take a branch — using ordinary Rust control flow. A codegen
backend instead has to produce a self-contained artifact (here, an object file)
that some *other* process — not this one — can later execute on its own, with no
`calc-ir` data structures or Rust runtime anywhere in sight. That's the essential
difference: an interpreter's job finishes when the program's answer is known; a
compiler's job finishes when a file exists that, months from now, on a different
machine, still computes that answer.

[Cranelift](https://cranelift.dev/) is the library doing the actual machine-code
generation here. It's a compiler backend in its own right (also used by Wasmtime
and as an alternate `rustc` codegen backend) — it takes its *own* IR (blocks,
typed values, instructions) and turns it into real x86-64/ARM64/etc. instructions.
`cranelift_backend.rs`'s whole job is translating `calc-ir`'s IR into Cranelift's,
instruction by instruction, then handing the result to `cranelift-object` to
write out as a native object file.

## Cranelift IR basics: blocks, values, and the builder

Cranelift structures a function as a graph of **blocks** — each one a
straight-line instruction sequence ending in a branch or return, exactly like
`calc-ir::Block`, except Cranelift's blocks are the *only* control-flow
mechanism (no nested structured `If`; a branch just names which block runs next).

That's a real structural difference, not just a naming one — the same program
looks like a **tree** in `calc-ir` and a **graph** in Cranelift's IR:

![calc-ir's nested-Block tree next to Cranelift's flat block graph, for the same `if x { x + 1 } else { 2 }` program, with matching colors marking the same value or step across both IRs](images/a6-ir-comparison.svg)

Matching colors (not lines — the two IRs' shapes are different enough that a
connecting line would have to cut across unrelated text to get from one side
to the other) mark the same thing named on both sides: `t0` is `v0`; the
condition check + branch is `fcmp`/`brif`; each branch's `Copy t1 = ...` is a
`def_var(t1, ...)` inside the matching block (`then_block`/`block1` in one
color, `else_block`/`block2` in another, since there are two of each); and the
`If`'s overall `result: t1` is the `return v6` that reads it back out.

On the left, `calc-ir`'s `Instr::If` is one instruction that *contains* two
whole nested `Block`s — the tree is exactly two levels deep, at exactly the one
place `calc-lang` has control flow. On the right, Cranelift has no containment
at all: `block0`/`block1`/`block2`/`block3` are four *siblings*, linked only by
the `brif`/`jump` edges between them. `block3` ending up with two predecessors
— reachable from either `block1` or `block2` — is exactly the situation that
needs a `phi`, and it's also something `calc-ir`'s tree shape structurally
cannot produce (there's only ever one merge point, dictated by the nesting).
"The trick that avoids hand-writing `phi` nodes," further down, covers how this
backend gets a correct answer out of `block3` without ever writing one by hand.

Every instruction that produces a result returns a Cranelift `Value` — Cranelift's
own SSA value, unrelated to `calc_ir::interp::Value` — and instructions are built
up through `FunctionBuilder`'s `.ins()` method:

```rust
let dst = builder.ins().fadd(lhs, rhs);
```

This mirrors `calc-ir::Instr::BinOp` closely enough that most of the lowering is
direct translation:

```rust
let result = match op {
    calc_ir::BinOp::Add => builder.ins().fadd(lhs, rhs),
    calc_ir::BinOp::Sub => builder.ins().fsub(lhs, rhs),
    calc_ir::BinOp::Mul => builder.ins().fmul(lhs, rhs),
    calc_ir::BinOp::Div => builder.ins().fdiv(lhs, rhs),
};
```

`calc-ir::Instr::Const` becomes `builder.ins().f64const(value)` the same way —
`calc-lang` has exactly one runtime type (`f64`), so every Cranelift value in this
backend is typed `types::F64`.

### Crash course: every Cranelift call in this file

Rather than introduce these one at a time as they come up, here's every Cranelift
call `cranelift_backend.rs` actually makes, grouped by what it's for and mapped to
what it means for *this* program specifically — a reference to come back to while
reading the rest of this page.

> **Picking a target and starting a module**
>
> - **`cranelift_native::builder()`** — "give me a builder configured for whatever
>   CPU is running this code right now." No hardcoded target triple: `calcc`
>   compiles for its own machine.
> - **`settings::builder()` / `.set("is_pic", "false")`** (the `Configurable`
>   trait) — a mutable bag of codegen flags. Used once, to explicitly turn off
>   position-independent-code generation rather than silently inherit whatever
>   the default happens to be.
> - **`settings::Flags::new(flag_builder)`** — freezes that bag of flags into an
>   immutable set, ready to hand to the ISA builder.
> - **`isa_builder.finish(flags)`** — combines "which CPU" with "which flags" into
>   one concrete `TargetIsa`: the object that actually knows how to turn Cranelift
>   IR into real machine instructions for this target.
> - **`ObjectBuilder::new(isa, name, default_libcall_names())`** — configures "emit
>   a native object file for this ISA." `default_libcall_names()` supplies the
>   standard symbol names Cranelift would use for any runtime helper calls it
>   might need to insert on its own (not used by this backend's simple f64 ops,
>   but required to construct the builder regardless).
> - **`ObjectModule::new(object_builder)`** — the container everything gets built
>   into. Both `calc_main` and `main` are declared and defined into this one
>   `module`, and it's what eventually becomes the returned object-file bytes.
> - **`module.isa()`** — "what target was this module built for?" — used to fetch
>   a correct default calling convention for every function signature, instead of
>   hardcoding one that would silently be wrong on some other platform (an actual
>   bug caught while building this session — see `DECISIONS.md`).
> - **`module.finish()` then `product.object.write()`** — `finish()` closes the
>   module out into an `ObjectProduct`; `.object.write()` (from the underlying
>   `object` crate) serializes that into real COFF/ELF/Mach-O bytes — the
>   `Vec<u8>` `compile_to_object` hands back.
>
> **Declaring a function's shape, and giving it a body**
>
> - **`Signature::new(call_conv)`** — the start of a function's "shape": which
>   calling convention it uses, with no parameters or return values yet.
> - **`AbiParam::new(ty)`** — describes one value crossing the ABI boundary; every
>   signature here has exactly one — `calc_main`'s `f64` return, or `main`'s `i32`
>   return.
> - **`module.declare_function(name, linkage, sig)`** — registers a named,
>   exported symbol slot and hands back a `FuncId` — this is what makes `calc_main`
>   and `main` real, linkable symbols in the eventual object file.
> - **`Context::new()`** — a fresh per-function scratch space holding the actual
>   `Function` (`ctx.func`) that gets built up.
> - **`FunctionBuilderContext::new()`** — a separate, reusable chunk of state the
>   `FunctionBuilder` needs for its own SSA-construction bookkeeping, kept apart
>   from `Context` precisely so it *can* be reused across functions.
> - **`FunctionBuilder::new(&mut ctx.func, &mut builder_ctx)`** — the tool that
>   actually builds a function body, instruction by instruction. Nearly every call
>   below is a method on this `builder`.
> - **`module.define_function(func_id, &mut ctx)`** — takes the finished,
>   built `Context` and attaches it to the `FuncId` declared earlier. This is the
>   step that gives a previously-empty symbol a real body.
> - **`module.declare_func_in_func(calc_main_id, builder.func)`** — imports
>   *another* function's `FuncId` (here, `calc_main`'s) into the function currently
>   being built (`main`), producing a local `FuncRef` that `.ins().call()` can
>   target. This is specifically how `main` gets permission to call `calc_main`.
>
> **Blocks: switching, jumping, and sealing**
>
> - **`builder.create_block()`** — allocates a new, empty basic block with nothing
>   in it yet. Called once per function for the entry block, and three times per
>   `If` (`then`, `else`, `merge`).
> - **`builder.switch_to_block(block)`** — moves the builder's "current insertion
>   point." Every `.ins()` call after this appends to whichever block was last
>   switched to — so lowering `If` calls `switch_to_block(then_blk)`, emits the
>   `then` branch's instructions, then `switch_to_block(else_blk)` and does the
>   same for `else`, then `switch_to_block(merge_blk)` for whatever comes after
>   the `If`. It's purely a "where do new instructions go" cursor — it doesn't by
>   itself create any control flow.
> - **`.ins().jump(target_block, [])`** — the instruction that actually *creates*
>   control flow: "unconditionally continue execution at `target_block`." Both
>   `then_blk` and `else_blk` end with a `jump(merge_blk, [])` — this is the literal
>   machine-level equivalent of the two branches of an `if`/`else` "meeting back
>   up" afterward.
> - **`.ins().brif(cond, then_blk, [], else_blk, [])`** — the two-way version of
>   `jump`: "go to `then_blk` if `cond` is true, otherwise go to `else_blk`." This
>   is the actual branch `Instr::If`'s condition compiles down to.
> - **`builder.seal_block(block)`** — a promise: "every branch that will ever
>   target this block has now been emitted; no more will show up later." This
>   matters because of how `use_var` (below) works — Cranelift can only resolve
>   what value a `Variable` holds at the *start* of a block once it knows every
>   block that might jump into it, so it can check what each of them last wrote.
>   `then_blk`/`else_blk` can be sealed immediately after creation (each has
>   exactly one predecessor: the `brif` above, already emitted); `merge_blk` can
>   only be sealed *after* both branches have emitted their `jump` into it, since
>   only then are all of its predecessors actually known. Sealing too early, before
>   a predecessor exists, is exactly the bug this ordering avoids.
>
> **Variables: the phi-avoidance trick**
>
> - **`builder.declare_var(ty)`** — mints one new `Variable` of a given type.
>   Called once per `calc_ir::Temp` the program ever uses (see `collect_temps`) —
>   every IR temporary gets its own Cranelift `Variable`.
> - **`builder.def_var(var, value)`** — "as of this point in this block, `var`
>   holds `value`." Maps to every IR instruction's *write*: `Const`, `BinOp`, and
>   `Copy` all end by `def_var`-ing their `dst` temp.
> - **`builder.use_var(var)`** — "give me `var`'s current value here." Maps to
>   every IR instruction's *read*: `BinOp`'s `lhs`/`rhs`, `If`'s `cond`, `Copy`'s
>   `src`. This is the one call doing the real phi-avoidance work: read from a
>   block with two sealed predecessors that each wrote something different, and
>   Cranelift's own SSA-construction algorithm resolves the correct value (a real
>   `phi`, if one is actually needed) without this backend's code ever asking for
>   one explicitly. The next section walks through exactly why that's the piece
>   that makes hand-written `phi` nodes unnecessary.
>
> **`ins()` and the instructions it inserts**
>
> - **`builder.ins()`** — not an instruction itself, and doesn't return a value on
>   its own — it's a handle onto "insert the next instruction at the builder's
>   current position" (wherever the last `switch_to_block` pointed). Every
>   instruction-emitting call in this file is a method chained directly off it,
>   e.g. `builder.ins().fadd(a, b)`.
> - **`.ins().f64const(value)`** — emits a floating-point constant. Maps to
>   `Instr::Const`.
> - **`.ins().fadd/.fsub/.fmul/.fdiv(lhs, rhs)`** — floating-point arithmetic.
>   Maps to `Instr::BinOp`, one call per `calc_ir::BinOp` variant.
> - **`.ins().fcmp(FloatCC::NotEqual, cond, zero)`** — a floating-point comparison
>   producing a boolean value. Maps to `Instr::If`'s "is the condition truthy"
>   check (nonzero-is-true, matching the interpreter exactly — see A5's docs).
> - **`.ins().call(func_ref, args)`** — calls another function. The only call in
>   this whole backend: `main` calling `calc_main`.
> - **`builder.inst_results(call)[0]`** — a call is itself just one instruction, so
>   its return value isn't handed back directly the way `fadd` etc.'s is — this
>   pulls the actual result value back out of the `call` instruction just emitted.
> - **`.ins().fcvt_to_sint_sat(types::I32, value)`** — a *saturating*
>   floating-point-to-integer conversion. Maps to turning `calc_main`'s `f64`
>   answer into the `i32` exit code `main` returns, without undefined behavior on
>   an out-of-range value.
> - **`.ins().return_(values)`** — ends the function, handing back the given
>   values. Maps to `calc_main`'s final result, or `main`'s exit code.
> - **`builder.finalize(module.isa().frontend_config())`** — the last call on a
>   given `builder`: finishes SSA construction for good, checks the function is
>   well-formed, and releases the borrow on `ctx.func` so `ctx` can be handed to
>   `module.define_function`.

## The trick that avoids hand-writing `phi` nodes

`Instr::If` is the one place lowering isn't a straight instruction-for-instruction
translation, because of what A4's own docs call "phi via copies": both of an
`If`'s branches write the *same* destination `Temp`, which is exactly the
situation that needs an SSA **`phi` node** — "this value is `X` if control came
from block A, or `Y` if it came from block B" — in a real SSA-based IR like
Cranelift's.

Rather than construct that `phi` by hand (create a `merge` block with an explicit
block parameter, pass the right value as a branch argument from each
predecessor), this backend uses Cranelift's `Variable` abstraction instead. A
`Variable` looks and acts like an ordinary mutable local — you `declare_var` it
once, then `def_var`/`use_var` it as many times as you like, from any block:

```rust
for temp in collect_temps(program) {
    vars.entry(temp).or_insert_with(|| builder.declare_var(types::F64));
}
```

(`collect_temps` can list the same `Temp` more than once — an `If`'s `dst` is
collected once for the `If` itself and again via each branch's `Copy` sharing
that `dst` — so `entry().or_insert_with(..)` is what keeps this a true
one-`Variable`-per-`Temp` mapping instead of quietly minting and orphaning an
extra one on every repeat.)

Under the hood, `cranelift-frontend` runs an on-the-fly SSA-construction
algorithm (from Braun, Buchwald, et al.'s paper on simple and efficient SSA
construction) that inserts whatever `phi`s are actually needed the moment a
variable is read in a block with multiple predecessors — but only once every
predecessor block has been **sealed** (a promise that no more predecessors will
ever be added). So lowering `Instr::If` looks like this:

```rust
let then_blk = builder.create_block();
let else_blk = builder.create_block();
let merge_blk = builder.create_block();

builder.ins().brif(is_truthy, then_blk, &[], else_blk, &[]);

builder.switch_to_block(then_blk);
builder.seal_block(then_blk);       // one predecessor: the brif above
lower_block(then_block, builder, vars);
builder.ins().jump(merge_blk, &[]);

builder.switch_to_block(else_blk);
builder.seal_block(else_blk);       // one predecessor: the brif above
lower_block(else_block, builder, vars);
builder.ins().jump(merge_blk, &[]);

builder.switch_to_block(merge_blk);
builder.seal_block(merge_blk);      // now both predecessors are known
```

Each branch's own `Instr::Copy { dst, src }` (the "phi via copies" instruction
A4's lowering pass already emits) becomes an ordinary `def_var(vars[dst], ...)`.
By the time `merge_blk` is sealed, both possible writes to that `Temp`'s
`Variable` are known, and the next `use_var` of it — reading the `If`'s overall
result — gets whichever one actually ran, with no `Phi` instruction ever written
by this backend's own code. This is a direct, one-level-down echo of A4's own
design: the mid-level IR sidesteps hand-written `phi`s with `Copy`, and Cranelift
sidesteps them again with `Variable` — see `DECISIONS.md`'s A6 entry.

`FloatCC::NotEqual` (used to test the condition against `0.0`) is worth calling
out: IEEE 754 float comparisons distinguish "not equal" from "equal or
unordered," and Cranelift's `NotEqual` is specifically the *unordered-or-not-equal*
case — true for `NaN != 0.0`, exactly matching Rust's own `f64 != 0.0`. That's
what makes this bit-for-bit the same truthiness rule as A5's interpreter
(`as_number() != 0.0`), including on inputs the language can't currently even
produce a `NaN` from.

## From a function to an object file

`compile_to_object` builds an `ObjectModule` (from `cranelift-object`) targeting
whatever CPU this backend is running on (`cranelift_native::builder()` — no
hardcoded target triple), and defines two functions in it:

- **`calc_main() -> f64`** — the actual compiled program, built by
  `define_calc_main` exactly as described above.
- **`main() -> i32`** — a small C-ABI wrapper that calls `calc_main` and converts
  its result to an exit code via `fcvt_to_sint_sat` (a *saturating* float→int
  conversion — always well-defined, even for a result outside `i32`'s range,
  unlike a raw truncating cast).

Why a wrapper at all? `calc-lang` has no I/O and no built-in functions yet (those
land in A9–A11) — so there is no way for a linked, standalone executable to
report its answer except through some channel the operating system itself
understands. A process's exit code is the simplest one available, and it's
exactly what this session's tests check.

## Linking: a stub, on purpose

An object file isn't runnable on its own — turning it into an executable needs a
linker, which is properly session A12's job ("The link driver"). `link_stub.rs`
is a small, explicitly temporary stand-in that reuses the widely-used `cc` crate
(the same one most `build.rs` scripts and `rustc` itself use) purely to *find*
whichever system C toolchain is available — MSVC's `cl.exe` on Windows, or
`cc`/`gcc`/`clang` on Unix — and invoke it as a linker against the one object
file this backend produced.

One platform wrinkle showed up here worth understanding, not just working around:
a hand-built object file carries none of the linker directives (`/DEFAULTLIB` on
MSVC) that a real `cl.exe`-compiled object would, so the C runtime startup
routine `main` needs (`mainCRTStartup`, which calls `main` and then exits the
process) doesn't get linked in automatically. `link_stub.rs` names the required
libraries explicitly on Windows (`libcmt.lib`, `libvcruntime.lib`,
`libucrt.lib`); Unix-style `cc`/`gcc` need no equivalent flag, since they already
assume any object exposing `main` wants the standard C startup sequence.

## The interpreter as this backend's oracle

spec.md §9.1 frames the interpreter as a "semantics reference" other backends get
checked against — this session is the first time that actually happens. Every
test in `cranelift_backend.rs` compiles a program through the real
`compile_to_object` → `link_stub::link` pipeline, runs the resulting executable,
and compares its exit code against what `calc_ir::interpret` computes for the
*same source*:

```rust
fn assert_matches_interpreter(src: &str, out_name: &str) {
    // ...
    let calc_ir::Value::Number(expected) = calc_ir::interpret(&calc_ir::lower(&ast));
    assert_eq!(compile_and_run(src, out_name), expected as i32);
}
```

This is differential testing in miniature — the same idea C4 formalizes later
across every sample program and every backend, exercised here on a small,
hand-picked set (straight-line arithmetic, and both branches of an `if`/`else`)
because a second real backend to compare against doesn't exist yet.

## What's deliberately not here yet

No `Backend` trait (A8 — there's only one codegen backend so far, nothing to
unify it with), no LLVM backend to compare against (A7), no real link driver
(A12 — FFI/IPC runtime libraries aren't linked in, because those built-in kinds
don't exist yet either), and no `--backend=` dispatch across more than one
backend (also A8/A13). Control flow beyond `if`/`else` was allowed to "lag" per
roadmap.md's own framing, but turned out to map naturally onto Cranelift's block
model, so it's included rather than deferred.
