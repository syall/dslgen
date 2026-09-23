# DSL-Generator — Implementation Plan (Learning Path)

This is a step-by-step build order for DSL-Generator, organized as **sessions**: each
one is scoped to fit in a single sitting, lists its prerequisites, and names exactly
what you'll learn (both about compiler/language-tooling architecture and about Rust).
You can jump straight to any session as a self-contained walkthrough as long as its
prerequisites are already done — or read the plan in order as a full course.

Every session cites the spec.md section(s) it implements, so you can always cross-check
"why are we building this" against the spec.

## Why this order, not the spec's order

spec.md describes DSL-Generator top-down: a meta-tool (`dslgen`, Level 1) that
*generates* a DSL's compiler and LSP (Level 2). Building it in that order first would
mean writing a code generator before you've ever built the thing it generates — hard to
get right and hard to learn from, since you'd be debugging templating logic and compiler
logic at the same time.

Instead, **Part A builds one concrete DSL toolchain entirely by hand** — a small
calculator language (`calc-lang`, the same example spec.md §13 uses), with every piece
hardcoded rather than generated: its own parser, AST, IR, interpreter, two codegen
backends, built-ins, and an LSP. This is where nearly all the compiler-construction and
Rust learning happens, and it matches the spec's own §2 implementation philosophy
("prefer small, focused, incremental additions... even where a bigger refactor might
look more elegant in isolation").

Only once that concrete pipeline works does **Part B extract the generic parts** into
the shared `dslgen-backend`/`dslgen-lsp` libraries and build the actual `dslgen`
meta-tool that generates a `calc-lang`-shaped workspace from inputs — using Part A's
hand-written code as the reference for what "correct generated output" looks like.

**Part C** layers in the features that only make sense once the generator itself works:
retargetable/overridable built-ins, the second memory strategy, differential testing,
and the teaching-documentation series.

---

## How to read a session entry

- **Spec refs** — spec.md section(s) this session implements.
- **Prereqs** — sessions that must be done first.
- **Rust you'll learn** — the language/ecosystem concepts this session exercises.
- **Compiler/tooling you'll learn** — the conceptual material, independent of Rust.
- **Deliverable** — the concrete, testable thing you'll have when the session is done.

Every session also produces a teaching-doc page in `calc-lang/docs/` (spec.md §12),
using that session's own code as the running example — this is a first-class part of
the deliverable, not an afterthought. C5 no longer writes the documentation series
from scratch; it consolidates and orders the pages each session already produced,
and fills in anything left uncovered.

---

## Part A — Hand-build one real DSL toolchain (`calc-lang`)

No code generation anywhere in Part A. Every file is written by hand for this one DSL.
Goal: a working `calcc` (compiler) and `calc-lsp` (language server) for a small
calculator language with variables, `if`/`else`, and a couple of built-in functions.

### A0. Workspace setup & parser-library decision

- **Spec refs**: §4 (workspace layout), §5, §14.1
- **Prereqs**: none
- **Rust you'll learn**: Cargo workspaces (`[workspace] members = [...]`), crate types
  (`bin` vs `lib`), basic `cargo build`/`cargo run`.
- **Compiler/tooling you'll learn**: why a real compiler project is split into multiple
  crates instead of one `main.rs`; what a parser-generator library buys you over a
  hand-rolled lexer/parser.
- **Deliverable**: an empty Cargo workspace with the crate layout from §4 (`calc-syntax`,
  `calc-ir`, `calc-compiler`, `calc-lsp`), each crate compiling as a stub. A short
  decision note recording that parsing will be built behind a `ParserFrontend` trait
  (§5) from the start, with **LALRPOP** as the first concrete implementation
  (recommended default: it gives you real inline Rust actions for free, which
  sidesteps designing the separate DSLA action language from §6.1/§14.2 until much
  later, if ever) — pest and a hand-written recursive-descent frontend follow as
  alternate implementations of the same trait in A1-pest/A1-custom, once A1 exists to
  compare them against.

### A1. The `ParserFrontend` trait, and a LALRPOP implementation of it

- **Spec refs**: §5
- **Prereqs**: A0
- **Rust you'll learn**: trait design with an associated type (`trait ParserFrontend {
  type Ast; fn parse(&self, src: &str) -> Result<Self::Ast, Vec<ParseDiagnostic>>;
  fn role_model(&self) -> RoleModel; }`), build scripts (`build.rs`), the `lalrpop`
  crate, generated-code integration via `include!`.
- **Compiler/tooling you'll learn**: why parsing is worth abstracting behind a trait at
  all (so role-driven lowering and the LSP never need to know which frontend produced
  the AST — the same reasoning as §8.1's `Backend` trait, applied one layer earlier);
  tokens vs. grammar rules; LR(1) parsing at a practical level (what LALRPOP does with
  your grammar, why left-recursive expression grammars "just work" in it);
  precedence/associativity declarations for `+ - * /`.
- **Deliverable**: a `ParserFrontend` trait in `calc-syntax`, and a `LalrpopFrontend`
  implementing it via a `.lalrpop` grammar parsing arithmetic expressions with
  variables, numeric literals, and `if`/`else`. A throwaway test calls `parse()`
  through the trait (not the LALRPOP-generated type directly) and asserts on the
  result, so later sessions never need to care which frontend is behind it.

### A1-pest. Alternate frontend: pest (optional, proves the trait boundary)

- **Spec refs**: §5
- **Prereqs**: A1
- **Rust you'll learn**: the `pest`/`pest_derive` crates, `.pest` grammar files,
  writing tree-walking glue that turns a pest `Pairs` iterator into your AST by hand
  (since pest, unlike LALRPOP, keeps actions out of the grammar file).
- **Compiler/tooling you'll learn**: PEG parsing (ordered-choice, no ambiguity by
  construction) vs. LR(1); why a grammar/action-separated library needs DSL-Generator
  to own more of the glue than LALRPOP does; concretely, what it costs to hand-write a
  `role_model()` when there's no grammar-file attribute syntax to translate.
- **Deliverable**: a `PestFrontend` implementing the same `ParserFrontend` trait from
  A1 for `calc-lang`'s grammar, passing the exact same test suite A1 wrote against the
  trait — proof the rest of the pipeline (A2 onward) is genuinely frontend-agnostic.

### A1-custom. Alternate frontend: hand-written recursive descent (optional, proves the "bring your own parser" path)

- **Spec refs**: §5
- **Prereqs**: A1
- **Rust you'll learn**: writing a recursive-descent parser by hand over a simple
  hand-rolled lexer (`Vec<Token>` + a cursor) — no parser-generator crate involved at
  all.
- **Compiler/tooling you'll learn**: what a parser-generator library is actually doing
  for you, learned by feel from *not* having it — manual precedence climbing for
  `+ - * /`, manual error recovery/diagnostics, manually constructing the `RoleModel`
  that A1's generated frontends got via grammar-file translation. This is the session
  that most directly answers "how would I do this with no library at all."
- **Deliverable**: a third `ParserFrontend` impl, `HandwrittenFrontend`, with no
  grammar file whatsoever — just Rust — passing A1's same test suite unchanged.

### A2. Typed AST and semantic actions

- **Spec refs**: §5 (result types), §6.1
- **Prereqs**: A1
- **Rust you'll learn**: `enum`/`struct` design for tree data, `Box<T>` for recursive
  types, `#[derive(Debug, Clone)]`, pattern matching.
