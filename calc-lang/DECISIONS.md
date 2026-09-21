# calc-lang decision log

## A0 — Parser frontend

Parsing will be built from the start behind a `ParserFrontend` trait (spec.md §5),
mirroring the pluggable `Backend` trait from §8.1 one layer earlier. No stage
downstream of parsing (role-driven lowering, codegen, the LSP) will depend on which
concrete frontend produced the AST — only on the trait's typed AST + role model
output.

The first concrete implementation is **LALRPOP**: it supports inline Rust actions
attached directly to grammar alternatives, which sidesteps designing a separate
action language (§6.1, §14.2) until/unless that's actually needed later. It also
handles left-recursive expression grammars naturally, which matters for calc-lang's
arithmetic expressions.

**pest** (grammar/actions kept separate) and a **hand-written recursive-descent**
frontend are planned as alternate implementations of the same trait, in sessions
A1-pest and A1-custom respectively, once A1 exists to write a shared test suite to
compare them against.

## A2 — Typed AST, and deferring `Stmt` to A3

A1's throwaway `RawAst` is promoted to a real `calc-syntax::ast::Expr`, with the
grammar's actions building it directly. `If`'s three sub-expressions moved from
positional fields (`If(Box<RawAst>, Box<RawAst>, Box<RawAst>)`) to named ones (`If {
cond, then_branch, else_branch }`), since this is meant to be the AST's permanent
shape and named fields make call sites and pattern matches self-documenting.

roadmap.md's session template describes A2's deliverable as an "`Expr`/`Stmt`"
module, but `Stmt` is deliberately not added yet: calc-lang has no construct that
isn't itself an expression-with-a-value — `if`/`else` evaluates to a branch's value
exactly like Rust's own `if` expression (A1's doc) — so there is nothing for a
statement type to represent, and no grammar rule that would produce one. An empty,
untested `Stmt` enum would be unused scaffolding, which cuts against this project's
own "don't restructure or generalize ahead of need" convention (CLAUDE.md). A3
("Scopes, symbol table") is the session that actually introduces a binding/
declaration construct to test "duplicate binding" errors against — that's the
natural, need-driven point to add `Stmt`, backed by real grammar and tests.

## A3 — Block-scoped `let` statements over ML-style `let ... in ...`

calc-lang's first binding construct is `Stmt::Let { name, value }`, usable only
inside `Expr::Block { stmts: Vec<Stmt>, result: Box<Expr> }` — `{ let x = 1; let y
= 2; x + y }` — rather than an ML-style `let x = v in body` *expression*.

The reason is that ML-style `let` makes "duplicate binding" (one of the two error
cases the roadmap names for this session) structurally unreachable: every `let`
would open its own brand-new scope containing exactly one name, so two bindings can
never land in the same scope no matter how they're nested — nested `let`s are
always shadowing, never a same-scope redeclaration. A block holding a *sequence* of
`let` statements sharing one scope is what makes redeclaring a name within that same
scope a real, testable case, distinct from shadowing across nested blocks (allowed).

This is also a backward-compatible grammar change, not a rewrite: `if`/`else`
branches already required literal `{ ... }` braces since A1. Giving `{ ... }` real
block syntax (`"{" Stmt* Expr "}"`) and using it for the `if`/`else` branches is a
strict superset — `if x { 1 } else { 2 }` still parses to the same `Expr::If` shape,
just with an empty `stmts` list on each branch's `Block`. `Block` is also added as a
plain `Term` alternative, so a block can appear as any expression (`2 + { let x = 1;
x }`), not just inside `if`/`else`.

