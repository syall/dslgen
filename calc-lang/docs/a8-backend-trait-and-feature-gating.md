# A8 — The `Backend` trait and feature-gated backend selection

**Session code**:
[`crates/calc-compiler/src/backend.rs`](../crates/calc-compiler/src/backend.rs),
[`crates/calc-compiler/src/main.rs`](../crates/calc-compiler/src/main.rs),
[`crates/calc-compiler/Cargo.toml`](../crates/calc-compiler/Cargo.toml),
[`crates/calc-compiler/src/cranelift_backend.rs`](../crates/calc-compiler/src/cranelift_backend.rs),
[`crates/calc-compiler/src/llvm_backend.rs`](../crates/calc-compiler/src/llvm_backend.rs).
**Spec refs**: spec.md §8.1 (pluggable codegen backends), §14 #4 (feature-flag
defaults). **Prereqs**: [A6](a6-cranelift-codegen-backend.md),
[A7](a7-llvm-codegen-backend.md).

After A7, `calc-lang` has two ways to turn the same IR into an executable, but `main.rs`
knew about each by name: one `match` arm per `--backend=` string, each passing a
different function to `build`. Adding a third backend would mean editing `main.rs`
again. A8 fixes that with two ideas that work together:

1. A **trait** (`Backend`) that says what every backend can do, so the rest of the
   compiler talks to "a backend" and never to "Cranelift" or "LLVM".
2. **Cargo features** that decide which backends are compiled into a given `calcc`
   at all, so someone who only wants Cranelift never needs LLVM installed.

## The extension-point pattern

A compiler that supports several backends has a choice about where the "which one?"
decision lives. The cheap way is `if backend == "llvm" { … } else { … }` sprinkled
wherever the difference matters. It works for two backends and becomes a maintenance
problem at five, because every new backend touches every `if`.

The pattern real compilers use instead is an **extension point**: one narrow interface
that every backend implements, and one place that knows how to pick among them.
Everything else is written against the interface. Here it is in full:

```rust
pub trait Backend {
    fn name(&self) -> &'static str;
    fn compile(&self, program: &Program) -> Result<Vec<u8>, BackendError>;
}
```

A trait is Rust's word for "a set of methods a type promises to provide". `Backend`
says: give me the IR (`Program`), I'll give you the bytes of an object file (or a
`BackendError`). Both backends now implement it:

```rust
pub struct CraneliftBackend;          // no fields — it carries no settings yet

impl Backend for CraneliftBackend {
    fn name(&self) -> &'static str { "cranelift" }
    fn compile(&self, program: &Program) -> Result<Vec<u8>, BackendError> {
        Ok(compile_to_object(program))   // A6's function, unchanged
    }
}
```

`LlvmBackend` is the same shape around A7's function. That is the entire refactor of A6
and A7: neither backend's real code changed.

