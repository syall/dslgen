# A0 — Workspace setup & parser-library decision

**Session code**: [`calc-lang/Cargo.toml`](../Cargo.toml),
[`calc-lang/crates/`](../crates), [`calc-lang/DECISIONS.md`](../DECISIONS.md).
**Spec refs**: spec.md §4, §5, §14.1.

This is the first session in Part A of `roadmap.md`: hand-building one real DSL
toolchain, `calc-lang`, entirely by hand — no code generation anywhere yet. A0
doesn't write any compiler logic. It answers two questions that every later session
depends on the answer to: *how is the code organized*, and *what will parse source
text*.

## Why a compiler isn't one file

A compiler that turns source text into a running program is usually built as a
pipeline of stages, each one taking the previous stage's output and producing
something more refined:

```
source text -> parse -> AST -> semantic analysis -> IR -> codegen -> executable
```

You could write all of that in a single `main.rs`. Most small, ad hoc language
projects do. But `calc-lang` isn't the only thing this project is building — per
spec.md §4, the whole point of DSL-Generator (the meta-tool built in Part B) is to
*generate* a workspace shaped exactly like this one, for DSLs the author hasn't
written yet. That only works if the pieces that are generic (an IR, a codegen
backend, an interpreter, LSP scaffolding) are physically separable from the pieces
that are specific to one DSL's grammar. A0 draws those lines now, by hand, before
any of the content exists to blur them.

## The four crates

```
crates/calc-syntax/      parser frontend, AST types, role/symbol model
crates/calc-ir/          lowering: AST -> mid-level IR
crates/calc-compiler/    bin: calcc — the compiler CLI, codegen backends
crates/calc-lsp/         bin: calc-lsp — the language server
```

Two of these (`calc-syntax`, `calc-ir`) are **library crates** (`src/lib.rs`) —
they don't do anything on their own, they exist to be depended on. The other two
(`calc-compiler`, `calc-lsp`) are **binary crates** (`src/main.rs`) — they're the
actual programs a user runs.

The dependency direction matters: `calc-compiler` depends on both `calc-syntax` and
`calc-ir` (it needs to parse a program and lower it before it can generate code from
it), while `calc-lsp` depends only on `calc-syntax` (an editor wants diagnostics and
go-to-definition, which only need parsing and the symbol model — not codegen at
all). That's not an accident of this session; it's the concrete reason `calc-syntax`
exists as its own crate rather than being folded into `calc-compiler`. A `cargo
build` of `calc-lsp` alone never needs to compile any codegen-backend code, which
matters a lot later (session A6/A7) once those backends pull in heavyweight
dependencies like LLVM.

This is also a rehearsal for Part B: when the shared, non-DSL-specific pieces get
extracted into `dslgen-backend`/`dslgen-lsp` libraries (sessions B2/B3), the seam
they get pulled out along is exactly the seam A0 already drew.

## Cargo workspace mechanics

[`calc-lang/Cargo.toml`](../Cargo.toml) is a **workspace** manifest: `[workspace]`
with a `members` list, and `resolver = "2"` (the modern dependency-resolution
algorithm — always specify it explicitly on a new workspace). `[workspace.package]`
holds settings (`edition`, `version`, `license`) that member crates inherit via
`edition.workspace = true` instead of repeating them four times.

Each member crate is otherwise an ordinary `Cargo.toml` + `src/`. `calc-compiler`'s
manifest uses an explicit `[[bin]]` table (`name = "calcc"`) because its *package*
name (`calc-compiler`, matching the crate directory) differs from the *binary* name
the built compiler should actually be called (`calcc`, per spec.md §4's generated
layout). `calc-lsp`'s package and binary name are the same, so no override is
needed.

## Deciding the parser strategy before writing any parser

spec.md §5 makes parsing a **pluggable frontend** — a `ParserFrontend` trait any
parsing strategy can implement, rather than DSL-Generator committing every DSL to
one hardcoded parsing library. A0 records the first concrete choice in
[`DECISIONS.md`](../DECISIONS.md): build behind that trait from day one, with
**LALRPOP** as the first implementation.

Why decide this now, before A1 writes any grammar code? Because it changes what A1
even needs to build. A parser-generator library exists to do the tedious, easy-to-
get-subtly-wrong part of a compiler for you: turning a grammar description into
actual lexing and parsing code, handling operator precedence/associativity, and
(for a mature library) years of production hardening on edge cases a hand-rolled
parser would take a long time to discover on its own. LALRPOP specifically was
picked over pest (spec.md's other built-in option) for one concrete reason: LALRPOP
lets a grammar rule's action be *literal inline Rust*, which overlaps with what
spec.md §6.1 calls "semantic actions" closely enough that A1 doesn't need to design
a separate action language (working name DSLA in the spec) just to get arithmetic
expressions parsing. pest keeps grammar and actions in separate files, which is
cleaner in one sense but means DSL-Generator would have to own more of the glue
that turns a parse tree into an AST — a cost worth paying later (session A1-pest),
not on the first pass.

## What "each crate compiling as a stub" means

At the end of A0, all four crates compile and the two binaries run — but they don't
*do* anything yet:

```
$ cargo run -p calc-compiler --bin calcc
calcc: not yet implemented
$ cargo run -p calc-lsp
calc-lsp: not yet implemented
```

That's the deliberate scope of this session: prove the workspace shape and the
toolchain are both sound before any real logic exists, so every session after this
one is adding to a structure that's already known to build.