- **Compiler/tooling you'll learn**: what an AST is and why it's typed per-rule rather
  than "just the parse tree"; how a parser action turns concrete syntax into an
  abstract representation.
- **Deliverable**: `calc-syntax::ast` module with `Expr`/`Stmt` types; A1's frontend
  (whichever `ParserFrontend` impl you're using) now builds real `Expr`/`Stmt` values
  instead of printing debug output. A test parses `"if x { 1 } else { 2 }"` and asserts
  on the resulting AST shape — and, if you did A1-pest/A1-custom, the same test run
  against each of them proves they agree.

### A3. Bindings in the AST, scopes, symbol table, and role-annotation concepts (by hand)

- **Spec refs**: §5 (result types, for the new AST node), §6.2, §6.3
- **Prereqs**: A2
- **Rust you'll learn**: extending `ast.rs` with a binding/declaration construct (e.g.
  a `let`-style statement) — `calc-lang` has had nothing to bind until now, which is
  exactly why A2's `DECISIONS.md` entry deferred `Stmt` to this session instead of
  adding it speculatively — plus `HashMap`, `Vec`-as-stack for nested scopes, and
  `Result`-based error handling for name resolution failures.
- **Compiler/tooling you'll learn**: lexical scoping, symbol tables, binding
  resolution — implemented concretely for `calc-lang` first (identifiers, `if`/`else`
  as control flow, block scopes) so that §6.2's abstract role vocabulary
  (`#[identifier]`, `#[keyword]`, `#[control_flow]`, `#[scope]`, `#[binding]`) has a
  working concrete referent before it's generalized in Part B.