This is why "add a backend later" is cheap (spec.md §8.1's phrase): a WASM backend would be
one new `impl Backend` plus one line in the list below. The parser, resolver, IR and
`main.rs`'s `build` function would not change.

### What `dyn` means

`dyn` is short for *dynamic* (dynamic dispatch): it marks a type as "some value that
implements this trait, and I'll find out which one at run time". `Backend` on its own
is a trait, not a type: you can't have a variable "of type `Backend`", because
`CraneliftBackend` and `LlvmBackend` are different types that may differ in size and
layout. `dyn Backend` is the type that stands for "either of them".

Its size isn't known at compile time (it depends on which backend is behind it), and
Rust needs a fixed size to put a value in a variable or a `Vec`. So a `dyn` value can
only be used *behind a pointer*: `&dyn Backend`, `Box<dyn Backend>`, and so on.

```rust
let b: &dyn Backend = &CraneliftBackend;   // vtable for CraneliftBackend
let b: &dyn Backend = &LlvmBackend;        // same type, different vtable
```

Both lines have exactly the same type, so `enabled()` can put them in one
`Vec<&'static dyn Backend>`. (`'static` just says the references live for the whole
program; they point at unit structs that hold no data.)

### The fat pointer

A normal reference, like `&CraneliftBackend`, is a single machine address. A reference
to a `dyn` type is a **fat pointer**: *two* addresses side by side.

```text
&CraneliftBackend            &dyn Backend (fat pointer)
┌──────────────┐             ┌──────────────┬──────────────┐
│ data address │             │ data address │ vtable addr  │
└──────────────┘             └──────┬───────┴──────┬───────┘
                                    │              │
                    the value ◄─────┘              ▼
                    (an empty struct here)   vtable for `CraneliftBackend as Backend`
                                             ┌────────────────────────────┐
                                             │ drop glue (cleanup fn)     │
                                             │ size, alignment            │
                                             │ → CraneliftBackend::name   │
                                             │ → CraneliftBackend::compile│
                                             └────────────────────────────┘
```

- The **data address** points at the actual value. For our backends that's a
  zero-sized struct, so it points at nothing meaningful. A backend with settings
  would point at those.
- The **vtable address** points at a small read-only table the compiler generates *once
  per (type, trait) pair*: `CraneliftBackend`'s and `LlvmBackend`'s `Backend`
  vtables are two different tables. It holds the type's size and alignment, how to
  clean it up, and a pointer to each trait method's implementation for that type.

The pointer itself carries "what this is", so no one has to remember it separately.

You can see the doubling with the compiler (64-bit machine):

```text
&CraneliftBackend  = 8 bytes     (one address)
&dyn Backend       = 16 bytes    (data address + vtable address)
CraneliftBackend   = 0 bytes     (a unit struct holds nothing)
```

A call like `backend.compile(&program)` then does: read the vtable address out of the
fat pointer, load the `compile` entry from the table, and jump to it with the data address
as `self`. That's one extra memory load and an indirect jump, compared with a normal call.
The compiler can't inline it, because it doesn't know the target when building `calcc`.
That's the cost of not knowing the type statically, and it is small enough to ignore
here (below).

### Why `dyn Backend` and not generics

Rust has two ways to write code against a trait:

- **Generics** (`fn build<B: Backend>(b: B)`): the compiler stamps out a separate copy
  of `build` per concrete type (*monomorphization*), and the choice is fixed at compile time.
- **Trait objects** (`&dyn Backend`): one copy of `build`, and the concrete type is
  looked up at run time through the vtable.

Generics are usually the faster, more idiomatic choice, so it's fair to ask why not
here. The reason is one constraint: **a generic parameter stands for exactly one concrete
type, and the compiler must know which one wherever the generic code is used.** Our
situation breaks that in three ways.

**1. The choice arrives at run time.** `--backend=llvm` is a string typed by a user
*after* `calcc` was built. Generic code is instantiated by the compiler ahead of time,
so `build::<LlvmBackend>` and `build::<CraneliftBackend>` both have to exist already, and
something has to choose between them while the program runs.

**2. A function can only return one type.** Try writing `select` with generics:

```rust
fn select(name: &str) -> impl Backend {
    if name == "llvm" { LlvmBackend } else { CraneliftBackend }
}
```

```text
error[E0308]: `if` and `else` have incompatible types
 --> select.rs:8:46
  |
8 |     if name == "llvm" { LlvmBackend } else { CraneliftBackend }
  |                         -----------          ^^^^^^^^^^^^^^^^ expected `LlvmBackend`, found `CraneliftBackend`
  |                         |
  |                         expected because of this
  |
help: you could change the return type to be a boxed trait object
  |
7 - fn select(name: &str) -> impl Backend {
7 + fn select(name: &str) -> Box<dyn Backend> {
  |
```

(This is `rustc`'s real output for that snippet, with a second `help:` suggestion that
boxes each returned value omitted.)

`impl Backend` means "one specific type that implements `Backend`, and I won't say
which". It doesn't mean "any of them". The two branches produce different types, so it
doesn't compile. (The compiler's own suggestion is a trait object.)

**3. A collection can only hold one type.** `enabled()` returns every compiled-in
backend in one list. A `Vec<B>` has a single `B`, so a `Vec<CraneliftBackend>` cannot
also hold an `LlvmBackend`. `Vec<&dyn Backend>` can, since both are the same type: a
fat pointer.

**What generics *could* do, and why we don't.** You can still use generics by pushing the
choice up to a `match` that calls a different instantiation in each arm:

```rust
match name {
    "cranelift" => build::<CraneliftBackend>(path, out),
    "llvm"      => build::<LlvmBackend>(path, out),
    …
}
```

That's essentially the code A7 had (with a function pointer in place of the generic), and
it is what A8 removes: the `match` names every backend, so adding one means editing it,
and each arm needs its own `#[cfg]`. A closed `enum AnyBackend { Cranelift(…), Llvm(…) }`
has the same problem. That is a poor fit for the extension-point goal in spec.md §8.1
("any number of backends… without touching anything upstream").

