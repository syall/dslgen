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

## A9 — Operators lower to built-in calls; runtime library built by `build.rs`

**No call syntax; `+` and `*` are the built-ins' surface.** Roadmap A9 asks for `add`/`mul`
"callable from a `.calc` program", but calc-lang has no call syntax and the generic `dslgen`
(not calc-lang) is where call syntax belongs. Alternatives: (a) add `name(args)` to the grammar,
AST, resolver and lowering; (b) route the existing operators to the built-ins in lowering.
Chose (b): no grammar/AST/resolve change, and every existing program exercises the built-ins
through the interpreter and both backends. The machinery is deliberately call-shaped and
operator-agnostic — `Instr::CallBuiltin { dst, name, args: Vec<Temp> }`, a manifest keyed by name
and arity — so a future call-syntax frontend emits the same node; only `builtin_for` in
`ast_to_ir.rs` is calc-lang-specific. `-` and `/` have no built-in and stay inline `BinOp`s.
Cost, accepted: `+`/`*` are no longer inlined or constant-folded (LLVM `-O2` leaves both calls),
which is a real slowdown for arithmetic but irrelevant to the point of the session.

**One manifest, one crate.** `calc-runtime` holds the `#[no_mangle] extern "C"` functions and a
hardcoded `BUILTINS` table (name, linker symbol, arity, interpreter `eval`), so the interpreter
and compiled code share one implementation. The data-driven `bindings.toml` is Part B. The IR
node stores the built-in's *name* (a `String`), not a table reference, so `Instr` keeps its
`Clone`/`PartialEq` derives; backends and interpreter `lookup` it.

**Getting the functions into the executable.** Alternatives: (a) make `calc-runtime` a
`staticlib` crate and locate the artifact next to the `calcc` binary — Cargo doesn't build
staticlibs of plain dependencies, so `cargo test -p calc-compiler` would miss it;
(b) embed the runtime's source and run `rustc` at `calcc build` time — needs `rustc` on the
user's machine at every build; (c) `calc-compiler/build.rs` runs `rustc --crate-type=staticlib`
on `calc-runtime/src/lib.rs` and bakes the archive's path in via `cargo:rustc-env`. Chose (c):
works from every `cargo` entry point. Consequences: `calc-runtime/src/lib.rs` must stay
dependency-free (built by bare `rustc`); `calcc` only works while its build tree exists (the
archive is ~12 MB and not embedded); `link_stub` always passes the archive (the linker pulls
only members it needs). On MSVC the archive links with the existing CRT libraries and needed no
extra system libraries, so the planned `no_std` fallback wasn't required. A12's real link driver
replaces this.