- **Deliverable**: a binding/declaration AST node and its grammar rule, filling in
  `RoleModel::scopes`/`bindings` (left empty by A1/A2 per `frontend.rs`'s own comment)
  for the first time; then a `resolve` pass that walks the AST, builds nested scopes,
  and reports "unresolved identifier" / "duplicate binding" errors, with tests for
  both.

### A4. Mid-level IR and the lowering pass

- **Spec refs**: §8.1
- **Prereqs**: A3
- **Rust you'll learn**: designing a second, flatter data model alongside the AST;
  `Vec`-based basic-block representation; `impl From`/explicit `lower()` functions.
- **Compiler/tooling you'll learn**: why compilers use an IR instead of code-generating
  straight from the AST; three-address code; structured control-flow IR nodes (`If`,
  `Loop`, `Break`/`Continue`, `Return`) vs. raw basic-block jumps.
- **Deliverable**: `calc-ir` crate with IR types and an `ast_to_ir::lower()` function;
  a test lowering the A2 sample program and asserting on the IR's shape.

### A5. Tree-walking interpreter (debug backend)

- **Spec refs**: §9.1
- **Prereqs**: A4
- **Rust you'll learn**: recursive evaluation functions, `enum`-based runtime values,
  `HashMap<String, Value>` as an interpreter environment.
- **Compiler/tooling you'll learn**: tree-walking interpretation as the simplest
  possible "does this IR mean what I think it means" check; why an interpreter makes a
  good semantics oracle for later-written codegen backends.
- **Deliverable**: `calcc run --interpret program.calc` works end-to-end (parse → AST
  → resolve → IR → interpret → print result) for arithmetic and `if`/`else` programs.

### A6. Cranelift codegen backend

- **Spec refs**: §8.1
- **Prereqs**: A4 (A5 recommended, for comparison)
- **Rust you'll learn**: the `cranelift-codegen`/`cranelift-frontend`/
  `cranelift-object` crates, `FunctionBuilder`, basic blocks and `Value`s in Cranelift's
  own IR, unsafe-free object-file emission.
- **Compiler/tooling you'll learn**: lowering a custom IR to a codegen library's IR;
  the difference between an AOT object-file backend and a JIT; calling conventions at
  a practical level.
- **Deliverable**: `calcc build --backend=cranelift program.calc -o program` produces a
  real object file for straight-line arithmetic programs (control flow can lag one
  session if needed) and, once linked (A12 stub is fine short-term), runs correctly.

### A7. LLVM codegen backend (via `inkwell`)

- **Spec refs**: §8.1
- **Prereqs**: A6 (so you have something to compare against)
- **Rust you'll learn**: `inkwell`'s builder/module/context API, `unsafe` boundaries
  around the LLVM C API, feature-gating heavy dependencies in `Cargo.toml`.
- **Compiler/tooling you'll learn**: LLVM IR basics (SSA values, basic blocks,
  `phi` nodes for `if`/`else` merges); what an "industrial-strength" backend buys you
  over Cranelift, concretely, by lowering the *same* IR to both and comparing output.
- **Deliverable**: `calcc build --backend=llvm` produces equivalent working output to
  A6 for the same test programs.

### A8. The `Backend` trait and feature-gated backend selection

- **Spec refs**: §8.1
- **Prereqs**: A6, A7
- **Rust you'll learn**: trait design (`trait Backend { fn compile(&self, ir: &Module)
  -> Result<Object>; }`), Cargo feature flags (`--features backend-llvm`), conditional
  compilation (`#[cfg(feature = "...")]`).
- **Compiler/tooling you'll learn**: the extension-point pattern that lets a compiler
  support N backends without every caller knowing which one is active; why this
  boundary is what makes "add a backend later" cheap.
- **Deliverable**: A6 and A7 refactored to both implement one `Backend` trait;
  `calcc build --backend=<name>` dispatches generically; building with only one feature
  enabled compiles successfully without the other backend's dependency.

### A9. Built-ins, kind 1: native Rust functions

- **Spec refs**: §7 (native Rust half)
- **Prereqs**: A4, A6/A7 (at least one backend)
- **Rust you'll learn**: function pointers / `extern "Rust"` linkage, exposing a Rust
  function so a linker can find its symbol.
- **Compiler/tooling you'll learn**: how a "call a built-in" IR node differs from a
  user-defined function call; direct-call codegen with zero dispatch overhead.
- **Deliverable**: `add`/`mul` built-ins declared (hardcoded list is fine here — the
  data-driven `bindings.toml` comes in Part B) and callable from a `.calc` program
  through both backends.

### A10. Built-ins, kind 2: C-ABI FFI

- **Spec refs**: §7 (FFI half)
- **Prereqs**: A9
- **Rust you'll learn**: `extern "C"` declarations, `#[no_mangle]`, linking a small C
  (or `cdylib` Rust) library via `build.rs`/`cc`.
- **Compiler/tooling you'll learn**: the C ABI as the universal interop boundary; static
  vs. dynamic linking tradeoffs (§7 default: static; dynamic linking itself is C7).
- **Deliverable**: one FFI built-in calling into a tiny linked C library, working
  through both backends.

### A11. Built-ins, kind 3: subprocess/IPC bridge

- **Spec refs**: §7 (IPC half), §14.5, §14.6
- **Prereqs**: A9
- **Rust you'll learn**: `std::process::Command`, stdio pipes, `serde`/`serde_json` for
  a simple request/response protocol.
- **Compiler/tooling you'll learn**: the "any language becomes usable as a built-in"
  tradeoff and its cost (external runtime dependency at run time — §3); spawn-per-call
  process lifecycle as the simplest starting design (long-lived worker deferred to
  Part C).
