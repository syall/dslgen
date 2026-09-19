# A7 — LLVM codegen backend

**Session code**:
[`crates/calc-compiler/src/llvm_backend.rs`](../crates/calc-compiler/src/llvm_backend.rs),
[`crates/calc-compiler/src/main.rs`](../crates/calc-compiler/src/main.rs),
[`crates/calc-compiler/Cargo.toml`](../crates/calc-compiler/Cargo.toml).
**Spec refs**: spec.md §8.1 (pluggable codegen backends). **Prereqs**:
[A6](a6-cranelift-codegen-backend.md).

A6 compiled `calc-ir` to a native executable with Cranelift. A7 does the same job with
[LLVM](https://llvm.org/), the industry-standard compiler backend behind Clang, Rust's
own `rustc`, Swift and many others. `calcc build --backend=llvm program.calc -o program`
now produces a runnable executable whose answer matches both the A5 interpreter and the
A6 Cranelift build — the tests check all three agree.

Having two backends consume the *same* IR is the whole point of the session: it is the
only honest way to see what each one does differently.

## Turning the LLVM backend on

LLVM is a huge C++ project, not a Rust crate. Using it means the machine building
`calcc` needs an LLVM install (here 21.x, found through the `LLVM_SYS_211_PREFIX`
environment variable), and linking it adds a lot to build time and binary size. So the
backend is **feature-gated** in `Cargo.toml`:

```toml
inkwell = { version = "0.10", default-features = false,
            features = ["llvm21-1", "target-x86"], optional = true }

[features]
backend-llvm = ["dep:inkwell"]
```

- `optional = true` means Cargo doesn't even download or compile `inkwell` unless
  something asks for it.
- `backend-llvm` is that something: `cargo build --features backend-llvm`.
- `#[cfg(feature = "backend-llvm")]` in `main.rs` then compiles `llvm_backend.rs` (and
  the `--backend=llvm` command-line arm) only when the feature is on. Without it,
  `--backend=llvm` prints "rebuild with `--features backend-llvm`" instead of failing
  mysteriously.
- `llvm21-1` picks which LLVM version to talk to (it must match what's installed), and
  `target-x86` links only LLVM's x86 code generator instead of every architecture.

It's off by default so a plain `cargo test --workspace` — including the GitHub Actions
run, which has no LLVM installed — still works. A8 will make this systematic for both
backends.

## LLVM IR basics: SSA, basic blocks, and `phi`

LLVM's IR looks a lot like Cranelift's (and the `calc-ir` sketch in A4): functions made
of **basic blocks**, each a straight-line list of instructions ending in a branch or
`ret`. It's **SSA** — *static single assignment* — meaning every value (`%x`) is defined
exactly once. That is great for optimizers, but it raises a puzzle: `if c { 10 } else
{ 20 }` produces a value that comes from *one of two places*. Which single definition
is it?

The answer is a **`phi` node** ("φ", pronounced "fee"), placed at the start of the block
where the two paths merge. It says "if I got here from block A use this value, if from
block B use that one". Here is the actual LLVM IR `llvm_backend.rs` builds for
`if 1 { 10 } else { 20 }` (unoptimized, from the module's `print_to_string`):

```llvm
define double @calc_main() {
entry:
  br i1 true, label %then, label %else
then:
  br label %merge
else:
  br label %merge
merge:                                            ; preds = %else, %then
  %if_result = phi double [ 1.000000e+01, %then ], [ 2.000000e+01, %else ]
  ret double %if_result
}
```

Two things worth noticing. First, `br i1 true` — the condition is a compile-time
constant, and the `inkwell` builder folds `fcmp 1.0, 0.0` to `true` *as it's emitting*
it (LLVM's `IRBuilder` constant-folds eagerly). Since calc-lang has no inputs yet,
every condition is a constant; a program with real inputs would keep an `fcmp`. Second,
the `phi`'s pairs name the *predecessor blocks*, not just values.

### The same program in all three IRs

![calc-ir's nested tree, Cranelift's block graph, and LLVM's block graph for the same `{ let x = 1; if x { x + 1 } else { 2 } }` program, with matching colors marking the same value or step across all three](images/a7-ir-comparison.svg)

Matching colors (not lines) mark the same thing across the three columns, as in A6's
diagram. Reading left to right:

- **`calc-ir` → both backends**: the tree flattens into sibling blocks linked by
  branches. `t0 = const 1.0` and the `If`'s condition become a compare and a
  conditional branch (`brif` in Cranelift, `br i1` in LLVM).
- **Cranelift → LLVM, the merge**: Cranelift's merge block reads `use_var(t1)` and the
  `phi` is inferred. LLVM's merge block *contains* the `phi`, spelled out with one
  `[value, predecessor]` pair per incoming edge.
- **`Copy` disappears in LLVM**: LLVM has no mutable variables to write into, so
  `Copy { dst, src }` emits *no instruction* — `values.insert(*dst, values[src])` just
  makes `dst` another name for an existing value. The only place `t1` becomes a real
  LLVM value is the `phi`.
- **Names are labels only**: `%truthy`, `%add`, `%if_result` come from the `name`
  string passed to each `build_*` call; LLVM doesn't care what they are.

The diagram shows `%x` as an opaque value. In the real build every input is a constant,
so `inkwell`'s builder folds the comparison to `br i1 true` (see the listing above);
the diagram shows what a non-constant condition would produce.

## The `inkwell` API and the LLVM C API beneath it

There are three layers between `llvm_backend.rs` and LLVM itself:

```
llvm_backend.rs  ->  inkwell (safe Rust wrappers)  ->  llvm-sys (raw `unsafe extern "C"`)  ->  LLVM (C++)
builder.build_float_add(a, b, "add")   ->   LLVMBuildFAdd(builder, a, b, "add")   ->   IRBuilder::CreateFAdd
```

- **LLVM** is C++ with no stable C++ ABI, so it ships a stable **C API**
  (`llvm-c/Core.h` and friends): plain functions named `LLVM…` that take and return
  opaque pointers (`LLVMValueRef`, `LLVMBuilderRef`, `LLVMBasicBlockRef`, …).
- **`llvm-sys`** declares those functions to Rust, one-to-one, all `unsafe`. Its version
  number tracks LLVM's (`211.x` = LLVM 21.1), which is why the `llvm21-1` feature has to
  match the installed LLVM.
- **`inkwell`** wraps each raw pointer in a Rust struct, turns the function names into
  methods, and adds what the C API lacks: **lifetimes** and **`Result`s**.

Here is one real `inkwell` method, from `inkwell-0.10.0/src/builder.rs` — the whole
wrapper is a position check, a C-string conversion, one `llvm-sys` call and a wrap:

```rust
pub fn build_float_add<T: FloatMathValue<'ctx>>(&self, lhs: T, rhs: T, name: &str)
    -> Result<T, BuilderError>
{
    if self.positioned.get() != PositionState::Set {
        return Err(BuilderError::UnsetPosition);        // inkwell's own check
    }
    let c_string = to_c_str(name);                       // &str -> NUL-terminated C string
    let value = unsafe {
        LLVMBuildFAdd(self.as_mut_ptr(), lhs.as_value_ref(), rhs.as_value_ref(), c_string.as_ptr())
    };
    unsafe { Ok(T::new(value)) }                         // raw pointer -> typed Rust value
}
```

That is why every `build_*` call in `llvm_backend.rs` ends in `.expect("builder is
positioned in a block")`: the `Result` exists only because `inkwell` checks the builder
has an insertion point (the C API would just crash). What else `inkwell` adds:

- **Lifetimes.** `Module<'ctx>`, `FloatValue<'ctx>` and `BasicBlock<'ctx>` can't outlive
  their `Context`. The C API hands out bare pointers and trusts you not to keep one
  after `LLVMContextDispose`; `inkwell` turns that trust into a compile error. It's why
  `'ctx` is threaded through `lower_instr`'s signature.
- **Typed values.** `LLVMValueRef` is one pointer type for everything; `inkwell` splits
  it into `FloatValue`, `IntValue`, `PhiValue`, `FunctionValue`, so passing an integer
  where a float is wanted is often a compile error rather than an LLVM assertion.
- **`Drop`.** Every owning type calls its `LLVM…Dispose…` function when it goes out of
  scope (`Context` → `LLVMContextDispose`, `Module` → `LLVMDisposeModule`, `Builder` →
  `LLVMDisposeBuilder`).
- **What it does not add.** `inkwell` can't check that instructions are well-formed
  (blocks terminated, `phi` entries matching predecessors, types lining up across an
  instruction). That is `module.verify()`'s job, and the table under "What LLVM
  enforces" below shows exactly how little the builder itself catches.

## Building real units: recipes

`inkwell`'s calls only make sense in a fixed order, and every larger unit (a block, an
`if`, a function, a whole object file) is that order repeated. So this section teaches by
building: four recipes, each a complete unit, then a reference table of every call this
backend makes and the C function it wraps. The four recipes are built as tests in
`tests/llvm_recipes.rs` (with `unwrap` where the snippets are shown), so they can't drift
from working code. The IR shown is what LLVM prints for each one
(`module.print_to_string()`).

**Two fixed orders cover everything.**

*Building a block* is always this sequence:

1. `context.append_basic_block(func, "name")` (`LLVMAppendBasicBlockInContext`) —
   allocates an empty block at the end of the function. Do it any time before you need to
   name the block: a branch must name its targets before they're filled.
2. `builder.position_at_end(block)` (`LLVMPositionBuilderAtEnd`) — moves the builder's
   insertion cursor. It only says *where new instructions go*; it creates no control flow.
3. `builder.build_*(..)` — emit instructions. Each `build_*` appends one instruction at
   the cursor and returns a `Result` holding that instruction's value.
4. **One terminator**: `build_return`, `build_unconditional_branch` or
   `build_conditional_branch`. `build_unconditional_branch` is what actually *creates*
   control flow ("continue at that block"); the conditional one is its two-way version.
   Nothing may follow a terminator in the same block.

That is the whole sequence: **there is no `seal_block` step.** Cranelift needed one because
it works out `phi`s for you and must know every predecessor first. LLVM never infers
anything: you write each `phi` and name its predecessors yourself, so there is nothing to
promise.

*Building a function* is always this sequence:

1. `Context::create()`, `context.create_module(name)`, `context.create_builder()` — the
   three objects everything hangs off. The `Context` owns every type, constant and value
   in the compilation, and nothing made in one `Context` can be mixed with another's.
2. Types: `context.f64_type()`, then `f64_ty.fn_type(&[param types], false)`
   (`LLVMFunctionType`) for the function's *type*. The `false` means "not variadic".
3. `module.add_function(name, fn_ty, None)` (`LLVMAddFunction`) → a `FunctionValue`.
   Unlike Cranelift there is no separate declare step: this *creates* the function, and
   other functions can call it immediately. `None` linkage means the default, external.
4. The entry block: `append_basic_block` then `position_at_end`. Parameters aren't block
   parameters here; they're read with `func.get_nth_param(i)` (`LLVMGetParam`).
5. The body: more blocks, each following the block sequence above.
6. `module.verify()` (`LLVMVerifyModule`) — LLVM's validator. **This is where nearly all
   mistakes are caught**, not at the `build_*` call that made them (see below).

**Recipe 1: a function that takes a parameter.** Parameters are already values, so there
is no "declare a variable" step:

```rust
let context = Context::create();
let module  = context.create_module("recipes");
let builder = context.create_builder();
let f64_ty  = context.f64_type();

let fn_ty = f64_ty.fn_type(&[f64_ty.into()], false);   // (double) -> double
let func  = module.add_function("twice", fn_ty, None);

let entry = context.append_basic_block(func, "entry");
builder.position_at_end(entry);

let x = func.get_nth_param(0).unwrap().into_float_value();
let doubled = builder.build_float_add(x, x, "doubled")?;
builder.build_return(Some(&doubled))?;                  // the terminator

module.verify()?;
```

```llvm
define double @twice(double %0) {
entry:
  %doubled = fadd double %0, %0
  ret double %doubled
}
```

Every Rust line has one visible effect: `fn_type` and `add_function` are the `define`
line, `append_basic_block` is `entry:`, `build_float_add` and `build_return` are the two
instructions. The unnamed parameter prints as `%0`. `.into_float_value()` converts
`inkwell`'s general `BasicValueEnum` into the specific `FloatValue` that `build_float_add`
wants. `define_calc_main` is this same recipe with no parameters and a body produced by
`lower_block`.

**Recipe 2: an `if`/`else` that produces a value.** This is `Instr::If` on its own, with a
parameter as the condition so LLVM's builder can't fold it away. Only the signature (three
`double` parameters, read with `get_nth_param` as `c`, `x`, `y`) and the body differ from
Recipe 1:

```rust
let truthy = builder
    .build_float_compare(FloatPredicate::UNE, c, f64_ty.const_zero(), "truthy")?;

let then_blk  = context.append_basic_block(func, "then");   // create all three up front,
let else_blk  = context.append_basic_block(func, "else");   // because the branch and both
let merge_blk = context.append_basic_block(func, "merge");  // jumps name them before they're filled
builder.build_conditional_branch(truthy, then_blk, else_blk)?;

builder.position_at_end(then_blk);                          // nothing to compute here
builder.build_unconditional_branch(merge_blk)?;

builder.position_at_end(else_blk);
builder.build_unconditional_branch(merge_blk)?;

builder.position_at_end(merge_blk);                         // the phi goes first in `merge`
let phi = builder.build_phi(f64_ty, "result")?;             // an empty phi of type double
phi.add_incoming(&[(&x, then_blk), (&y, else_blk)]);        // [value, predecessor] pairs
builder.build_return(Some(&phi.as_basic_value()))?;
```

```llvm
define double @select(double %c, double %x, double %y) {
entry:
  %truthy = fcmp une double %c, 0.000000e+00
  br i1 %truthy, label %then, label %else

then:                                             ; preds = %entry
  br label %merge

else:                                             ; preds = %entry
  br label %merge

merge:                                            ; preds = %else, %then
  %result = phi double [ %x, %then ], [ %y, %else ]
  ret double %result
}
```

Read it against the code: `build_conditional_branch` is the `br i1 …`; each
`build_unconditional_branch` is a `br label %merge`; and `build_phi` + `add_incoming` is
the `phi` line, whose two `[value, block]` pairs are exactly the two tuples passed in.
**The `phi` is the thing Cranelift's `use_var` produced for you** (`block3(v5: f64)` in
A6's Recipe 2); here you write it, which is why you also had to name each predecessor
yourself. `build_phi` and `add_incoming` are separate calls so the entries can be added
after the predecessor blocks have been built, a fact the "extending" sketch below relies on.

**Recipe 3: one function calling another.** Create the callee first (only so you have its
`FunctionValue`; LLVM doesn't require it to have a body yet), then hand that value straight
to `build_call`. There is no per-function import step like Cranelift's
`declare_func_in_func`:

```rust
let result = builder
    .build_call(five, &[], "result")?          // (callee, arguments, name)
    .try_as_basic_value()                      // a call *could* return `void`, so it's an enum
    .unwrap_basic();                           // we know it returns a double
builder.build_return(Some(&result))?;
```

```llvm
define double @five() {
entry:
  ret double 5.000000e+00
}

define double @caller() {
entry:
  %result = call double @five()
  ret double %result
}
```

The call instruction *is* its own result value (`%result`); Cranelift needed a separate
`inst_results(call)[0]` lookup. `define_c_main` is exactly this, plus a call to the
`llvm.fptosi.sat` intrinsic on the result.

**Recipe 4: from a module to an object file.** Everything about *which machine* lives in
`host_machine()`, opened up:

```rust
fn host_machine() -> TargetMachine {
    Target::initialize_native(&InitializationConfig::default())?;   // register the host's backend
    let triple = TargetMachine::get_default_triple();               // e.g. x86_64-pc-windows-msvc
    let target = Target::from_triple(&triple)?;
    target.create_target_machine(
        &triple,
        &TargetMachine::get_host_cpu_name().to_string_lossy(),      // "the CPU running this code"
        &TargetMachine::get_host_cpu_features().to_string_lossy(),  // ...and its instruction sets
        OptimizationLevel::Default, RelocMode::Default, CodeModel::Default,
    )?
}
```

A `TargetMachine` is LLVM's counterpart of Cranelift's `TargetIsa`: the object that turns
IR into machine instructions for one CPU, carrying the optimization level and relocation
model. Like Cranelift's `cranelift_native::builder()`, no target triple is hardcoded, so
`calcc` compiles for the machine it runs on. Then the whole of `compile_to_object` is:
build the module, ask the machine to emit it.

```rust
let module  = build_module(&context, program);    // Recipes 1-3 fill it, then module.verify()
let machine = host_machine();
module.set_triple(&machine.get_triple());         // stamp the module with the machine's target...
module.set_data_layout(&machine.get_target_data().get_data_layout());  // ...and data layout
let bytes = machine.write_to_memory_buffer(&module, FileType::Object)?
    .as_slice().to_vec();                         // instruction selection + register allocation
                                                  // + object-file encoding: the .obj bytes
```

**Every `inkwell` call this backend makes, and the C function it wraps.** The recipes cover
the structure; this is the full list for reference (`llvm-sys` names, all in
`llvm-c/*.h`). A6's matching table, "Other calls this backend makes", lists only calls its
recipes don't use; this one lists *all* of them, because mapping each `inkwell` call to its C
function is this page's second job:

| `inkwell` call | LLVM C function | What it does here |
| --- | --- | --- |
| `Context::create()` | `LLVMContextCreate` | Owner of every type and value; `Drop` calls `LLVMContextDispose`. |
| `context.create_module(name)` | `LLVMModuleCreateWithNameInContext` | One compilation unit; becomes one object file. |
| `context.create_builder()` | `LLVMCreateBuilderInContext` | The instruction cursor; one builder is reused for every function. |
| `context.f64_type()` / `i32_type()` | `LLVMDoubleTypeInContext` / `LLVMInt32TypeInContext` | The types (`f64` is calc-lang's one runtime type). |
| `ty.fn_type(&[..], false)` | `LLVMFunctionType` | A function's type: return type + parameter types. |
| `f64_ty.const_float(v)` / `const_zero()` | `LLVMConstReal` / `LLVMConstNull` | A constant (`Instr::Const`; the `0.0` in `If`). Emits **no instruction**. |
| `module.add_function(name, ty, None)` | `LLVMAddFunction` | Creates a function (declare + define in one). |
| `func.get_nth_param(i)` | `LLVMGetParam` | A parameter, already a value. |
| `context.append_basic_block(f, name)` | `LLVMAppendBasicBlockInContext` | An empty block at the end of `f`. |
| `builder.position_at_end(b)` | `LLVMPositionBuilderAtEnd` | Move the cursor into `b`. |
| `builder.get_insert_block()` | `LLVMGetInsertBlock` | "Which block is the cursor in *now*?" (nested `if`s move it). |
| `build_float_add/sub/mul/div` | `LLVMBuildFAdd/FSub/FMul/FDiv` | `Instr::BinOp`, one per `calc_ir::BinOp`. |
| `build_float_compare(UNE, a, b, n)` | `LLVMBuildFCmp` | Produces an `i1`. `UNE` is true for NaN, like Cranelift's `NotEqual`. |
| `build_conditional_branch(c, t, e)` | `LLVMBuildCondBr` | A two-way terminator. |
| `build_unconditional_branch(t)` | `LLVMBuildBr` | A one-way terminator. |
| `build_phi(ty, name)` | `LLVMBuildPhi` | An empty `phi`; must be first in its block. |
| `phi.add_incoming(&[(&v, blk), ..])` | `LLVMAddIncoming` | Fills the `[value, predecessor]` pairs (`inkwell` splits the slice into the two parallel C arrays). |
| `build_call(f, &[args], name)` | `LLVMBuildCall2` | `main` calling `calc_main`, and the intrinsic call. |
| `build_return(Some(&v))` | `LLVMBuildRet` | Ends the function with a value. |
| `Intrinsic::find("llvm.fptosi.sat")` then `.get_declaration(&module, &[i32, f64])` | `LLVMLookupIntrinsicID` then `LLVMGetIntrinsicDeclaration` | LLVM's built-in saturating float→int conversion, declared in the module. Cranelift's `fcvt_to_sint_sat` is one instruction; LLVM makes it a call its backend replaces with real instructions. |
| `module.verify()` | `LLVMVerifyModule` | LLVM's IR validator. |
| `module.print_to_string()` | `LLVMPrintModuleToString` | The textual `.ll` form shown on this page. |
| `Target::initialize_native(..)` | `LLVM_InitializeNativeTarget` (and its asm-printer siblings) | LLVM registers no targets until asked. |
| `TargetMachine::get_default_triple()` / `Target::from_triple(..)` | `LLVMGetDefaultTargetTriple` / `LLVMGetTargetFromTriple` | The host's target triple, and the `Target` for it. |
| `get_host_cpu_name()` / `get_host_cpu_features()` | `LLVMGetHostCPUName` / `LLVMGetHostCPUFeatures` | Lets LLVM use this CPU's instructions (why the disassembly has AVX `vmovsd`). |
| `target.create_target_machine(..)` | `LLVMCreateTargetMachine` | The `TargetIsa` analog. |
| `module.set_triple(..)` / `set_data_layout(..)` | `LLVMSetTarget` / `LLVMSetDataLayout` | Stamp the module with the target. |
| `machine.write_to_memory_buffer(&m, FileType::Object)` | `LLVMTargetMachineEmitToMemoryBuffer` | Machine code out, as object-file bytes. |
| `module.run_passes("default<O2>", ..)` | `LLVMRunPasses` | Runs LLVM's optimizer by name (only in the `-O2` test). |

**What LLVM enforces.** Each of these mistakes was made deliberately against this crate
version (`inkwell` 0.10.0, LLVM 21.1.1) in `tests/llvm_recipes.rs`'s `mistakes` module, so
the messages below are verbatim:

| Mistake | What happens |
| --- | --- |
| `build_*` before any `position_at_end` | `Err(BuilderError::UnsetPosition)` at the call: `Builder position is not set` |
| Block never ends in a terminator | `verify()` returns `Err`: `Basic Block in function 'f' does not have terminator!` |
| Instruction after the terminator | the same `verify()` error: the block's *last* instruction is no longer a terminator |
| Returned an `i32` from a `double` function | `verify()`: `Function return type does not match operand type of return inst!` |
| `phi` entry names a block that isn't a predecessor | `verify()`: `PHI node entries do not match predecessors!` |
| A `phi` after a non-`phi` instruction | `verify()`: `PHI nodes not grouped at top of basic block!` |
| Used a value defined on only some paths | `verify()`: `Instruction does not dominate all uses!` |

Compare with A6's version of this table: Cranelift's builder **panics at the mistaken
call** (`you cannot add an instruction to a block already filled`, and so on), so most
errors point at the line that made them. `inkwell`'s builder catches only the first mistake
in the table and lets you keep building; every other mistake surfaces later, from `verify()`,
naming the function and the offending instruction but not the Rust line that created it.
That is why `build_module` calls `module.verify()` unconditionally, and why the verifier's
message is the first thing to read when a lowering change misbehaves.

**Extending to something new.** A construct calc-lang doesn't have yet, like `while`, is the
same two orders with one twist, and it's where `add_incoming` being a separate call pays
off: the loop variable's `phi` needs an entry from a block that doesn't exist yet (the end
of the body). Sketch:

1. `append_basic_block` for `header`, `body`, `exit`; `build_unconditional_branch(header)`
   from where you are.
2. `position_at_end(header)`. Build the loop variable's `phi` **first** (a `phi` must be the
   first instruction), with only the entry from the pre-loop block:
   `phi.add_incoming(&[(&initial, before)])`. Then the condition and
   `build_conditional_branch(cond, body, exit)`.
3. `position_at_end(body)`, emit the body (using the `phi` as the variable), then
   `build_unconditional_branch(header)`. Use `get_insert_block()` afterwards: if the body
   contained an `if`, the block it *ends* in is not `body`.
4. Now add the back-edge entry: `phi.add_incoming(&[(&updated, body_end)])`. Then
   `position_at_end(exit)`.

(`calc-lang` has no loop construct, so this is a sketch, not code in this repo; see
`DECISIONS.md`'s A4 entry on deferring `Loop`. Note the contrast with Cranelift: there the
same problem is solved by *not sealing* the header until the back-edge exists.)

## How the backend builds the `phi`

Recipe 2 built an `if`/`else` from function parameters. This section is the same shape
driven by `calc-ir`: it's `lower_instr`'s `Instr::If` arm, which is Recipe 2 with
`lower_block` filling each branch instead of nothing, and a per-branch value table to feed
the `phi`. In A6, Cranelift's `Variable`s (`def_var`/`use_var`) built the merge for us; here
it's written by hand. Below is that arm's real code, split into its four steps, with the LLVM IR that exists *after* each
step. The example is `if 1 { 10 } else { 20 }` (constant, so the compare has folded to
`br i1 true`, as explained above).

**Step 1 — compare, create three blocks, branch.**

```rust
let is_truthy = builder
    .build_float_compare(FloatPredicate::UNE, values[cond], f64_ty.const_zero(), "truthy")
    .expect("builder is positioned in a block");

let then_blk  = context.append_basic_block(func, "then");
let else_blk  = context.append_basic_block(func, "else");
let merge_blk = context.append_basic_block(func, "merge");
builder
    .build_conditional_branch(is_truthy, then_blk, else_blk)
    .expect("builder is positioned in a block");
```

`une` ("unordered or not equal") is true for NaN, matching Rust's `x != 0.0` and A6's
`FloatCC::NotEqual`. The three new blocks exist but are empty, and the cursor is still in
the *old* block, which now ends in the branch:

```llvm
entry:
  br i1 true, label %then, label %else     ; <- the conditional branch just built
then:                                       ; <- empty: no instructions, no terminator yet
else:                                       ; <- empty
merge:                                      ; <- empty
```

**Step 2 — lower each branch into its own block.**

```rust
let branch_value = |blk, ir_block: &IrBlock| {
    builder.position_at_end(blk);                       // move the cursor into the block
    let mut branch_values = values.clone();             // <- its own copy of the temp table
    lower_block(context, builder, func, ir_block, &mut branch_values);
    builder.build_unconditional_branch(merge_blk)       // <- close the block: jump to merge
        .expect("builder is positioned in a block");
    // ...step 3 continues here
```

Each branch gets a **clone** of `values` (the `Temp -> FloatValue` table) because a value
defined inside `then` doesn't exist on the `else` path; only the `If`'s own `dst` is
allowed to escape, through the `phi`. The unconditional branch is what makes the block
legal, since LLVM requires every block to end in exactly one *terminator*. Here both
branches are only `Const`s and a `Copy` (which emits nothing), so all that appears is:

```llvm
then:                                       ; preds = %entry
  br label %merge                           ; <- the constant 10.0 needed no instruction
else:                                       ; preds = %entry
  br label %merge
```

**Step 3 — remember each branch's final value and the block it ended in.**

```rust
    // (still inside the closure)
    (
        branch_values[dst],                                    // the value the branch's Copy wrote to `dst`
        builder.get_insert_block().expect("builder has a block"),   // where the cursor is NOW
    )
};
let (then_val, then_end) = branch_value(then_blk, then_block);
let (else_val, else_end) = branch_value(else_blk, else_block);
```

`then_val` is the constant `10.0` and `else_val` is `20.0` — Rust values, not IR text, so
there is nothing new in the IR yet. The reason to ask the builder where it ended, rather
than just remembering `then_blk`, is the nested case, covered below.

**Step 4 — move to `merge` and build the `phi`.**

```rust
builder.position_at_end(merge_blk);
let phi = builder
    .build_phi(f64_ty, "if_result")
    .expect("builder is positioned in a block");
phi.add_incoming(&[(&then_val, then_end), (&else_val, else_end)]);
values.insert(*dst, phi.as_basic_value().into_float_value());   // `dst` now means "the phi"
```

`build_phi` creates an empty `phi`; `add_incoming` fills in one `[value, predecessor]` pair
per entry. Whatever code follows the `If` in `calc-ir` continues from `merge_blk` and reads
`dst` from `values`, which now holds the `phi`. Final IR for the whole function:

```llvm
define double @calc_main() {
entry:
  br i1 true, label %then, label %else
then:                                       ; preds = %entry
  br label %merge
else:                                       ; preds = %entry
  br label %merge
merge:                                      ; preds = %else, %then
  %if_result = phi double [ 1.000000e+01, %then ], [ 2.000000e+01, %else ]
  ret double %if_result                     ; <- from define_calc_main's build_return
}
```

Compare with the diagram above: the `phi`'s two pairs are exactly `(then_val, then_end)`
and `(else_val, else_end)` from step 3.

### Why step 3 asks the builder where it ended: a nested `if`

Take `if 1 { if 0 { 1 } else { 2 } } else { 3 }`. Lowering the outer `then` branch runs
`lower_instr` *again* for the inner `If`, which creates its own blocks and leaves the
cursor in the **inner** `merge`. This is the real output (LLVM auto-numbers duplicate
names, so the inner blocks are `then1`, `else2`, `merge3`):

```llvm
define double @calc_main() {
entry:
  br i1 true, label %then, label %else
then:                                       ; preds = %entry      (outer then)
  br i1 false, label %then1, label %else2                         ; inner If's compare + branch
else:                                       ; preds = %entry      (outer else)
  br label %merge
merge:                                      ; preds = %else, %merge3
  %if_result4 = phi double [ %if_result, %merge3 ], [ 3.000000e+00, %else ]
  ret double %if_result4
then1:                                      ; preds = %then
  br label %merge3
else2:                                      ; preds = %then
  br label %merge3
merge3:                                     ; preds = %else2, %then1
  %if_result = phi double [ 1.000000e+00, %then1 ], [ 2.000000e+00, %else2 ]
  br label %merge                                                 ; the outer then's closing branch
}
```

Look at the outer `phi` (`%if_result4`): its first entry names **`%merge3`** — the block
the outer `then` branch *actually* jumps from — not `%then`. Nothing in `%then` branches to
`%merge`; the jump comes from `%merge3`. LLVM requires a `phi`'s entries to match the
block's real predecessors exactly (the `preds =` comment lists them: `%else, %merge3`), so
naming `%then` would make the module invalid. `builder.get_insert_block()` returns
`%merge3` because the inner `If`'s step 4 left the cursor there, so the closure captures it
after `lower_block` returns and before anything else moves the cursor. The
`compiles_and_runs_a_nested_if` test runs this program, and `build_module`'s
`module.verify()` (LLVM's IR validator) would reject the wrong version before any code was
generated. (Blocks print in creation order, which is why `merge` appears before `then1`;
order in the listing doesn't affect meaning.)

## What LLVM buys over Cranelift: comparing real output

Both backends were given the same program, `if 1 { 10 } else { 20 }`, and the resulting
object files were disassembled with `llvm-objdump` (x86-64, Windows):

**LLVM** (`calc_main`, at the default codegen level):

```asm
vmovsd  xmm0, qword ptr [rip]   ; load the constant 10.0
ret
```

**Cranelift** (`calc_main`, A6's settings): a stack-frame prologue, save/restore of
`rsi`/`rdi`/`xmm7`, materialize `1.0`, `vucomisd` against `0.0`, two conditional jumps,
load `20.0` or `10.0` into `xmm0`, then a jump and epilogue — about 0x75 bytes of code
versus LLVM's 9.[¹](#footnote-1)

The full disassembly of both objects, annotated line by line, is in
[footnote 1](#footnote-1) at the bottom of this page.

LLVM decided at compile time that the branch is always taken and emitted only the
result. Cranelift compiled the branch structure faithfully, since its default is to
spend as little compile time as possible. To see LLVM's full optimizer,
`optimizer_collapses_a_constant_if` runs the standard `default<O2>` pass pipeline over
the module: the whole `calc_main` becomes `ret double 1.000000e+01`, and even `main`
folds to `ret i32 10`.

That's the trade in one example. LLVM has decades of optimization passes; Cranelift
compiles faster and is a pure-Rust dependency. The price on the LLVM side: an external
install, a ~1 min cold build of `llvm-sys`/`inkwell`, and a bigger object (1049 bytes
against ~350 for these tiny programs — mostly metadata LLVM adds, not code). For a
program this small the *code* difference is invisible at runtime; it's the shape of the
output that teaches.

Everything else in `llvm_backend.rs` is the A6 story with different names: a
`calc_main() -> double` holding the program, and a C `main() -> i32` returning
`llvm.fptosi.sat.i32.f64` of the result as the exit code (the saturating conversion A6
got from `fcvt_to_sint_sat`; a plain `fptosi` gives *poison* — undefined — for
out-of-range input). Linking reuses A6's `link_stub` unchanged: it finds MSVC through
the `cc` crate, which also sidesteps Git for Windows' `/usr/bin/link.exe` shadowing
Microsoft's `link.exe` on `PATH`.

## `unsafe` — what it is, why it's here, and what it means for you

Rust's headline promise is **memory safety**: in ordinary ("safe") Rust the compiler
proves you can't use freed memory, read past the end of an array, or have two threads
race on the same data. It checks this through ownership and lifetimes — which is why
`Builder<'ctx>` and `FloatValue<'ctx>` in this file carry that odd `'ctx` annotation.

`unsafe` is a keyword that marks a block (or function, or trait) where the *programmer*
takes over that job. Inside an `unsafe` block you may do a handful of things the
compiler can't verify — dereference a raw pointer, call a function it can't inspect —
but note what it does *not* do: it doesn't turn off type checking, borrow checking of
everything else, or bounds checks. It just says "for these specific operations, trust
me; I've checked the rules myself."

### Why LLVM needs it

LLVM is written in C++ and exposed through a **C API** (`llvm-c/*.h`). When Rust calls a
C function (an "FFI" — foreign function interface — call) the compiler has no idea what
the function does with its pointers: does it free them? keep them? assume they're
non-null? There's nothing to check against, so *every* such call is `unsafe`. The
`llvm-sys` crate is the raw, mechanical binding: hundreds of `unsafe extern "C"`
functions handing around `*mut LLVMValue` pointers.

### Where the `unsafe` lives — and where it doesn't

`llvm_backend.rs` contains **no `unsafe` blocks**. Deliberately, `inkwell` sits between
us and `llvm-sys`: it wraps each raw pointer in a Rust type (`Module`, `Builder`,
`FloatValue`, …) whose lifetimes encode LLVM's ownership rules — a `FloatValue<'ctx>`
can't outlive the `Context` it came from, and `Context`'s `Drop` (Rust's destructor)
frees LLVM's memory exactly once. That's the standard Rust pattern: a small, carefully
reviewed layer of `unsafe` inside a library, exposing a **safe API** on top so callers
can't trigger the bad cases by accident. You can see the boundary yourself by searching
`inkwell`'s source for `unsafe`: it's concentrated in wrappers around `llvm-sys` calls.

### What this means for us as users

- **Our own code is still fully checked** by the compiler; there is nothing here for
  us to audit line-by-line. But we're *trusting* `inkwell`'s authors that their wrappers
  really uphold LLVM's rules — a bug in there could cause a crash or memory corruption
  that no amount of correct code on our side would prevent. That trust is the same kind
  we already extend to the standard library, which is itself built on `unsafe`.
- **"Safe" doesn't mean "can't go wrong".** Safe Rust guarantees no memory
  *corruption*; it can't guarantee LLVM gets *sensible IR*. Handing LLVM malformed
  IR (a `phi` naming the wrong predecessor, mismatched types) may produce a crash deep
  inside C++ or, worse, silently wrong machine code. That is why `build_module` always
  calls `module.verify()` — a cheap check that turns that class of bug into a
  readable panic message during development.
- **Build-time cost and portability are the practical consequences**: because the C++
  library is outside Cargo's world, the build needs `LLVM_SYS_211_PREFIX` and a matching
  LLVM version, and `cargo audit` and Rust's guarantees stop at the FFI boundary.
  Cranelift, being pure Rust, doesn't have this boundary at all, which is part of the
  argument for shipping both (spec.md §8.1).
- **If you ever write your own `unsafe`**, the convention is a `// SAFETY:` comment
  above the block stating the invariant you're relying on. There's none in this session
  because `inkwell` already covers everything this backend needs; if an API we need
  turns out to require it (some JIT-execution functions do), that comment is where the
  reasoning belongs.

## The interpreter and Cranelift as this backend's oracles

A6 checked Cranelift's output against A5's interpreter, and said a second real backend to
compare against didn't exist yet. Now it does, so each test in `llvm_backend.rs` checks
*two* things: the LLVM-built executable's exit code equals the interpreter's answer, **and**
equals the Cranelift-built executable's exit code for the same program:

```rust
fn assert_matches_interpreter_and_cranelift(src: &str, out_name: &str) {
    let program = lower_source(src);
    let calc_ir::Value::Number(expected) = calc_ir::interpret(&program);

    let llvm = link_and_run(&compile_to_object(&program), &format!("llvm_{out_name}"));
    let clif = link_and_run(&cranelift_backend::compile_to_object(&program), &format!("clif_{out_name}"));
    assert_eq!(llvm, expected as i32);
    assert_eq!(llvm, clif);
}
```

The programs are A6's four (straight-line arithmetic, both branches of an `if`/`else`, a
`let`-bound `if`) plus a nested `if` for the `phi` predecessor case. Both objects go through
the same unchanged `link_stub::link`. This is still differential testing in miniature; C4
formalizes it later across every sample program and every backend.

## What's deliberately not here yet

No `Backend` trait (A8 — this session picked a plain `fn(&Program) -> Vec<u8>` so `main.rs`
can share one `build` function, nothing more), no CI job that installs LLVM (CI builds only
the default, Cranelift configuration; `DECISIONS.md`'s A7 entry explains why), no real link
driver (A12), and no non-x86 hosts (`target-x86` only). Recipes and enforcement details
above are about building IR with `inkwell`; the backend itself still lowers only what
`calc-ir` has (`Const`, `BinOp`, `Copy`, `If`).

## Recap

- `--features backend-llvm` gates the LLVM dependency; off by default so nothing else
  needs LLVM installed.
- LLVM IR is SSA over basic blocks; a `phi` in the merge block picks the value based on
  which predecessor ran, and it must name the block lowering *ended* in.
- Same IR, two backends: Cranelift compiles the branch, LLVM's constant folding removes
  it. Tests confirm interpreter, Cranelift and LLVM all agree.
- The recipes (function, `if`/`else` + `phi`, call, object file) are built as tests in
  `tests/llvm_recipes.rs`; unlike Cranelift's builder, `inkwell`'s mostly defers mistakes to
  `module.verify()`.
- `unsafe` is the FFI boundary to C++; `inkwell` keeps it out of our code, and
  `module.verify()` covers what safety can't.

---

## Footnotes

<a id="footnote-1"></a>
**1. Full disassembly of the Cranelift and LLVM objects** ([back to the comparison](#what-llvm-buys-over-cranelift-comparing-real-output))

<details>
<summary>Show the full disassembly of both objects (<code>calc_main</code> and <code>main</code>)</summary>

Produced with `llvm-objdump -d -M intel` on each backend's object for
`if 1 { 10 } else { 20 }`. The `;` comments are added here; the addresses and bytes are
the tool's own. Both objects are COFF (Windows), so `call` targets show as `0x…` until
the linker fills them in.

**LLVM: `calc_main` (9 bytes of code) and `main`**

```asm
0000000000000000 <calc_main>:
   0: c5 fb 10 05 00 00 00 00   vmovsd  xmm0, qword ptr [rip]  ; xmm0 = 10.0 (from the constant pool)
   8: c3                        ret

0000000000000010 <main>:
  10: 48 83 ec 28               sub     rsp, 0x28              ; shadow space the Windows ABI requires for calls
  14: e8 00 00 00 00            call    <calc_main>            ; result comes back in xmm0
  19: c5 fb 5f 0d 00 00 00 00   vmaxsd  xmm1, xmm0, qword ptr [rip]  ; --+ clamp to i32's
  21: c5 f3 5d 0d 00 00 00 00   vminsd  xmm1, xmm1, qword ptr [rip]  ; --+ min/max range
  29: c5 fb 2c c9               vcvttsd2si ecx, xmm1           ; truncate the clamped double to i32
  2d: 31 c0                     xor     eax, eax               ; eax = 0
  2f: c5 f9 2e c0               vucomisd xmm0, xmm0            ; is the result NaN? (x != x)
  33: 0f 4b c1                  cmovnp  eax, ecx               ; not NaN: eax = converted value, else stay 0
  36: 48 83 c4 28               add     rsp, 0x28
  3a: c3                        ret
```

No branch survives in `calc_main`: LLVM's `TargetMachine` folded `br i1 true` while
generating code. `main`'s saturating conversion (`llvm.fptosi.sat`) became
branch-free `vmaxsd`/`vminsd`/`cmovnp` instead of a chain of compares.

**Cranelift: `calc_main` (0x75 bytes of code) and `main`**

```asm
0000000000000000 <calc_main>:
   0: 55                        push    rbp                    ; ---- prologue: build a stack frame
   1: 48 89 e5                  mov     rbp, rsp
   4: 48 83 ec 20              sub     rsp, 0x20
   8: 48 89 34 24              mov     qword ptr [rsp], rsi   ; save rsi, rdi, xmm7 (callee-saved on
   c: 48 89 7c 24 08           mov     qword ptr [rsp + 0x8], rdi  ;   Windows x64) before using them
  11: f3 0f 7f 7c 24 10        movdqu  xmmword ptr [rsp + 0x10], xmm7
  17: 48 be 00 00 00 00 00 00 f0 3f   movabs rsi, 0x3ff0000000000000  ; the bits of 1.0
  21: c4 e1 f9 6e fe           vmovq   xmm7, rsi              ; v0 = f64const 1.0
  26: c5 f9 2e 3d 52 00 00 00  vucomisd xmm7, qword ptr [rip + 0x52]  ; compare v0 with 0.0 (at 0x80)
  2e: 0f 8a 1a 00 00 00        jp      0x4e                   ; unordered (NaN): truthy -> then
  34: 0f 85 14 00 00 00        jne     0x4e                   ; not equal to 0.0: truthy -> then
  3a: 48 bf 00 00 00 00 00 00 34 40   movabs rdi, 0x4034000000000000  ; else: the bits of 20.0
  44: c4 e1 f9 6e c7           vmovq   xmm0, rdi              ; xmm0 = 20.0
  49: e9 0f 00 00 00           jmp     0x5d                   ; jump to the merge block
  4e: 49 b8 00 00 00 00 00 00 24 40   movabs r8, 0x4024000000000000   ; then: the bits of 10.0
  58: c4 c1 f9 6e c0           vmovq   xmm0, r8               ; xmm0 = 10.0
  5d: 48 8b 34 24              mov     rsi, qword ptr [rsp]   ; merge: restore the saved registers
  61: 48 8b 7c 24 08           mov     rdi, qword ptr [rsp + 0x8]
  66: f3 0f 6f 7c 24 10        movdqu  xmm7, xmmword ptr [rsp + 0x10]
  6c: 48 83 c4 20             add     rsp, 0x20              ; ---- epilogue
  70: 48 89 ec                 mov     rsp, rbp
  73: 5d                       pop     rbp
  74: c3                       ret
                                                              ; (0x75..0x7f: padding; 0x80: the 0.0 constant)

0000000000000090 <main>:
  90: 55                        push    rbp
  91: 48 89 e5                  mov     rbp, rsp
  94: 48 83 ec 30              sub     rsp, 0x30
  98: 48 89 74 24 20           mov     qword ptr [rsp + 0x20], rsi
  9d: e8 00 00 00 00           call    <calc_main>
  a2: f2 0f 2c c0              cvttsd2si eax, xmm0            ; try the plain truncation first
  a6: 83 f8 01                 cmp     eax, 0x1               ; 0x80000000 ("indefinite") means it overflowed:
  a9: 0f 81 24 00 00 00       jno     0xd3                   ;   otherwise the result is fine, done
  af: 66 0f 2e c0              ucomisd xmm0, xmm0             ; overflowed: was it NaN?
  b3: 0f 8b 07 00 00 00       jnp     0xc0
  b9: 33 c0                    xor     eax, eax               ;   NaN -> 0
  bb: e9 13 00 00 00          jmp     0xd3
  c0: 66 0f 57 d2              xorpd   xmm2, xmm2             ; not NaN: which side did it overflow?
  c4: 66 0f 2e d0              ucomisd xmm2, xmm0
  c8: 0f 83 05 00 00 00       jae     0xd3                   ;   negative overflow -> keep 0x80000000
  ce: b8 ff ff ff 7f          mov     eax, 0x7fffffff        ;   positive overflow -> i32::MAX
  d3: 48 8b 74 24 20           mov     rsi, qword ptr [rsp + 0x20]
  d8: 48 83 c4 30             add     rsp, 0x30
  dc: 48 89 ec                 mov     rsp, rbp
  df: 5d                       pop     rbp
  e0: c3                       ret
```

Two differences to take from it. **Structure**: Cranelift's `calc_main` still contains the
compare-and-branch, the frame setup and register saves, because it compiled the IR it was
given without folding the constant condition. **The same job, different shape**: both
backends implement the saturating `f64 -> i32` conversion in `main`, but LLVM does it
branch-free with `vmaxsd`/`vminsd`/`cmovnp`, while Cranelift emits a compare-and-jump
sequence.

</details>