**Position-independent code (`is_pic=true`).** The first Linux CI run linked fine but warned
`relocation against calc_add in read-only section .text` / `creating DT_TEXTREL in a PIE` for
Cranelift-built programs: with `is_pic=false` (A6's setting) each built-in call embeds the
callee's absolute address in the code (`movabsq` + `R_X86_64_64`), which a PIE executable's
loader must patch at run time. Alternatives: (a) link with `-no-pie` — drops ASLR for every
produced program, and the LLVM backend's objects didn't need it; (b) mark the callee `colocated`
so Cranelift emits a direct relative call — a bigger machine-code change, and assumes the runtime
is always statically linked, which A10 (dynamic loading is an option) may not guarantee;
(c) `is_pic=true` — one setting; the address is read from a linker-filled slot (GOT on Linux,
`.refptr.*` stub on Windows) so the code holds no absolute address. Chose (c): the smallest change
that works however the runtime is linked. Cost: one extra load per call; the call stays indirect.
A6's Recipe 4 listing and the `new_object_module` copy in `tests/cranelift_recipes.rs` still show
`is_pic=false` and were left unchanged (they describe A6's function; the recipes never link).

**Deliberately not done**: user-defined functions, call syntax, built-ins with a non-`f64`
signature, validating a manifest entry's signature against the symbol (spec.md §7's
generation-time check; Part B).

## A10 — `-` becomes the FFI built-in; `eval` must call the real implementation, never a copy

**`-` is the new built-in's surface.** Same reasoning as A9: calc-lang has no call syntax,
so `Sub` routes to a `sub` built-in the same way `Add`/`Mul` route to `add`/`mul`. `/`
(`Div`) is now calc-lang's only remaining inline `BinOp`.

**A design gap, found before it shipped: `eval` must call the real implementation, not a
rewritten copy of it.** The first draft gave `sub`'s interpreter `eval` a hand-written
Rust closure (`|args| args[0] - args[1]`) — a second implementation of subtraction,
separate from the real `calc_sub` in C. That's exactly the "two implementations can
disagree" risk `calc-runtime` was created to eliminate for kind 1 (A9: interpreter and
compiled code call the literal same `calc_add`). Reviewed and rejected before
implementation, because it's also a dead end for A11: there's no safe way to
hand-duplicate "spawn python3 and format a string" in pure Rust. Resolved principle,
expected to hold for every future built-in kind: `eval` always calls the one real
implementation. Checked against A11 specifically: `print`'s real implementation
(`ipc_runtime.rs`, spawning `python3`) is plain Rust with no ABI boundary to cross, so
wiring it into `eval` will be *easier* than this session's FFI case, not harder — no
build script, no `unsafe`. The only new cost is inherent to IPC itself, not to wiring
the interpreter to it: `calcc run --interpret` will need `python3` on PATH too, once
A11 lands.