**Why the cost doesn't matter.** Generics win by letting the compiler inline calls and
optimize across them. That matters for something called millions of times. `compile` is
called once per `calcc build`, and then spends its time generating machine code. One
indirect call is invisible next to that.

The general rule: use generics when the type is known at compile time and the code is hot;
use `dyn` when the type is chosen at run time, or when you need to mix different types in
one place.

### Picking one: `backend::select`

```rust
pub fn enabled() -> Vec<&'static dyn Backend> {
    vec![
        #[cfg(feature = "backend-cranelift")]
        &crate::cranelift_backend::CraneliftBackend,
        #[cfg(feature = "backend-llvm")]
        &crate::llvm_backend::LlvmBackend,
    ]
}
```

`enabled()` is the only place that lists backends. `select(name)` searches it:

| You ran | Backends compiled in | Result |
| --- | --- | --- |
| `calcc build p.calc -o p` | just Cranelift | Cranelift (the only one is the implicit default) |
| `calcc build p.calc -o p` | both | error: `several backends are enabled; pick one with --backend=<cranelift\|llvm>` |
| `calcc build --backend=llvm …` | just Cranelift | error: ``this build has no llvm backend; rebuild with `--features backend-llvm` (enabled: cranelift)`` |
| `calcc build --backend=wasm …` | any | error: ``unknown backend `wasm` (enabled: …)`` |

The third row is why `select` knows the full list of names (`KNOWN`), not just the
enabled ones: it can tell "you typo'd" apart from "this build left that one out".
Those messages are real output from the tests in `backend.rs` and from running `calcc`.

## How Cargo features work

The other half of the deliverable is "building with only one feature enabled compiles
successfully without the other backend's dependency". That is a Cargo feature, so it's
worth understanding them properly.

### What a feature is

A **feature** is a named, compile-time on/off switch for a crate. You declare features
in the `[features]` table of `Cargo.toml`, and whoever builds the crate turns them on
or off. Nothing about a feature exists at run time; it changes *what gets compiled*.

Here is this crate's table:

```toml
[features]
default = ["backend-cranelift"]
backend-cranelift = [
    "dep:cranelift-codegen", "dep:cranelift-frontend", "dep:cranelift-module",
    "dep:cranelift-object",  "dep:cranelift-native",
]
backend-llvm = ["dep:inkwell"]
```

Read `backend-llvm = ["dep:inkwell"]` as: "the feature `backend-llvm`, when on, also
switches on the dependency `inkwell`."

### Optional dependencies

A normal dependency is always downloaded and compiled. A dependency marked
`optional = true` is not: it's only pulled in when some feature names it with `dep:`.

```toml
cranelift-codegen = { version = "0.135.2", optional = true }
inkwell = { version = "0.10", …, optional = true }
```

This is what makes the "without the other backend's dependency" promise real, not
cosmetic. You can check it: ask Cargo for the dependency tree of an LLVM-only build and
count Cranelift crates.

```text
$ cargo tree -p calc-compiler --no-default-features --features backend-llvm --prefix none | grep -ci cranelift
0
$ cargo tree -p calc-compiler --prefix none | grep -ci inkwell        # default build
0
$ cargo tree -p calc-compiler --prefix none | grep -ci cranelift      # default build
20
```

Zero crates means zero download, zero compile time, and nothing to link, not merely
"unused code".

`target-lexicon` stays a normal dependency: it isn't part of either backend. `link_stub`
uses it to find the host's linker, so gating it with Cranelift would break an LLVM-only
build. The rule of thumb is that a dependency belongs to a feature only if *only* that
feature's code uses it.

### Default features, and turning things on and off

`default = [...]` lists the features on when you don't say otherwise. From the command
line:

```bash
cargo build                                               # defaults: Cranelift only
cargo build --features backend-llvm                       # defaults + LLVM (both)
cargo build --no-default-features --features backend-llvm # LLVM only
cargo build --all-features                                # everything
cargo build --no-default-features                         # nothing: hits compile_error!
```

Which backend to default to is spec.md §14 #4's open question; the answer and its
reasoning are in `DECISIONS.md`'s A8 entry. Short version: Cranelift is pure Rust, so a
fresh clone builds with nothing else installed, while LLVM needs a C++ install first.

### Conditional compilation: `#[cfg(feature = "…")]`

A feature only matters if the *source* reacts to it. The attribute `#[cfg(feature = "x")]`
means "only include the next item if feature `x` is on":

```rust
#[cfg(feature = "backend-llvm")]
mod llvm_backend;                       // the whole file, in or out
```