- **Deliverable**: `src/ipc_runtime.rs` shim; a `print` built-in backed by a
  small `python3` script that prints its numeric argument using an f-string (e.g.
  `print(f"result = {x:.2f}")`) and returns it unchanged, callable from a `.calc`
  program and producing correct output (calc-lang's first real output, beyond the exit code).

### A12. The link driver

- **Spec refs**: §4 (`src/link.rs`)
- **Prereqs**: A6/A7, A10, A11
- **Rust you'll learn**: invoking the system linker/`cc` as a subprocess from Rust,
  assembling the right object files/libraries into one command line.
- **Compiler/tooling you'll learn**: what actually happens between "object file exists"
  and "runnable executable exists" — symbol resolution, static linking of the IPC shim
  and FFI libs into one self-contained binary.
- **Deliverable**: `calcc build` goes all the way from `.calc` source to a runnable
  native executable, for programs exercising all three built-in kinds at once.

### A13. `calcc` CLI surface

- **Spec refs**: §11 (generated-compiler CLI)
- **Prereqs**: A5, A12
- **Rust you'll learn**: a CLI-argument crate (`clap` recommended), subcommand design.
- **Compiler/tooling you'll learn**: the shape of a real compiler CLI —
  build/run/check as distinct, composable commands.
- **Deliverable**: `calcc build`, `calcc run --interpret`, `calcc check` all working
  per §11's spec, with `--backend=<name>` threaded through.

### A14. Generated LSP server (`calc-lsp`)

- **Spec refs**: §10
- **Prereqs**: A3 (symbol model), A2 (AST)
- **Rust you'll learn**: `async`/`await` and `tokio`, the `tower-lsp` crate, JSON-RPC
  over stdio.
- **Compiler/tooling you'll learn**: the LSP protocol's core requests (diagnostics
  publish, `textDocument/semanticTokens`, `textDocument/definition`/`hover`); why
  reusing the compiler's own AST/symbol model (rather than reimplementing parsing in
  the LSP) keeps the two in sync by construction.
- **Deliverable**: `calc-lsp` binary giving live parse/resolve diagnostics, semantic
  token classification for keywords/identifiers/literals, and go-to-definition/hover,
  verified against a real editor (VS Code with a generic LSP-client config is easiest).

### A15. Cranelift-JIT hot reload

- **Spec refs**: §9.2, §14.15
- **Prereqs**: A6, A13
- **Rust you'll learn**: `cranelift-jit`, file-watching (`notify` crate), in-process
  function-pointer patching.
- **Compiler/tooling you'll learn**: JIT vs AOT compilation in practice; why
  function/module-granular hot-patching is safe for some edits and not others (e.g.
  changed data layout), and how to detect the unsafe case and fall back to a restart.
- **Deliverable**: `calcc run --hot-reload program.calc` recompiles and patches in a
  changed function on save, with a working fallback-to-restart path for a layout-
  changing edit.

### A16. Memory management, strategy 1: manual

- **Spec refs**: §7 (memory primitives as built-ins), §8.2 (manual)
- **Prereqs**: A4, A6/A7, A9
- **Rust you'll learn**: wrapping `std::alloc::{alloc, dealloc}` behind a plain native
  Rust function — no new API beyond what A9 already exercised.
- **Compiler/tooling you'll learn**: why "manual" is the simplest strategy to implement
  (closest to C) and the most restrictive for DSL authors; and, per §7's design, that
  `alloc`/`free` are **not** a special codegen path at all — they're declared as native
  Rust built-ins exactly like A9's `add`/`mul`, reusing the same "call built-in" IR node
  from A4/A9. The only thing specific to "manual" is *which* IR construct emits the
  calls (allocation sites, scope-exit points call `free`), not how the call itself
  lowers.
- **Deliverable**: `calc-lang`'s `alloc`/`free` declared alongside `add`/`mul` in the
  same built-in manifest from A9, working through both backends; a stack-only mode
  remains available as the zero-runtime-footprint option for programs that need
  neither.

### A17. Scope/symbol-table primitives as built-ins

- **Spec refs**: §6.3, §7.3
- **Prereqs**: A3, A9
- **Rust you'll learn**: nothing new beyond A9/A16's pattern, applied to a different
  call site — `scope_enter`, `scope_exit`, `symbol_declare(name, kind, type)`, and
  `symbol_lookup(name)` as plain native-Rust functions bound the same way `add`/`mul`
  were.
