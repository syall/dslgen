# calc-lang teaching docs

This is DSL-Generator's teaching-documentation series (spec.md §12): one page per
build session, written so someone with no prior compiler background can follow it to
understand what that stage of a real, working compiler does and why — using
`calc-lang`'s actual code as the running example, not toy snippets.

Read in order:

1. [a0-workspace-and-parser-decision.md](a0-workspace-and-parser-decision.md) — why a
   Cargo workspace split into four crates, and why the parsing layer is a trait rather
   than a hardcoded library choice.
2. [a1-parserfrontend-trait-and-lalrpop.md](a1-parserfrontend-trait-and-lalrpop.md) —
   what a parser-generator library does for you, and the `ParserFrontend` trait's
   first concrete implementation.
3. [a2-typed-ast-and-semantic-actions.md](a2-typed-ast-and-semantic-actions.md) — what
   an AST is and why it's typed per-rule, and how a grammar's semantic actions build
   one; also covers why `calc-lang` doesn't have a `Stmt` type yet.
4. [a3-scopes-bindings-and-resolution.md](a3-scopes-bindings-and-resolution.md) — what
   scopes, bindings, and a symbol table are; `calc-lang`'s first binding construct
   (`Stmt::Let` inside `Expr::Block`) and the hand-written `resolve` pass that checks
   names against nested lexical scopes.
5. [a4-mid-level-ir-and-lowering.md](a4-mid-level-ir-and-lowering.md) — what a
   mid-level IR is and why compilers use one instead of generating code straight from
   the AST; three-address code, structured control-flow nodes vs. jump-based basic
   blocks, and how `calc-ir::ast_to_ir::lower` turns an `if`-expression's two branches
   into one value ("phi via copies").
6. [a5-tree-walking-interpreter.md](a5-tree-walking-interpreter.md) — how a
   tree-walking interpreter evaluates the IR directly (and why it's useful as a
   semantics oracle for later codegen backends); why the interpreter's environment is
   a dense `Temp`-indexed array rather than a name-keyed map, since variable names are
   already gone by the time IR exists; and how `if`'s truthiness is defined with no
   boolean type in the language.
7. [a6-cranelift-codegen-backend.md](a6-cranelift-codegen-backend.md) — what a real
   codegen backend does differently from an interpreter; Cranelift IR basics (blocks,
   values, the builder); how Cranelift's `Variable`/SSA-construction mechanism avoids
   hand-writing `phi` nodes for `if`/`else`; and the first real use of the interpreter
   as a correctness oracle for a codegen backend's output.
8. [a7-llvm-codegen-backend.md](a7-llvm-codegen-backend.md) — a second backend over the
   same IR: LLVM IR basics (SSA, basic blocks, hand-built `phi` nodes); a three-way
   diagram of calc-ir vs Cranelift IR vs LLVM IR; how `inkwell`'s methods map onto
   `llvm-sys`'s C API calls; worked recipes for building a function, an `if`/`else` with a
   `phi`, a call and an object file, plus LLVM's real error messages for common mistakes;
   real Cranelift-vs-LLVM output for one program and what an optimizing backend buys;
   feature-gating a heavy dependency in `Cargo.toml`; and what `unsafe` is, why an FFI
   boundary like LLVM's C API needs it, and what that means for users of a safe wrapper.

9. [a8-backend-trait-and-feature-gating.md](a8-backend-trait-and-feature-gating.md) — the
   extension-point pattern: one `Backend` trait both codegen backends implement, and
   run-time `--backend=` selection via `&dyn Backend`; how Cargo features work (optional
   dependencies, `default`, `#[cfg(feature)]`, feature unification and why backends can't
   be mutually exclusive); testing each feature combination, including an LLVM CI job.

10. [a9-native-rust-builtins.md](a9-native-rust-builtins.md) — built-ins backed by native
    Rust functions: why `+`/`*` now lower to a generic `CallBuiltin` instruction; the new
    `calc-runtime` crate explained from scratch (`extern "C"`, `#[no_mangle]`, symbols, the
    ABI, function pointers); what a linker is and how an object file's undefined symbols are
    resolved from a static library built by `build.rs`; before/after calc-ir, Cranelift IR,
    LLVM IR and machine code for one program, plus a "call an imported function" recipe for
    each backend; what losing inlining costs.

11. [a10-c-abi-ffi-builtins.md](a10-c-abi-ffi-builtins.md) — built-ins backed by C-ABI
    FFI: `-` becomes calc-lang's second built-in, backed by a real C file; a design
    mistake (giving the interpreter a hand-duplicated Rust reimplementation instead of
    calling the real C function) caught and fixed before it shipped, and the resulting
    principle that `eval` must always call the one real implementation; why
    `calc-runtime` needs its own `build.rs` for the first time, unlike anything A9
    required; two independent builds of the same C source, one per consumer; and a real
    MSVC linker warning (a static-vs-dynamic CRT mismatch) and how it was actually fixed.

12. [a11-subprocess-ipc-builtins.md](a11-subprocess-ipc-builtins.md) — built-ins backed
    by a subprocess: `print`, calc-lang's first new syntax since `if`/`let`, and its
    first built-in with a side effect and a real external runtime dependency; why it's
    a statement rather than an expression, and a tried-and-reverted follow-up on
    whether its call should be genuinely `void` (it worked, but the value was already
    unreachable either way, so the cross-cutting complexity wasn't worth it); a new
    top-level `Program` grammar rule (distinct from `Expr`) that lets a statement
    sequence appear with no surrounding `{ }`, making `print(1); 2` a complete
    program, while every existing statement-free program keeps its exact original
    AST shape; the any-language-becomes-a-built-in tradeoff IPC makes and its cost;
    spawn-per-call as the simplest process lifecycle; why the built-in's protocol is a
    plain command-line argument rather than JSON over stdio, forced by
    `calc-runtime`'s dependency-free special build (with a diagram of exactly how
    `include_str!` embeds the script's text into both `calcc` and every compiled
    program); a surprise finding that the built-in links into *compiled* programs too,
    not just the interpreter, plus the real MSVC linker errors (missing Winsock/
    NT-native-API import libraries) and what it took to fix them; and a real
    Windows-specific gotcha (a non-functional `python3` "app execution alias") and its
    fallback fix.

More pages land as later sessions in [../../roadmap.md](../../roadmap.md) land.