It works on modules, functions, `impl` blocks, statements, even a single element of a
`vec![…]` (see `enabled()` above). A leading `#![cfg(…)]` (with the `!`) applies to the
whole file it's in: `tests/cranelift_recipes.rs` starts with
`#![cfg(feature = "backend-cranelift")]`, so an LLVM-only build skips that file entirely.

The crucial detail is that **code disabled by `cfg` is removed before type-checking**. A
disabled `llvm_backend.rs` may name `inkwell` types that don't even exist in this build
and Cargo won't complain. The flip side is that a mistake inside disabled code goes
unnoticed until someone turns that feature on, which is why CI must build every
combination you promise (see below).

### The trap: features are additive

Cargo builds a *whole graph* of crates, and if two crates in the graph ask for different
features of a third, Cargo turns on the **union** of what they asked for. This is called
*feature unification*. It means a feature can be added by someone you've never heard of
but never removed.

The consequence for design: never build two features that are mutually exclusive ("exactly
one backend"), because a downstream crate can always end up with both enabled. So A8 does
not forbid two backends; it makes both being on a normal state, resolved at run time:

- both on and no `--backend`: a clear error asking you to choose;
- none on: `compile_error!`, since a compiler with no backend can do nothing (a
  compile-time error is right here, because it's a misconfiguration rather than user
  input).

### Testing every combination

A combination that isn't built in CI will break unnoticed. There are three that matter:

| Combination | Command |
| --- | --- |
| Cranelift only (default) | `cargo test -p calc-compiler` |
| LLVM only | `cargo test -p calc-compiler --no-default-features --features backend-llvm` |
| Both | `cargo test -p calc-compiler --all-features` |

[`agent-evals.yml`](../../.github/workflows/agent-evals.yml) has one CI job per row:
`test-cranelift`, `test-llvm` and `test-all-features`. Each runs clippy, doc (with
warnings as errors) and tests for just its own features, so a failure names the
combination that broke. GitHub's `ubuntu-latest` has no LLVM, so the two LLVM jobs
install it first. A fourth job, `checks`, holds what isn't about a backend: `cargo fmt`
and `cargo audit` (formatting is source-wide, and `Cargo.lock` is a single file), plus a
plain workspace `check` and `build`.

One test needed adjusting to make the LLVM-only build work: A7's LLVM tests also
compared against the Cranelift backend's output. That comparison is now wrapped in
`#[cfg(feature = "backend-cranelift")]`, so LLVM-only builds still check LLVM against the
interpreter, and the three-way check remains when both are compiled in.

## What changed in `main.rs`

Before, dispatch was a `match` with a case per backend and a function pointer:

```rust
[cmd, backend, path, out_flag, out]
    if cmd == "build" && backend == "--backend=cranelift" && out_flag == "-o" => …
#[cfg(feature = "backend-llvm")]
[cmd, backend, path, out_flag, out] if … "--backend=llvm" … => …
#[cfg(not(feature = "backend-llvm"))]
[cmd, backend, ..] if … "--backend=llvm" => { eprintln!("no LLVM backend…") }
```

Now `build` parses the (optional) `--backend=` value and asks `backend::select` for
the backend. `main.rs` doesn't mention "cranelift" or "llvm" any more, so it can't
drift out of sync with which backends exist. `--backend` also became optional:
with one backend compiled in, it's the default.

## What's deliberately not here yet

- **`BackendError` is never produced.** Both backends still `expect(…)` (panic) on
  internal failures. The trait allows failure so its shape won't have to change later;
  converting the backends' internals is separate work with no user-visible payoff yet.
- **No options on the trait** (optimization level, target triple). Nothing needs them,
  and `DECISIONS.md` records that.
- **Still x86-only for LLVM** (`target-x86` in `inkwell`'s features), as in A7.
- **The interpreter isn't a `Backend`.** It produces an answer, not an object file, so
  spec.md §8.1 keeps it on a parallel path.

## Recap

- A `Backend` trait is an extension point: `main.rs` talks to "a backend", and a new
  backend is one `impl` plus one line in `enabled()`.
- `&dyn Backend` (run-time choice) fits because `--backend=` is user input.
- Cargo **features** are compile-time switches; **optional dependencies** are what keep
  an unwanted backend out of the build graph entirely (verified with `cargo tree`).
- `#[cfg(feature = …)]` removes code before type-checking, so every combination you
  support must be built in CI.
- Features are **additive** (unification), so backends are not mutually exclusive:
  selection among the enabled ones happens at run time.