- **Compiler/tooling you'll learn**: A16 showed memory-management primitives are just
  built-ins, not a special codegen path; this session shows the same is true of A3's
  scope/symbol-table operations — but with a difference worth learning concretely.
  Memory-management built-ins are only ever called *inside the compiled DSL program*,
  at a time the DSL author controls; `scope_enter`/`scope_exit`/`symbol_declare`/
  `symbol_lookup` are instead called by `calc-syntax`'s own resolver, running *inside
  DSL-Generator's own tooling* — once per `calcc build`, but potentially on every
  keystroke inside `calc-lsp` for live diagnostics. Refactor A3's resolve pass to call
  these four operations through a small pluggable interface (native Rust by default)
  instead of inlining `HashMap`/`Vec` logic directly, and see firsthand why §7.3 flags
  an IPC-backed override of these four as a deliberate, editor-responsiveness-costing
  tradeoff rather than the free choice §7's general framing otherwise implies.
- **Deliverable**: `scope_enter`/`scope_exit`/`symbol_declare`/`symbol_lookup`
  declared as native-Rust built-ins alongside A9's `add`/`mul` and A16's `alloc`/
  `free`, with A3's resolve pass now calling them through that interface instead of
  direct data-structure manipulation; A3's existing tests still pass unchanged,
  proving the refactor is behavior-preserving.

At the end of Part A you have a complete, working, hand-built compiler + LSP for one
DSL — everything spec.md's architecture calls for at Level 2, built without Level 1.

---

## Part B — Generalize into the `dslgen` meta-tool

Part B's job is mechanical in spirit but conceptually the heart of the project: take
the hand-written `calc-lang` code from Part A and turn it into *generated* code, driven
by inputs (grammar + role annotations + `bindings.toml`) instead of hardcoded facts.

### B1. Audit: generic vs. DSL-specific

- **Spec refs**: §4 (last paragraph)
- **Prereqs**: all of Part A
- **Rust you'll learn**: nothing new — this is a design/reading session.
- **Compiler/tooling you'll learn**: how to identify a library boundary inside code
  that wasn't written with one in mind; the actual line spec.md draws between shared
  infrastructure (IR, `Backend` impls, interpreter, LSP scaffolding, IPC shim) and
  per-DSL generated code (grammar, AST, lowering, role annotations, builtin decls).
- **Deliverable**: a short written map of every file from Part A into "generic
  (→ `dslgen-backend`/`dslgen-lsp`)" or "per-DSL (→ generated per project)".

### B2. Extract `dslgen-backend`

- **Spec refs**: §4, §14.11
- **Prereqs**: B1
- **Rust you'll learn**: multi-crate refactors, path dependencies
  (`{ path = "../dslgen-backend" }`), crate versioning basics (even if unpublished).
- **Compiler/tooling you'll learn**: nothing new conceptually — this is the "libstd
  analogy" from §4 made real: every generated workspace depends on this crate instead
  of reimplementing it.
- **Deliverable**: `Backend` trait, both backend impls, the interpreter, and
  `ipc_runtime.rs` moved into a standalone `dslgen-backend` crate; `calc-compiler` from
  Part A now consumes it as a dependency and still passes all its existing tests.

### B3. Extract `dslgen-lsp`

- **Spec refs**: §4, §10
- **Prereqs**: B1, A14
- **Rust you'll learn**: same extraction pattern as B2, applied to `tower-lsp`
  scaffolding.
- **Compiler/tooling you'll learn**: separating protocol plumbing (generic) from
  DSL-specific hookup (what counts as a keyword, how to resolve a symbol) — the seam
  that makes "every DSL gets an LSP for free" actually work.
- **Deliverable**: `dslgen-lsp` crate providing the `tower-lsp` server scaffolding
  parameterized by a small trait (e.g. `LanguageModel`) that `calc-lsp` implements.

### B4. Role-annotation parsing and the `RoleModel` type

- **Spec refs**: §5, §6.2
- **Prereqs**: B1, A3, A1
- **Rust you'll learn**: designing a small attribute/annotation syntax and parsing it
  as an extension to the LALRPOP grammar file (for the generated-frontend case),
  designing the standalone `RoleModel` data type every `ParserFrontend` impl reports
  (for the custom-frontend case), validation logic.
- **Compiler/tooling you'll learn**: turning the *concept* from A3 (identifiers,
  keywords, control flow, scopes, bindings) into *data* DSL-Generator reads, plus the
  consistency checks §6.2 calls for (no rule both `#[identifier]` and `#[keyword]`,
  etc.) — and, per §5, making sure this validation runs identically whether the role
  tags came from translating a LALRPOP/pest grammar file's attributes or from a
  hand-written frontend's own `role_model()` implementation, since downstream
  consumers (B5's lowering, the LSP) only ever see the resulting `RoleModel`.
- **Deliverable**: a parser + validator producing a `RoleModel` from LALRPOP
  grammar-file attributes, tested against both a valid and a deliberately-inconsistent
  annotation set; if you did A1-pest/A1-custom, the same validator also accepts a
  `RoleModel` supplied directly (no grammar file to parse), proving the two paths
  converge on one shared type.

### B5. Generic, annotation-driven lowering

- **Spec refs**: §6.3, §8.1
- **Prereqs**: B4, A4
- **Rust you'll learn**: table-driven / generic dispatch in Rust (matching on role tags
  rather than hardcoded rule names).
