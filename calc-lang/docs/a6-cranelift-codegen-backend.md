# A6 — Cranelift codegen backend

**Session code**:
[`crates/calc-compiler/src/cranelift_backend.rs`](../crates/calc-compiler/src/cranelift_backend.rs),
[`crates/calc-compiler/tests/cranelift_recipes.rs`](../crates/calc-compiler/tests/cranelift_recipes.rs),
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

## Building real units: recipes

Cranelift's calls only make sense in a fixed order, and every larger unit (a
block, an `if`, a function, a whole object file) is that order repeated. So this
section teaches by building: four recipes, each a complete unit, then the few
remaining calls none of them use. The first three are built as tests in
`tests/cranelift_recipes.rs` (with `unwrap` where the snippets show `?`), so they
can't drift from working code. The IR shown is what Cranelift prints for each one
(`ctx.func.display()`); `windows_fastcall` is this machine's calling convention
and will read differently on yours.

**Two fixed orders cover everything.**

*Building a block* is always this sequence:

1. `create_block()` — allocates an empty block. Do it any time before you need to
   name the block (a `brif` or `jump` must name its targets before they're filled).
2. `switch_to_block(b)` — moves the builder's insertion cursor to `b`. It only
   says *where new instructions go*; it creates no control flow by itself.
3. `.ins().something(..)` — emit instructions. `ins()` isn't an instruction and
   returns no value: it's a handle meaning "insert at the cursor", and every
   instruction-emitting call (`fadd`, `brif`, `return_`, ...) hangs off it.
4. **One terminator**: `jump`, `brif` or `return_`. `jump` is what actually
   *creates* control flow ("continue at that block"); `brif` is its two-way
   version. Nothing may follow a terminator in the same block.
5. `seal_block(b)` — a promise that every jump *into* `b` has now been emitted.
   Cranelift can only work out what value a `Variable` holds at the start of a
   block once it knows every block that might jump in, so a block with a single
   predecessor can be sealed right away, while a merge point can only be sealed
   after all its incoming jumps exist.

*Building a function* is always this sequence:

1. a module: `new_object_module()` (Recipe 4 opens it up)
2. a `Signature`: the calling convention, then `params`/`returns` of `AbiParam`s
3. `module.declare_function(name, linkage, &sig)` → a `FuncId`. *Declaring* only
   reserves the name and shape, so other functions can call it before it has a body.
4. a `Context` (holds the `Function` being built; put the signature in
   `ctx.func.signature`), a `FunctionBuilderContext` (the builder's own scratch
   space, separate so it can be reused across functions), and
   `FunctionBuilder::new(&mut ctx.func, &mut ..)`
5. the entry block: `create_block`, `append_block_params_for_function_params`,
   `switch_to_block`, `seal_block` (nothing jumps into the entry, so seal at once)
6. the body: more blocks, each following the block sequence above
7. `finalize(..)` — ends the builder: finishes SSA construction and releases the
   borrow on `ctx.func`. It takes the target's `frontend_config()`.
8. `module.define_function(func_id, &mut ctx)` — attaches the finished body to the
   declared `FuncId` and runs Cranelift's verifier

**Recipe 1: a function that takes a parameter.** Parameters arrive as the entry
block's parameters, read with `block_params`, not through a call:

```rust
let mut sig = Signature::new(module.isa().default_call_conv());
sig.params.push(AbiParam::new(types::F64));
sig.returns.push(AbiParam::new(types::F64));
let id = module.declare_function("twice", Linkage::Export, &sig)?;

let mut ctx = Context::new();
ctx.func.signature = sig;
let mut fb_ctx = FunctionBuilderContext::new();
let mut b = FunctionBuilder::new(&mut ctx.func, &mut fb_ctx);

let entry = b.create_block();
b.append_block_params_for_function_params(entry); // x becomes block0's parameter
b.switch_to_block(entry);
b.seal_block(entry);

let x = b.block_params(entry)[0];
let doubled = b.ins().fadd(x, x);
b.ins().return_(&[doubled]);                      // the terminator

b.finalize(module.isa().frontend_config());
module.define_function(id, &mut ctx)?;
```

```
function u0:0(f64) -> f64 windows_fastcall {
block0(v0: f64):
    v1 = fadd v0, v0
    return v1
}
```

Every Rust line above has exactly one visible effect: the signature is the
header line, the entry block and its parameter are `block0(v0: f64):`, `fadd` and
`return_` are the two instructions. `define_calc_main` is this same recipe with
no parameters and a body produced by `lower_block`.

**Recipe 2: an `if`/`else` that produces a value.** This is `Instr::If` on its
own, with a parameter as the condition so Cranelift can't fold it away. Apart
from the signature (three `f64` parameters) only the body differs from Recipe 1;
steps 1–5 and 7–8 are the same:

```rust
let (c, x, y) = { let p = b.block_params(entry); (p[0], p[1], p[2]) };

let result = b.declare_var(types::F64);           // one Variable for the merged value
let zero = b.ins().f64const(0.0);
let truthy = b.ins().fcmp(FloatCC::NotEqual, c, zero);

let then_blk = b.create_block();                  // create all three up front,
let else_blk = b.create_block();                  // because brif and both jumps
let merge_blk = b.create_block();                 // name them before they're filled
b.ins().brif(truthy, then_blk, &[], else_blk, &[]);

b.switch_to_block(then_blk);                      // one predecessor (the brif): seal now
b.seal_block(then_blk);
b.def_var(result, x);
b.ins().jump(merge_blk, &[]);

b.switch_to_block(else_blk);
b.seal_block(else_blk);
b.def_var(result, y);
b.ins().jump(merge_blk, &[]);

b.switch_to_block(merge_blk);                     // both jumps exist: now seal
b.seal_block(merge_blk);
let r = b.use_var(result);
b.ins().return_(&[r]);
```

```
function u0:0(f64, f64, f64) -> f64 windows_fastcall {
block0(v0: f64, v1: f64, v2: f64):
    v3 = f64const 0.0
    v4 = fcmp ne v0, v3  ; v3 = 0.0
    brif v4, block1, block2

block1:
    jump block3(v1)

block2:
    jump block3(v2)

block3(v5: f64):
    return v5
}
```

Read it against the code: `brif` is the `brif`; each `def_var` + `jump` pair
became `jump block3(vN)`, passing that branch's value; and `use_var` in the merge
block became the parameter `block3(v5: f64)`. **That parameter is the `phi`.**
You never wrote it; sealing `merge_blk` after both jumps existed is what let
Cranelift work out it was needed. Written by hand, it would be
`let merged = b.append_block_param(merge_blk, types::F64);` plus
`jump(merge_blk, &[x])` / `jump(merge_blk, &[y])`, and reading `merged` instead of
calling `use_var`. This backend uses `Variable` so `Instr::Copy` needs no
special handling.

**Recipe 3: one function calling another.** Declare both functions first, then
define them; inside the caller, turn the callee's `FuncId` into a local `FuncRef`
and `call` it:

```rust
let callee_ref = module.declare_func_in_func(callee, b.func);
let call = b.ins().call(callee_ref, &[]);
let result = b.inst_results(call)[0];     // a call's results are looked up separately
b.ins().return_(&[result]);
```

```
function u0:0() -> f64 windows_fastcall {
    sig0 = () -> f64 windows_fastcall
    fn0 = colocated u0:0 sig0

block0:
    v0 = call fn0()
    return v0
}
```

`fn0` is only this function's private name for the callee, which is why it
doesn't equal the callee's own `u0:0`. `define_c_main` is exactly this, plus
`fcvt_to_sint_sat` on the result.

**Recipe 4: from a module to an object file.** Step 1 of the function order,
`new_object_module()`, is where all the target-selection calls live. Opened up:

```rust
fn new_object_module() -> ObjectModule {
    let isa_builder = cranelift_native::builder()?;            // "the CPU running this code"
    let mut flags = settings::builder();                       // a mutable bag of codegen flags
    flags.set("is_pic", "false")?;                             // no position-independent code
    let isa = isa_builder.finish(settings::Flags::new(flags))?; // CPU + frozen flags = a TargetIsa
    let object_builder =
        ObjectBuilder::new(isa, "calc_main", default_libcall_names())?; // "emit an object file"
    ObjectModule::new(object_builder)                          // the container functions go into
}
```

The `TargetIsa` is the object that actually knows how to turn Cranelift IR into
machine instructions for this CPU. `default_libcall_names()` names the runtime
helper functions Cranelift might insert on its own (this backend's `f64` ops never
need one, but the builder requires the argument). No target triple is written
anywhere, so `calcc` compiles for the machine it runs on.

From then on the module answers questions about its target, and you should ask it
rather than assume: `module.isa().default_call_conv()` for every `Signature` (an
early draft here hardcoded the System V convention, which is wrong on Windows) and
`module.isa().frontend_config()` for every `finalize`.

Then the whole of `compile_to_object` is: make the module, *declare, build,
define* each function, then `finish`:

```rust
let mut module = new_object_module();
let calc_main_id = declare_calc_main(&mut module);       // declare, so others can call it
define_calc_main(&mut module, calc_main_id, program);    // build + define (Recipes 1 + 2)
let main_id = declare_c_main(&mut module);
define_c_main(&mut module, main_id, calc_main_id);       // build + define (Recipe 3)
module.finish().object.write()   // finish() closes the module; write() serializes it
                                 // to real COFF/ELF/Mach-O bytes: the .o/.obj file
```

**Other calls this backend makes.** The recipes cover the structure; these are the
remaining individual calls, each used in `cranelift_backend.rs`:

| Call | What it does here |
| --- | --- |
| `b.ins().f64const(v)` | Emits a constant; `calc-ir`'s `Instr::Const`. |
| `b.ins().fadd/fsub/fmul/fdiv(l, r)` | Float arithmetic; one per `calc_ir::BinOp` (see the `match` in "Cranelift IR basics"). |
| `b.ins().fcmp(FloatCC::NotEqual, cond, zero)` | A comparison producing a truth value for `brif` to branch on; how `If`'s "nonzero is true" is spelled. Its NaN behavior is covered in the `phi` section below. |
| `b.ins().fcvt_to_sint_sat(types::I32, v)` | Float to integer with *saturating* conversion (always defined, even out of `i32`'s range). Used by `main` to turn `calc_main`'s `f64` answer into an exit code. |
| `Linkage::Export` | Makes a declared function's symbol visible to the linker. `main` needs it so the C startup code can find it. |

**What Cranelift enforces.** Each of these mistakes was made deliberately
against this crate version (0.135.2) and the response recorded, so the messages
below are verbatim:

| Mistake | What happens |
| --- | --- |
| Block never ends in a terminator | panic at `finalize`: `FunctionBuilder finalized, but block block0 is not filled` |
| Instruction after the terminator | panic: `you cannot add an instruction to a block already filled` |
| `.ins()` before any `switch_to_block` | panic: `Please call switch_to_block before inserting instructions` |
| `switch_to_block` away from an unfinished block | panic: `you have to fill your block before switching` |
| Forgot `seal_block` | panic at `finalize`: `FunctionBuilder finalized, but block block0 is not sealed` |
| Sealed a block, then jumped into it | panic: `assertion failed: !self.is_sealed(block)` |
| Returned an `i32` from an `f64` function | `define_function` returns `Err`: `Compilation error: Verifier errors` |
| `use_var` on a variable no path ever `def_var`'d | same verifier `Err` from `define_function` |

The first six are the builder catching sequencing errors as you make them; the
last two only surface when `define_function` verifies the finished function, which
is why this backend's `.expect("... well-formed")` on it is the real safety net.

**Extending to something new.** A construct calc-lang doesn't have yet, like
`while`, is the same two orders with one twist, and that twist is *why* sealing
exists: the loop's header block has a predecessor that doesn't exist yet (the
jump back from the end of the body). Sketch:

1. `create_block` for `header`, `body`, `exit`; `jump(header)` from where you are.
2. `switch_to_block(header)`, but **don't seal it yet**. Emit the condition and
   `brif(cond, body, &[], exit, &[])`.
3. `switch_to_block(body)`, `seal_block(body)`, emit the body, then
   `jump(header, &[])`. That back-edge is the last predecessor `header` will get.
4. Now `seal_block(header)`. Then `switch_to_block(exit)` and `seal_block(exit)`.

Any variable the body changes is picked up as a header block parameter by the
same `def_var`/`use_var` mechanism as the `phi` above; nothing extra to write.
(`calc-lang` has no loop construct, so this is a sketch, not code in this repo;
see `DECISIONS.md`'s A4 entry on deferring `Loop`.)

## The trick that avoids hand-writing `phi` nodes

Recipe 2 above built an `if`/`else` by hand from a function's parameters. This
section is the same shape again, but driven by `calc-ir` instead of parameters,
and it explains why the backend leans on `Variable` to do it. `Instr::If` is the
one place lowering isn't a straight instruction-for-instruction translation,
because of what A4's own docs call "phi via copies": both of an
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
ever be added). So lowering `Instr::If` is Recipe 2's sequence, with
`lower_block` filling each branch instead of a single `def_var`:

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