**`calc-runtime` gains its own `build.rs` — genuinely new work, not a repeat of A9's
"compile twice" pattern.** In A9, calc-runtime's *ordinary* `cargo build` already
produces a normal rlib, and `calc-runtime`'s own `eval` closure calls `calc_add` as a
plain Rust function call — the interpreter itself only ever makes one generic
`(builtin.eval)(&args)` call, unchanged by kind — no linking step involved, since
Rust-to-Rust calls within one compilation graph need no special archive. A9's only special build was calc-compiler's manual
`rustc --crate-type=staticlib` invocation, needed solely to give `link_stub.rs` an
archive for the *generated program's* separate, non-Cargo link. That "just works
automatically" path doesn't exist for `calc_sub`: it's C, so even the interpreter,
running in the same process, can't call it without an actual link step. So
`calc-runtime/build.rs` (new; the crate's first dependency of any kind, via
`[build-dependencies] cc`) compiles `native/calc_ffi.c` and lets Cargo auto-link the
result into every ordinary consumer — the interpreter, this crate's own tests,
`calc-ir`'s tests, `calcc` itself — so `lib.rs`'s `extern "C" { fn calc_sub(...); }`
resolves for all of them. `calc-compiler/build.rs` *separately* compiles the same
source a second time (via `cc` this time, not `rustc`, since a C file is exactly what
`cc` is for) purely to hand `link_stub.rs` an archive path for the generated program's
link — this part does mirror A9, for the same reason (a path-addressable archive
outside Cargo's own linking). The two archives are named differently
(`calc_ffi` vs. `calc_compiler_calc_ffi`) so `calcc`'s own link never has two
same-named static libraries both offering `calc_sub` — likely harmless either way, but
not worth relying on.

**MSVC CRT mismatch, and why the two archives use *different* CRT settings.** The first
build printed `LNK4098` (defaultlib conflicts) — `cc` defaults to MSVC's dynamic CRT,
while `link_stub.rs` explicitly links the static CRT (`libcmt.lib` and friends) for the
generated program. Setting `.static_crt(true)` on *calc-runtime's* build fixed that link
but broke the other one: ordinary Rust binaries (`calcc`, every crate's test harness)
use rustc's own default (dynamic) CRT on `*-msvc`, so forcing static CRT there conflicts
instead, and — because a modern rustc surfaces linker warnings as the `linker_messages`
lint — `cargo clippy --all-targets -- -D warnings` would have turned that into a hard
error. Fix: `.static_crt(true)` only on *calc-compiler's* independent build (which feeds
`link_stub.rs`'s explicitly-static link), left at `cc`'s default on *calc-runtime's* own
build (which feeds ordinary Cargo-linked binaries). Two archives from the same source,
each matching its one consumer's CRT policy.

**Kind 2 needs zero new external dependencies.** A system C toolchain, needed to compile
`calc_ffi.c`, is a *build-time-only* requirement — nothing at run time, since it's
statically linked — and not even a new assumption: `link_stub.rs` has required a system
C toolchain since A6, via the same `cc::Build::get_compiler()` call it already makes to
find a linker.

**Deliberately not done**: enumerating/declaring a built-in's external dependency (e.g.
A11's eventual `python3` requirement) anywhere in the manifest — nothing would read such
a field yet, and spec.md §7.2's override layer (`bindings.toml` → `calcc.toml` → CLI
flag) is only meaningful once `bindings.toml` (Part B, B6) is real data. Also unchanged
from A9: call syntax, a data-driven manifest, signature validation, dynamic (vs. static)
FFI linking, the real link driver (A12).

## A11 — `print(<expr>);` syntax; `Stmt`, not `Expr`; a hand-rolled IPC protocol
instead of `serde_json`; backend/link support landed anyway

**`print(<expr>);`, not a bare `print <expr>` prefix.** calc-lang's third built-in kind
(subprocess/IPC, spec.md §7) needed a unary surface, unlike A9/A10's binary operators —
there's no existing operator to route through. Requiring parens (`"print" "(" <Expr>
")"` in `calc.lalrpop`) sidesteps picking an arbitrary unary-operator precedence tier,
at the cost of reading like a function call. It deliberately isn't general call syntax
in the sense A9 ruled out, though: there's exactly one hardcoded keyword, no callee name
to resolve, no arity beyond one — the same "operators are the surface" principle, just
extended to a fixed keyword instead of a symbol.

**`Stmt::Print`, not `Expr::Print` — moved after initially shipping as the latter.**
The first version of this session made `print` an `Expr`, so it could sit anywhere an
expression could (`1 + print(x)`, `let y = print(x)`) — matching roadmap.md's own
"returns it unchanged" framing read literally. Revisited on request: `calc-syntax/
ast.rs`'s own doc comment states the test this codebase already uses for `Stmt` vs.
`Expr` — `Stmt` (so far, just `Let`) is for constructs that aren't themselves
value-producing; A2 deliberately didn't add `Stmt` at all until A3 had a real one.
`print` is run for a side effect, which is a `Stmt`-shaped thing by that same test. So
`print(<expr>)` moved into `Stmt`, requiring a trailing `;` like `let` does, and
lowering moved from `lower_expr`'s top-level `match` into the block statement loop
(`Stmt::Let { .. } => ..., Stmt::Print(inner) => ...`). Consequence, named explicitly
when this was discussed: `print(<expr>)` is no longer a complete calc-lang program on
its own — `Stmt` only ever appears inside `Expr::Block`'s `{ ... }`, alongside a
mandatory trailing result expression, so it now needs `{ print(x); 0 }` or similar.
That's consistent with the rest of the language rather than a special case:
`ParserFrontend::parse` parses one `Expr`, not a separate "program" rule, and `if`/`else`
already requires braced blocks for its own branches — the same "blocks are the only
statement-sequencing construct" rule Rust function bodies use. The other side of the
tradeoff: a bare `print(x)` can no longer appear inside an arithmetic expression
(`1 + print(x)`) — only as one statement in a block's list — and multiple `print`s can
now be sequenced directly (`{ print(1); print(2); 0 }`) without threading them through
throwaway `let` bindings, which was the awkward workaround the `Expr` version left in
place.

**A new top-level `Program` grammar rule, distinct from `Expr` — the actual parser
entry point.** Once `print` needed a wrapping `{ ... }` to be a complete program at
all, that friction was worth removing at the one place it matters: the top level.
`calc.lalrpop` gains `pub Program: Expr = <stmts:Stmt*> <result:Expr> => ...`, parsed
via the new `calc::ProgramParser` (`LalrpopFrontend::parse` calls this now, not
`calc::ExprParser`); `Expr` itself drops `pub` since nothing outside the grammar calls
it directly anymore. `print(1); 2` is now a complete program, equivalent to
`{ print(1); 2 }`, without requiring the braces — the top level behaves like an
*implicit* block, the same "statement sequence, then a mandatory result" shape
`Block`'s insides already have, just without the delimiters. Backward compatibility
mattered here: `Program`'s action special-cases an empty `stmts` list to produce the
bare `result` `Expr` directly, not `Expr::Block { stmts: vec![], .. }` — every existing
program with no top-level statements (the large majority of this project's tests)
keeps its exact original AST shape; only programs that actually use top-level
statements get the `Expr::Block` wrapping, which `resolve`/`lower`/both backends
already handle correctly (nothing downstream of parsing needed to change at all).

**Considered, implemented, then reverted: making `print`'s call genuinely `void`.**
Making `print` a `Stmt` (above) left its call still computing and returning a real
`f64` under the hood — `calc_print` still returned `x` unchanged, `Instr::CallBuiltin`
still allocated a `dst` `Temp` for it, just one calc-lang's grammar never bound to a
name. Asked directly whether `print` should evaluate to anything at all, this was
built out fully: `Instr::CallBuiltin.dst` became `Option<Temp>`, `Builtin.eval`'s type
became `fn(&[f64]) -> Option<f64>`, `calc_print` changed to `extern "C" fn(f64)` (no
return), and both backends declared a genuinely return-type-less callee signature when
`dst` was `None`. It worked, end-to-end, on both backends. But asked directly whether
the returned value was ever actually *usable* first: no — `Stmt::Print`'s lowering
never inserted its result into `env` the way `Stmt::Let` does, and no grammar rule
lets a `Stmt`'s value be referenced afterward, so the value was already completely
unreachable from calc-lang in the plain `Stmt` design, void or not. The `void` version
was therefore a pure internal-ABI-correctness nicety with zero observable behavioral
difference, at the cost of touching the shared IR node every built-in lowers to, both
backends' codegen, the interpreter's dispatch, and the manifest's `eval` type —
cross-cutting complexity CLAUDE.md's own "prefer small, focused, incremental changes"
doesn't justify for an invisible benefit. Reverted in full: `dst` is back to plain
`Temp` (always allocated, just unbound for `print`), `eval` is back to `fn(&[f64]) ->
f64`, `calc_print` returns `x` unchanged again, and both backends unconditionally
declare/read an `f64` return.

**A hand-rolled protocol (plain CLI arg + captured stdout), not `serde_json` over
stdin, despite roadmap.md's own suggestion.** `calc-runtime/src/lib.rs` must compile
standalone via a bare `rustc --crate-type=staticlib` invocation with zero `--extern`
flags (`calc-compiler/build.rs`; established A9, reinforced A10) — every built-in's
real implementation has to stay dependency-free, because `rustc` follows `lib.rs`'s
`mod` declarations regardless of Cargo, with no dependency resolution available to
that raw invocation. Putting `serde_json` in `ipc_runtime.rs` and `mod`-declaring it
from `lib.rs` would break that build unconditionally, for every kind, not just IPC.
Alternatives considered: (a) a Cargo-driven build (`cargo rustc`/multiple crate-types)
replacing the raw `rustc` call, so all three kinds get real dependency access — the
correct long-term fix, but real build-system work (isolated `--target-dir`, artifact
discovery, forwarding `--target`/profile/the MSVC static-CRT flag), and squarely what
A9's own decision log already earmarked for A12 ("the link driver"); (b) `#[cfg]`-gating
`ipc_runtime` out of just the special build — forks the `BUILTINS` array (`eval` can't
be cfg'd per-element cleanly) for one built-in; (c) a separate crate for IPC logic,
reached via a real `--extern` — hits the same artifact-path-discovery problem as (a)
the moment it needs to be link-able, for no less complexity. Chose: a plain CLI
argument (`python3 -c <embedded script> <x>`, via `include_str!`) instead of a JSON
request over stdin, captured via `Command::output()` — still real
`std::process::Command`/stdio-pipe use, just no serialization library, since a single
scalar doesn't need one. **Discussed with the user**: none of the three binding kinds
should be architecturally required to stay dependency-free long-term — that's an
artifact of the current build mechanism, not a design goal, and fixing it (option (a)
above) is real, deferred, standing project direction, not scoped to any specific future
session. Whenever it lands, `ipc_runtime.rs` should switch to a genuine `serde_json`
request/response protocol specifically to demonstrate the fix actually grants built-ins
real dependency access.

**Resolving spec.md §14.5–§14.7**: protocol is a plain command-line argument plus
captured stdout (§14.5); lifecycle is spawn-per-call, matching roadmap.md's own framing
of a long-lived worker as Part C's job (§14.6); failure (spawn error or non-zero exit)
is a `panic!` — process-level abort — matching this codebase's existing style for
invariants with no error-handling story yet (§14.7).

**Backend/link support landed too, not just the interpreter — the opposite of what A10
forecast.** Because `ipc_runtime.rs` stays dependency-free, `calc_print` compiles under
the same bare-`rustc` build as `calc_add`/`calc_mul` and lands in the same archive
(`RUNTIME_LIB`) — a real, self-contained definition, architecturally like kind 1, not
like kind 2's `sub` (only forward-declared in Rust, defined in a separately linked C
file). Both `cranelift_backend.rs` and `llvm_backend.rs` already declare/call a
built-in generically from `calc_runtime::lookup(name)`'s `symbol`/`arity` — neither
hardcodes built-in names — so no backend code changed at all. One real gap surfaced
when actually linking a compiled program that calls `print`: on MSVC,
`std::process::Command`'s Windows implementation needs `ws2_32.lib` (Winsock, pulled in
by `std`'s networking code even though `print` never opens a socket), `ntdll.lib`
(native named-pipe I/O, for piping the child's stdio), and `userenv.lib`
(`std::env::home_dir`) — none of which a hand-built object file's absent
`/DEFAULTLIB` directives supply, unlike a normal `rustc`-compiled one. `link_stub.rs`
(previously just `libcmt`/`libvcruntime`/`libucrt`) now names all three explicitly;
found by hitting `LNK2019` on exactly those symbols, fixed, and verified end-to-end
(`links_and_runs_a_program_that_calls_the_ipc_builtin`, both backends, asserting the
relayed `print.py` output and the final exit code). Consequence for A12: since
`link_stub.rs` already links every built-in kind together, A12's own stated deliverable
("`calcc build` ... for programs exercising all three built-in kinds at once") is
already true as of this session — A12's real remaining content is graduating the ad hoc
bare-`rustc`/`cc` build mechanism to something durable (see the Cargo-driven-build
alternative above), not making IPC linkable, which no longer needs doing.

**A real, empirical fix, not a design choice**: `run_print_script` tries `python3`
first, falling back to `python`. Found on this project's own dev machine: `python3`
resolves to a non-functional Windows "app execution alias" stub (spawns fine, exits
9009, never runs anything) because `Command::new` doesn't execute `.cmd` shims the way
a shell does, while a real `python.exe` is also on `PATH`. Both names are tried by
spawning and checking exit status (a spawn failure alone doesn't distinguish the stub
case, which spawns successfully).