- **Compiler/tooling you'll learn**: the actual mechanism behind "any
  `#[control_flow(if)]`-tagged rule lowers to the IR's `If` node the same way,
  regardless of DSL" — generalizing A4's lowering pass so it's driven by roles instead
  of by knowing `calc-lang`'s grammar by name.
- **Deliverable**: A4's lowering pass rewritten to consume role annotations from B4
  generically; re-running it on `calc-lang`'s own grammar produces the same IR as
  before (regression test against A4/A5's output).

### B6. `bindings.toml` parsing and validation

- **Spec refs**: §7 (manifest), §7.1 excluded for now (that's C1)
- **Prereqs**: B1, A9–A11, A16, A17
- **Rust you'll learn**: `serde` + `toml` crate for structured config parsing,
  designing a validation pass with good error messages.
- **Compiler/tooling you'll learn**: replacing A9–A11's hardcoded built-in lists — and
  A16/A17's, which follow the exact same "declare it, don't hardcode it" pattern —
  with a real manifest format; symbol/signature validation for native+FFI, and
  executable/protocol validation for IPC, surfaced at generation time per §4 step 3.
- **Deliverable**: a `bindings.toml` parser/validator; feeding it `calc-lang`'s
  built-ins from Part A — the author-declared ones (A9–A11) and the memory/scope
  primitives (A16/A17) alike — reproduces the same generation output as the hardcoded
  version.

### B7. The code generator itself

- **Spec refs**: §4, §5 (last bullet), §11 (`dslgen build`)
- **Prereqs**: B2–B6
- **Rust you'll learn**: code-generation techniques — either string/template-based
  (simplest to start) or `syn`/`quote`-based AST-level generation for the Rust glue
  code; file-system scaffolding (`std::fs`, walking a template directory).
- **Compiler/tooling you'll learn**: this is the session where "meta-tool that
  generates a compiler" stops being abstract — templating `calc-syntax`/`calc-ir`/
  `calc-compiler`/`calc-lsp`'s per-DSL parts (parser glue, AST, lowering, CLI wiring)
  parameterized by grammar + role annotations + bindings, using Part A's hand-written
  files as the literal reference for what correct output looks like. Per §5, this
  session also has to branch on frontend choice from `dslgen.toml`: for `lalrpop`/
  `pest`, generate the grammar-file glue and translate its role annotations into a
  `RoleModel`; for `custom`, generate nothing for parsing at all — just copy the
  author-supplied `ParserFrontend` module into the workspace at the same slot a
  generated frontend would occupy.
- **Deliverable**: running the generator against `calc-lang`'s own grammar +
  annotations + `bindings.toml` produces a workspace that, once `cargo build
  --release`'d, behaves identically to Part A's hand-written `calcc`/`calc-lsp` — and
  switching `dslgen.toml`'s frontend from `lalrpop` to `custom` (pointing at A1-custom's
  hand-written module, if you did that session) regenerates a workspace that behaves
  identically too, with no changes needed anywhere downstream of parsing.

### B8. `dslgen` CLI: `new` / `check` / `build`

- **Spec refs**: §11 (meta-tool CLI)
- **Prereqs**: B7
- **Rust you'll learn**: same CLI-crate skills as A13, applied to the meta-tool.
- **Compiler/tooling you'll learn**: the three meta-tool verbs from §4 step-by-step —
  `check` runs validation only (§4 steps 1–3) without generating anything, `build` runs
  validation then generation then `cargo build --release`.
- **Deliverable**: `dslgen new calc-lang`, `dslgen check`, `dslgen build
  --backends=llvm,cranelift` all working against a scaffolded project directory.

### B9. Prove genericity: a second DSL

- **Spec refs**: §13 (validates the whole walkthrough), implicitly all of §4–§10
- **Prereqs**: B8
- **Rust you'll learn**: golden-file/snapshot testing patterns for generated code.
- **Compiler/tooling you'll learn**: the real test of a code generator isn't "does it
  reproduce the one example I built it against" — it's "does it work on something new
  I didn't special-case." A small second DSL (e.g. a tiny state-machine or config-
  validation language) with a different shape of control flow/bindings is the actual
  proof that B4–B7 generalized correctly rather than accidentally staying
  `calc-lang`-specific.
- **Deliverable**: a second working generated DSL toolchain, plus a regression test
  suite that runs `dslgen build` against both DSLs' inputs and checks the output
  compiles and passes each DSL's own sample-program tests.

### B10. Prove the frontend boundary through the generator, not just by hand

- **Spec refs**: §5
- **Prereqs**: B7, B9; A1-pest and/or A1-custom (at least one, to have something to
  switch to)
- **Rust you'll learn**: nothing new — this session is about exercising B7's branching
  logic, not writing new Rust concepts.