Resolution (`calc-syntax::resolve`) is hand-written directly over `Expr`/`Stmt` — a
`Vec<HashMap<String, ()>>` scope stack pushed/popped at each `Expr::Block` — rather
than routed through the `scope_enter`/`scope_exit`/`symbol_declare`/`symbol_lookup`
built-in indirection spec.md §7.3 describes. That refactor is deliberately deferred
to the optional A17 session, once Part B's generic role-driven lowering exists to
plug a pluggable implementation into; building it by hand first here (per roadmap
A3's own framing, "role-annotation concepts... by hand") gives A17 a concrete,
tested implementation to extract behind that interface later, mirroring how A0–A2
built LALRPOP by hand before any generalization was attempted.

## A4 — Only `If` in the IR; deferring `Loop`/`Break`/`Continue`/`Return`

spec.md §8.1 describes the mid-level IR's eventual control-flow vocabulary as `If`,
`Loop`, `Break`/`Continue`, and `Return`. `calc-ir::ir::Instr` implements only `If`.
`calc-lang`'s AST has no loop or early-return construct — no grammar rule produces
one — so there is nothing for those variants to lower from. Adding them now would be
exactly the kind of speculative scaffolding A2's `Stmt` deferral already argued
against: untested enum variants nothing constructs, added because the spec mentions
them rather than because anything needs them. They'll be added in whichever future
session first gives `calc-lang` a loop or `return` construct, the same way A3 added
`Stmt` only once `let` existed to justify it.

## A4 — "Phi via copies" for `If`'s branch merge

`calc-lang`'s `if`/`else` is an *expression* — both branches must funnel into one
value the surrounding code can use — but the IR represents control flow with a
structured `Instr::If` node (two nested `Block`s), not a jump-connected CFG, so
there's no natural single point to insert an SSA `phi` the way LLVM would. Instead,
`ast_to_ir::lower` ends each branch's instruction sequence with an explicit
`Instr::Copy { dst, src: <branch's result temp> }`, where `dst` is the same `Temp`
for both branches. This is a standard, simple technique ("phi elimination via
copies") and keeps the IR's instruction set free of a `Phi` node that would only
ever appear in exactly one place (structured `If`'s merge point). It's also the one
place this IR isn't strictly SSA — `dst` has two static definition sites — which
matches spec.md §8.1's "roughly SSA-ish" phrasing rather than a strict-SSA
guarantee.

## A5 — `Temp`-indexed store instead of a name-keyed environment

roadmap.md's session template describes the interpreter's environment as
`HashMap<String, Value>`, but `calc_ir::interp` uses a `Vec<Option<Value>>` indexed
by `Temp.0` instead, with no variable names involved at all. This isn't a simplification
made for its own sake — by the time IR reaches the interpreter, source-level names no
longer exist in the data: `calc_syntax::resolve()` (A3) already rejected duplicate/
unresolved names over the AST, and `ast_to_ir::lower()` (A4) already rewrote every
`Var(name)` into a direct `Temp` reference during lowering, using its own
lowering-time-only `HashMap<String, Temp>` that never reaches the IR. A `Vec` is safe
here specifically because `lower()`'s `next_temp` counter is threaded through the
*entire* recursive lowering, including into both arms of every `If` — so every `Temp`
in a program is globally unique and densely numbered from 0, and (per A4's "roughly
SSA-ish" entry above) always written before it's read. `TempStore::read` panics on a
read of an unwritten slot rather than silently defaulting, catching a malformed-IR bug
the same way `ast_to_ir::lookup` already panics on an unresolved identifier, instead of
returning a wrong answer.

## A5 — Nonzero-is-truthy for `if`'s numeric condition

calc-lang has no boolean type — the grammar's `if <cond> { ... } else { ... }` accepts
any numeric expression as `<cond>` — so the interpreter defines truthiness as "nonzero
is true, zero is false," mirroring C's convention, since there is nothing else in the
AST or spec to check a condition against. This is purely an interpreter-level runtime
semantics decision; it doesn't require or imply adding a boolean type anywhere
upstream.

## A6 — Cranelift `Variable`/SSA-construction instead of manual phi-threading

`cranelift_backend::define_calc_main` maps every `calc_ir::Temp` to a Cranelift
`Variable` and lowers `Const`/`BinOp`/`Copy`/`If` using `declare_var`/`def_var`/
`use_var`, rather than tracking a `Temp -> cranelift Value` table and manually
threading a block parameter through `If`'s merge point (the "textbook" way to
build SSA `phi`s by hand in an IR builder). Cranelift's `Variable` mechanism exists
specifically to let a backend author write "mutable-looking" locals and have
Cranelift's own SSA-construction algorithm (Braun et al.) insert the right `phi`s
at block boundaries automatically — so `Instr::If`'s two branches each just
`def_var` the same destination `Temp`'s variable, and reading it back in the
sealed `merge` block resolves correctly with no manual plumbing. This is a direct
structural echo of A4's own "phi via copies" decision (`Instr::Copy` ending each
`If` branch, because the mid-level IR itself isn't strict SSA at that one point) —
Cranelift's `Variable` abstraction is, in effect, the same trick one layer down,
and picking it over manual block arguments means the codegen backend doesn't have
to re-solve a problem the mid-level IR already worked around.

## A6 — `main() -> i32` exit-code convention, and an explicitly provisional link stub

`cranelift_backend::compile_to_object` emits the compiled program as
`calc_main() -> f64` plus a small C-ABI `main() -> i32` that calls it and returns
`fcvt_to_sint_sat` of the result. calc-lang has no I/O and no built-ins yet (A9–A11
haven't landed), so there is no way for a compiled, linked executable to report its
answer except through some OS-visible side channel — the process's exit code is
the simplest one available, and it's exactly what this session's own tests check
against `calc_ir::interpret`'s result. This convention is expected to become
unnecessary, not be built on, once A9 lands real built-ins.

Turning the object file into a runnable executable at all requires a linker, which
is properly A12's job ("The link driver"). `crates/calc-compiler/src/link_stub.rs`
is a deliberately minimal, explicitly-labeled stand-in: it shells out to whatever
system C toolchain the `cc` crate finds (MSVC's `cl.exe`, or a Unix-style `cc`),
using only the *linking* half of what that toolchain can do. It's real enough to
make this session's tests genuinely link-and-run rather than only checking that
codegen produced *some* bytes, but it doesn't attempt A12's actual scope (assembling
FFI/IPC runtime libraries alongside the object file) and should be replaced, not
extended, when A12 is built. One MSVC-specific wrinkle fell out of this: a
hand-built object file carries none of the `/DEFAULTLIB` directives a real
`cl.exe`-compiled object would, so `libcmt.lib`/`libvcruntime.lib`/`libucrt.lib`
have to be named explicitly on the MSVC link line for `mainCRTStartup` (which calls
`main`) to resolve — Unix-style `cc`/`gcc` needed no equivalent change.

## A7 — LLVM backend is feature-gated off by default; CI doesn't exercise it

`llvm_backend.rs` is compiled only with `--features backend-llvm`, and that feature is
*not* a default feature. LLVM is a large external C++ dependency needing an install
(`LLVM_SYS_211_PREFIX`, LLVM 21.x); GitHub Actions' `ubuntu-latest` has none, and
CLAUDE.md §4 says a commit must never land something CI would reject. Alternatives
considered: (b) add an LLVM install step to `agent-evals.yml` — rejected for now because
it adds minutes and a new failure mode to every run for a backend the required path
doesn't depend on (spec.md §8.1 already wants it optional). Consequence: CI builds and
tests only the default (Cranelift) configuration, so the LLVM backend is verified locally
via `cargo {check,clippy,doc,build,test} --features backend-llvm` before committing.
Adding a CI job with LLVM installed, and settling the default-feature question
(spec.md §14 #4), is deferred to A8 alongside the `Backend` trait.

Other small choices: (1) the `If` merge `phi` is built by hand (per-branch value-table
clones plus `get_insert_block` for the predecessor), unlike A6's `Variable`s, because
LLVM's API has no equivalent and building it is the point of the session; only the
`If`'s `dst` needs one since it's the only multiply-defined `Temp` (A4's "phi via
copies"). (2) `main.rs` dispatches via a `fn(&Program) -> Vec<u8>` pointer, the minimum
that lets `build` be shared — not a `Backend` trait, which stays A8's. (3) `target-x86`
only: `Target::initialize_native` needs the host architecture's `target-*` feature, so
non-x86 hosts need one added until A8.

## A8 — Cranelift is the default backend; one `Backend` trait; LLVM gets its own CI job

**Default features (spec.md §14 #4).** `default = ["backend-cranelift"]`; `backend-llvm`
stays opt-in. Alternatives: LLVM-only (needs an LLVM install for every first build) or
both on (same problem). Cranelift is pure Rust, so a fresh clone builds with nothing
else installed, which is the friendlier first run; authors who want LLVM opt in with
`--features backend-llvm` (or `--no-default-features --features backend-llvm` for LLVM
alone). Cargo features are additive, so the two are deliberately *not* mutually
exclusive: with both enabled, `--backend=` is required (there's no implicit default among
several); with none, a `compile_error!` fires.

**The trait.** `trait Backend { fn name(&self); fn compile(&self, &Program) ->
Result<Vec<u8>, BackendError> }`, chosen at runtime through `&'static dyn Backend`
(`backend::select`) rather than generics, because which backend runs is decided by a
command-line string, not at compile time. The two `impl`s are unit structs wrapping the
existing `compile_to_object` functions. Deliberately deferred: (1) converting the
backends' internal `expect`/panics into `BackendError`s — the trait allows failure but
nothing produces it yet, and rewriting both backends' internals isn't this session's
deliverable; (2) any options on the trait (optimization level, target triple) — nothing
needs them yet. `target-lexicon` stays unconditional because `link_stub` (not a backend)
uses it, so it isn't gated with Cranelift.

**CI.** A7 deferred an LLVM CI job to A8, but roadmap.md's A8 entry doesn't list one, so
it was added here on request. `agent-evals.yml` now has a backend-agnostic `checks` job (`fmt`, `check`, `audit`,
`build`) plus one job per feature combination (`test-cranelift`, `test-llvm`,
`test-all-features`), each running clippy, doc (`-D warnings`) and tests for its own set;
the two LLVM jobs install LLVM 21 from apt.llvm.org. (`KyleMayes/install-llvm-action`'s prebuilt 21.1.1 tarball was tried first and failed to link on the Ubuntu runner: it is built against libc++, but `llvm-sys` links `-lstdc++`, giving undefined `std::__1::...` symbols. apt.llvm.org's packages are built against libstdc++.) `fmt` and `audit`
don't depend on features at all; `check`/`build` there use default features only. Separate jobs rather than one, because `#[cfg]`-gated code is
only checked when its feature is on, so each promised combination must be built on its own
and a failure should name the combination. The three feature combinations were verified locally against a real
LLVM 21.1.1 install. That did not catch the libc++/libstdc++ link failure above, which is
specific to Linux; the apt.llvm.org setup is likewise only confirmed by its CI run.
