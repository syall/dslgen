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

More pages land as later sessions in [../../roadmap.md](../../roadmap.md) land.