- **Compiler/tooling you'll learn**: A1/A1-pest/A1-custom proved the `ParserFrontend`
  trait boundary by hand; B7 taught the generator to branch on `dslgen.toml`'s
  frontend key; this session is the point where those two facts get tied together —
  confirming `dslgen build` produces a working `calcc`/`calc-lsp` for *both* DSLs from
  B9 under at least two different frontend choices each (e.g. `calc-lang` via LALRPOP
  and via the hand-written frontend), with no code changes outside `dslgen.toml`.
- **Deliverable**: a regression matrix (DSL × frontend) in the B9 test suite, proving
  "bring your own parser" is a real, generator-level capability and not just something
  that happened to work in Part A's hand-written code.

---

## Part C — Advanced features and polish

These build on a working `dslgen`, and can be done in roughly any order relative to
each other (dependencies noted per-session).

### C1. Retargetable built-in implementations

- **Spec refs**: §7.1, §14.17
- **Prereqs**: B6
- **Rust you'll learn**: pattern-matching/wildcard logic for target-selector strings.
- **Compiler/tooling you'll learn**: separating a built-in's *interface* from its
  *implementation*; multi-target manifests and selection precedence (needs a concrete
  decision on §14.17's open question — recommend "most-specific-literal-match wins,
  ties broken by declaration order").
- **Deliverable**: a built-in with two `[[builtin.impl]]` blocks (e.g. `default` +
  `wasm32-*`), with a test proving the right one is selected per target.

### C1-wasm. A working `wasm32` target (optional, gated on §14.19)

- **Spec refs**: §7.1, §8.1 (`wasm32` bullet), §3 (cross-compilation exception),
  §14.13, §14.19
- **Prereqs**: A7/A8 (LLVM backend), A12 (link driver), C1
- **Rust you'll learn**: `inkwell`'s WebAssembly target (enabling its
  `target-webassembly` feature alongside `target-x86`), building a Rust crate for
  `wasm32-*` targets, embedding a wasm runtime (e.g. `wasmtime`) in a test harness.
- **Compiler/tooling you'll learn**: what changes when the target isn't the host —
  a different object format, a different linker (`wasm-ld` instead of the system C
  toolchain, as a third path in A12's link driver), and no operating system
  underneath: FFI built-ins become imported host functions, and the IPC kind has no
  way to spawn a process on `wasm32-unknown-unknown` (a WASI target may relax that).
  Why Cranelift can't take part (it consumes wasm, it doesn't emit it).
- **Deliverable**: first, a recorded answer to §14.19 (does v1 ship this?); if yes,
  `calcc build --backend=llvm --target=wasm32-...` producing a `.wasm` module for a
  program using a native-Rust built-in and C1's `wasm32-*` implementation of an FFI
  one, run under a wasm runtime in a test with the correct result — and a clear
  build-time error for any built-in whose only implementation is IPC.

### C2. External override configuration

- **Spec refs**: §7.2, §14.18
- **Prereqs**: C1 (or B6 alone, if you skip multi-target selection)
- **Rust you'll learn**: layered config resolution (defaults → file → CLI flags),
  `std::env` for optional env-var overrides.
- **Compiler/tooling you'll learn**: why a compiled artifact's built-in *locations*
  need to stay externally configurable without regenerating or recompiling anything —
  the precedence chain from §7.2.
- **Deliverable**: `calcc.toml` overrides and `--builtin-path` CLI flags both working,
  with a test proving the documented precedence (CLI > file > manifest default) —
  plus, since §7.3/§14.21 flag it as a real concern rather than a hypothetical one, a
  check of what actually happens if `symbol_lookup` gets overridden to a
  subprocess/IPC implementation this way. Whether `calcc`/`calc-lsp` should warn or
  outright refuse that is still open (§14.21); this session is where you'd wire in
  whichever answer the project settles on.

### C3. Second memory-management strategy

- **Spec refs**: §7 (memory primitives as built-ins), §8.2 (ownership/borrow-checked
  or GC — pick one), §14.9, §14.10
- **Prereqs**: A16, B5 (needs generic lowering to add strategy-specific passes
  cleanly), B6 (bindings.toml is now data-driven, not A9's hardcoded list)
- **Rust you'll learn**: depends on choice — either implementing a small borrow-checker
  pass, or integrating/writing a simple mark-sweep collector and emitting GC-aware
  allocation calls.
- **Compiler/tooling you'll learn**: why memory strategy is "a second axis alongside
  codegen backend, not an independent concern" (§8.2) — this strategy needs its own
  lowering support in *both* the Cranelift and LLVM backends from Part A, which is the
  concrete lesson in why that matrix is designed together rather than bolted on. As in
  A16, the strategy's own primitives (`drop` for ownership/borrow-checked; `gc_alloc`/
  `gc_collect`/`gc_safepoint` for GC) are declared as built-ins through B6's
  data-driven manifest, not hardcoded into either backend — the borrow-check or GC
  lowering pass's only job is to insert the right built-in-call IR nodes at the right
  points, exactly like A16's automatic `free` insertion.
- **Deliverable**: the second strategy selectable via `dslgen.toml`, with its required
  built-in names declared in `bindings.toml` (native Rust by default, per §8.2),
  working through both backends, with a test DSL program that would behave differently
  under manual vs. this strategy (e.g. one relying on automatic cleanup) — and, as a
  bonus check on §7.1/§7.2, one of those primitives (e.g. `gc_alloc`) successfully
  retargeted to an alternate implementation via `calcc.toml` without touching
  generated code.

### C4. Differential testing: interpreter as oracle

- **Spec refs**: §8.1 (last bullet), §9.1
- **Prereqs**: A5, A8
- **Rust you'll learn**: property-based/fuzz-style test harnesses (e.g. `proptest`),
  or a simpler hand-rolled "run N sample programs through all execution paths and diff
  results" harness.
- **Compiler/tooling you'll learn**: using a simple, obviously-correct execution path
  (the interpreter) to catch bugs in more complex ones (the codegen backends) —
  a standard, high-leverage compiler-testing technique.
- **Deliverable**: a test suite running each sample `.calc` program through the
  interpreter and both backends, asserting identical results.

### C5. Teaching documentation series: consolidation pass

- **Spec refs**: §12, §14.16
- **Prereqs**: whichever sessions you're documenting (naturally trails behind the rest)
- **Rust you'll learn**: `cargo doc`, if generating an additional API-reference view
  alongside the teaching pages.
- **Compiler/tooling you'll learn**: nothing new conceptually — every session since A0
  already wrote its own `calc-lang/docs/` page as part of its deliverable (per the
  policy note above "How to read a session entry"), so this is a reading/editing pass:
  checking the series actually reads as one coherent walkthrough end to end rather than
  N independent pages, fixing cross-references, and writing any page a session skipped.
- **Deliverable**: the full `calc-lang/docs/` series, checked and reordered so it walks
  a reader through grammar → parsing → semantic analysis → IR → codegen → linking → a
  working binary per §12's stated structure, plus an index page linking them in that
  reading order.

### C6. Decision log for §14's open questions

- **Spec refs**: §14 (all)
- **Prereqs**: none strictly, but most decisions will already be made implicitly by
  the time you reach this point
- **Deliverable**: a short ADR-style log recording which way each of §14's 21 open
  questions was actually resolved during the build (most will already be decided by
  earlier sessions' "recommended default" choices, and by `calc-lang/DECISIONS.md`'s
  per-session entries) — useful both as a record and as a sanity check that nothing
  was left silently ambiguous.

### C7. Dynamic linking for FFI built-ins

- **Spec refs**: §7 (FFI linking model), §7.2
- **Prereqs**: A12 (link driver), B6 (the manifest gains the `link` field), C2
  (location overrides, which dynamic linking makes useful without re-linking)
- **Rust you'll learn**: building a `cdylib`, platform-conditional linker arguments
  (`-Wl,-rpath,...` vs. Windows import libraries), locating a shared library next to
  an executable in tests.
- **Compiler/tooling you'll learn**: static vs. dynamic linking in practice rather
  than as A10's tradeoff discussion — shared libraries, import libraries (`.lib` vs.
  `.dll` on Windows), how the system loader finds a library at program startup
  (rpath, the executable's directory, `LD_LIBRARY_PATH`/`PATH`), and why a dynamically
  linked library turns into a *run-time* dependency the link driver must report, just
  like an IPC built-in's interpreter.
- **Deliverable**: an FFI built-in declared with `link = "dynamic"` in
  `bindings.toml`, linked by `calcc build` against its shared library and reported as
  a run-time dependency; a test proving the compiled program picks up a *different*
  build of that library (e.g. swapped via C2's override, or just replaced on disk)
  without re-running `calcc build`. Static stays the default.

---

## Suggested minimum path

If you want the shortest path to "I understand how the whole thing fits together" before
committing to every session: **A0 → A1 → A2 → A3 → A4 → A5 → A6 → A8 → A9 → A13**, then
**B1 → B2 → B4 → B5 → B7**. That's parsing, AST, scopes, IR, interpreter, one codegen
backend, the backend abstraction, one built-in kind, a CLI, then the full
generalize-into-a-generator arc. Everything else (second backend, FFI/IPC, LSP, hot
reload, memory strategies, retargeting, overrides, dynamic linking, docs) layers on
afterward in any order you like — including **A1-pest**, **A1-custom**, **A17**,
**B10**, and **C1-wasm**, which are entirely optional side branches: skip them if you're
fine taking "the parser layer is pluggable", "memory/scope primitives are built-ins
too", and "the retargeting model reaches wasm" on faith, do them if you want to see
pest, a hand-written recursive-descent parser, the scope/symbol-table refactor, and a
real `wasm32` build actually happen.
