# A11 — Built-ins, kind 3: subprocess/IPC bridge

**Session code**:
[`crates/calc-runtime/native/print.py`](../crates/calc-runtime/native/print.py),
[`crates/calc-runtime/src/ipc_runtime.rs`](../crates/calc-runtime/src/ipc_runtime.rs),
[`crates/calc-runtime/src/lib.rs`](../crates/calc-runtime/src/lib.rs),
[`crates/calc-syntax/src/ast.rs`](../crates/calc-syntax/src/ast.rs),
[`crates/calc-syntax/src/calc.lalrpop`](../crates/calc-syntax/src/calc.lalrpop),
[`crates/calc-syntax/src/lalrpop_frontend.rs`](../crates/calc-syntax/src/lalrpop_frontend.rs),
[`crates/calc-syntax/src/resolve.rs`](../crates/calc-syntax/src/resolve.rs),
[`crates/calc-ir/src/ast_to_ir.rs`](../crates/calc-ir/src/ast_to_ir.rs),
[`crates/calc-compiler/src/link_stub.rs`](../crates/calc-compiler/src/link_stub.rs).
**Spec refs**: spec.md §7 (IPC half of built-in bindings), §14.5–§14.7. **Prereqs**:
[A9](a9-native-rust-builtins.md).

A9 and A10 gave calc-lang two of spec.md §7's three built-in kinds: native Rust
(`add`/`mul`) and C-ABI FFI (`sub`). A11 adds the third — **subprocess/IPC**, where a
built-in's real work happens in an entirely separate process, possibly written in a
language with no C-ABI story at all. The new built-in, `print`, is calc-lang's first
one with an observable side effect (it prints something) rather than just a return
value, and its first genuine external runtime dependency: running a `.calc` program
that uses `print` now requires Python on the machine that runs it.

- [What changed, in one paragraph](#what-changed-in-one-paragraph)
- [Why `print` needs new syntax, and why parens](#why-print-needs-new-syntax-and-why-parens)
- [Why `print` is a statement, not an expression](#why-print-is-a-statement-not-an-expression)
- [The top-level `Program` rule: an implicit block](#the-top-level-program-rule-an-implicit-block)
- [The tradeoff IPC makes, concretely](#the-tradeoff-ipc-makes-concretely)
- [Spawn-per-call: the simplest lifecycle, and what it costs](#spawn-per-call-the-simplest-lifecycle-and-what-it-costs)
- [The protocol: a CLI argument, not JSON over stdin](#the-protocol-a-cli-argument-not-json-over-stdin)
- [A surprise: `print` links into compiled programs too](#a-surprise-print-links-into-compiled-programs-too)
- [A real Windows gotcha: the `python3` alias that isn't Python](#a-real-windows-gotcha-the-python3-alias-that-isnt-python)
- [What it costs, and what's deliberately not here](#what-it-costs-and-whats-deliberately-not-here)
- [Try it](#try-it)

## What changed, in one paragraph

A9's trick was routing existing operators (`+`, `*`) to built-in calls; A10 added a
third (`-`). `print` can't reuse that trick — there's no existing calc-lang operator
that takes one argument, and forcing it onto an arbitrary one would be confusing.
So this session adds calc-lang's first genuinely new piece of syntax since `if`/`else`
and `let`: `print(<expr>);`, a **statement** (see the next section for why), usable
inside a block's statement list the same way `let` is. Parsing it builds a new
`Stmt::Print(Expr)` AST node; resolution walks into its inner expression like any
other sub-expression; and lowering turns it into the *exact same* `Instr::CallBuiltin`
node `+`/`*`/`-` already produce — just with one argument instead of two, and with its
result left unused rather than bound to a name:

```rust
Stmt::Print(inner) => {
    let arg = lower_expr(inner, env, instrs, next_temp);
    let dst = fresh(next_temp);
    instrs.push(Instr::CallBuiltin { dst, name: "print".to_string(), args: vec![arg] });
    // `dst` is intentionally left unbound — see the next section.
}
```

Nothing downstream of that — the IR, both backends' codegen, the interpreter,
the linker's symbol resolution — needed to change at all, for the same reason A10's
page already demonstrated: none of that machinery was ever specific to *how* a
built-in is implemented, only to the generic shape "a name, an arity, a symbol."

## Why `print` needs new syntax, and why parens

A9's page settled a principle worth restating here: calc-lang deliberately has no
general call syntax (`name(args)` for an arbitrary, resolvable `name`) — that belongs
to the generic `dslgen` meta-tool (Part B), not to a hand-built example DSL. `print`
looks like it violates that, since `print(1 + 2)` reads exactly like a function call.
It doesn't, in the sense that matters: there is exactly **one** hardcoded keyword
(`"print"` is a grammar literal, not an identifier the parser resolves), with exactly
one fixed arity. Nothing about parsing `print(<expr>)` generalizes to `anything(<expr>)`
— adding a second built-in with this shape would mean adding a second grammar rule
for it, not extending a general mechanism.

The parens themselves are a narrower, more mechanical choice: calc-lang has no unary
operators yet, so there's no established precedence tier to slot a bare `print <expr>`
prefix into (does `print 1 + 2` mean `print(1 + 2)` or `print(1) + 2`?). Requiring
`(...)` sidesteps that question by making the argument's boundary explicit in the
grammar itself, the same way `(...)` already disambiguates grouped arithmetic.

## Why `print` is a statement, not an expression

The first version of this session made `print(<expr>)` an `Expr`, so it could sit
anywhere any other expression could — `1 + print(x)`, `let y = print(x)`, nested
inside `if`/arithmetic — matching, read literally, roadmap.md's framing that the
built-in "returns it unchanged." Revisiting that: `calc-syntax/ast.rs` already has a
test for what belongs in `Stmt` versus `Expr`, stated in its own doc comment. `Stmt`
(so far, just `Let`) exists for constructs that *aren't themselves value-producing* —
`A2`'s design deliberately didn't add `Stmt` at all until `A3` had a real one to add.
`print` is run for a side effect; by that same test, it belongs in `Stmt`, not `Expr`.

This has a real, visible consequence: `Stmt` only ever appears alongside a mandatory
trailing result expression — inside `Expr::Block`'s `{ stmts... result }`, and (per the
next section) at the top level too. At the point this session made `print` a `Stmt`,
before that next section's change, there was no separate "program" grammar rule yet —
`ParserFrontend::parse` parsed one bare `Expr` — so `print(1 + 2)` alone briefly stopped
being a complete calc-lang program at all; it needed a wrapping block,
`{ print(1 + 2); 0 }`, the same rule `if`/`else` already imposes on its own branches
(always braced). The next section removes exactly that top-level friction; what stays
true either way is `print`'s own tradeoff — it can now be sequenced directly
(`print(1); print(2); 0`, no braces needed after the next section's change) without
threading calls through throwaway `let` bindings, which was the only way to do that
under the `Expr` version, but it can no longer appear *inside* an arithmetic expression
like `1 + print(x)` — only as its own statement.

**A tempting follow-up, tried and reverted**: could `print`'s call itself be made
genuinely `void` — no return value at all, rather than an `f64` `calc-lang`'s grammar
happens not to bind? This was actually built out fully at one point: `Instr::
CallBuiltin`'s `dst` became `Option<Temp>`, `calc_print` changed to a real `extern
"C" fn(f64)` with no return, and both backends learned to declare a return-type-less
callee signature. It worked end-to-end. But the deciding question turned out to be
simpler: was the old, unvoid version's return value ever *usable*? No — `Stmt::Print`
never inserted its result into `env` the way `Stmt::Let` does, so nothing in
calc-lang could reference it either way, `void` or not. A change with zero observable
behavior touching the shared IR node every built-in lowers to, both backends, and the
interpreter's dispatch logic isn't worth it — reverted, back to what this section
already describes: `calc_print` still returns `x` unchanged, `Instr::CallBuiltin`
still allocates a `dst` `Temp` for `print`'s call, calc-lang's grammar just never
gives that `Temp` a name to be reached by. A useful lesson on its own: "more
technically correct" and "worth the complexity" aren't the same test, and the second
one is what actually governs a change like this.

## The top-level `Program` rule: an implicit block

Making `print` a statement (above) comes with a real cost: every program that uses
it now needs a wrapping `{ ... }`, even a program that's just one `print` call and a
result — `{ print(1); 2 }`, not `print(1); 2`. That friction is worth removing at
exactly one place: the top level. calc-lang's parser entry point changes from `Expr`
to a new rule, `Program`, with the same shape `Block`'s insides already have — a
statement sequence, then a mandatory result expression — just without the `{ }`:

```rust
// calc.lalrpop
pub Program: Expr = {
    <stmts:Stmt*> <result:Expr> => if stmts.is_empty() {
        result
    } else {
        Expr::Block { stmts, result: Box::new(result) }
    },
};
```

`LalrpopFrontend::parse` calls the generated `calc::ProgramParser` now, not
`calc::ExprParser` (`Expr` itself drops `pub`, since nothing outside the grammar
calls it directly anymore). With this, `print(1); 2` parses as a complete program —
`Stmt::Print(Expr::Number(1.0))` followed by the result `Expr::Number(2.0)` — with no
surrounding braces at all.

The one thing worth being careful about: **not every program suddenly gets wrapped
in `Expr::Block`.** If `Program` unconditionally produced `Expr::Block { stmts,
result }`, then *every* existing program — even a bare `1 + 2` with no statements at
all — would change shape, from `Expr::BinOp(..)` directly to `Expr::Block { stmts:
vec![], result: Box::new(Expr::BinOp(..)) }`. That's harmless *semantically* (an
empty-statement block evaluates to exactly its result, nothing more), but it would
silently change the exact AST every existing test and every prior session's example
asserts against. The `if stmts.is_empty() { result } else { .. }` branch in the
action above avoids that: a program with no top-level statements parses to the bare
`Expr` it always did, and only a program that actually *uses* the new capability
gets wrapped. Nothing downstream needed to change to support either shape —
`resolve`, `lower`, the interpreter, and both codegen backends already handle
`Expr::Block` correctly, since it's the same node `{ ... }` blocks have always
produced.

## The tradeoff IPC makes, concretely

Native Rust (A9) and C-ABI FFI (A10) both compile a built-in call to one native `call`
instruction, resolved by the linker, with zero per-call overhead — the built-in's code
ends up *inside* the same executable as everything else. IPC gives up that property
entirely in exchange for a much bigger one: **any language becomes usable as a
built-in**, including ones with no meaningful way to export a C-ABI symbol at all
(Python, obviously, but the same mechanism would work for a shell script, a Node
script, anything that can be invoked as a process and communicate over stdio).
`print`'s real implementation is three lines of Python:

```python
import sys
x = float(sys.argv[1])
print(f"result = {x:.2f}")
```

The cost is real and unavoidable, not an implementation detail this session could
have engineered away: a compiled `.calc` program that uses `print` now needs Python
installed on whatever machine actually *runs* it — not just the machine that compiled
it. Every other built-in so far is statically linked into the executable; this one
reaches out to the surrounding system at run time. spec.md §3 accepts this explicitly
as the price of IPC's flexibility, and this session is where that tradeoff stops being
abstract and starts being a real `python3`/`python` lookup that can fail.

## Spawn-per-call: the simplest lifecycle, and what it costs

spec.md §14.6 leaves open whether an IPC built-in spawns a fresh process per call or
talks to one long-lived worker process across many calls. `ipc_runtime::call` spawns a
brand new `python3`/`python` process **every time `print` is invoked** — the simplest
possible design, and the one roadmap.md explicitly asks for, deferring a long-lived
worker to Part C. The cost is exactly what you'd expect: process creation is slow
compared to a native call (worst-case, several milliseconds instead of nanoseconds),
so a calc-lang program calling `print` in a tight loop would spend almost all its time
spawning Python interpreters, not doing arithmetic. That's an acceptable, deliberate
starting point — nothing about the built-in's *interface* (`name`, `arity`, `eval`)
depends on which lifecycle backs it, so switching to a long-lived worker later is a
change entirely inside `ipc_runtime.rs`.

## The protocol: a CLI argument, not JSON over stdin

roadmap.md's own suggestion for this session was a JSON request/response protocol over
stdio, using `serde_json`. That ran straight into an architectural constraint A9/A10
already established and this session couldn't work around cheaply: `calc-runtime`'s
`src/lib.rs` has to compile **standalone**, via a bare `rustc --crate-type=staticlib`
invocation with zero `--extern` flags (`calc-compiler/build.rs`) — that's what turns
`add`/`mul`/`sub`'s real implementations into a linkable static archive for *compiled*
`.calc` programs, not just interpreted ones. `rustc` follows a file's `mod`
declarations regardless of whether Cargo is involved, so anything reachable from
`lib.rs` — which now includes `ipc_runtime.rs` — has to stay dependency-free. Putting
`serde_json` there would break that special build outright, for every built-in kind,
not just this one.

So `ipc_runtime::call` uses the simplest protocol that needs no serialization library
at all: `print`'s numeric argument is passed as a plain command-line argument, and
Python's script source is embedded into the Rust binary at compile time
(`include_str!("../native/print.py")`) and handed to the interpreter via `-c`:

```rust
Command::new(interpreter)
    .arg("-c")
    .arg(PRINT_SCRIPT)
    .arg(x.to_string())
    .output()
```

**What `include_str!` actually does.** It's a compiler built-in, resolved entirely at
compile time: it reads the named file (the path is relative to the source file that
calls it, so `../native/print.py` here means "next to `ipc_runtime.rs`'s parent
directory") and splices its contents in as a `&'static str` literal — as if the
file's text had been typed directly into the source. It never touches the
filesystem again after that: no `File::open`, no runtime path lookup, nothing that
could fail because a directory moved between machines. Once `rustc` finishes,
`PRINT_SCRIPT` is an ordinary compiled string constant living in the binary's data
section, indistinguishable from a string literal written by hand — and since that
compilation happens twice (`calc-runtime`'s own `cargo build`, and
`calc-compiler/build.rs`'s separate bare-`rustc` build of the same source), the text
ends up embedded in both `calcc` itself and in every `.calc` program `calcc build`
produces.

**What's actually needed at run time, and what isn't.** Because the script's text
already lives inside the running executable, `native/print.py` the *file* is never
read again after `calc-runtime` is built — a compiled `.calc` program doesn't ship
it alongside itself, and doesn't look for it on disk. What the machine *running*
that program does need is a working `python3` or `python` executable on `PATH` (see
"A real Windows gotcha" below for what happens when that's not quite true) — nothing
more. Not needed: `calc-runtime`'s source tree, a Rust toolchain, or any
serialization library (`serde_json` was never adopted here, for the reason below).

![native/print.py, a source file on disk, is read once at compile time by include_str!("../native/print.py"), producing PRINT_SCRIPT — a constant that holds the file's full text, not a path to it. That constant compiles into both calc-runtime's ordinary rlib (linked into the calcc binary, used by calcc run --interpret) and a separately-built static archive (linked by link_stub.rs into every compiled prog.exe). At run time, both binaries reach the same step: spawn python3 -c with the embedded text passed as an argument. The only real external dependency is a python3/python executable on PATH — native/print.py itself is never read again after calc-runtime is built.](images/a11-print-script-embedding.svg)

`.output()` is still real `std::process::Command`/stdio-pipe usage — it spawns the
child, pipes its stdout, and waits for it to exit — there's just no JSON envelope
wrapping the one scalar being passed back and forth. This is a deliberate,
[documented](../DECISIONS.md) departure from the roadmap's suggested technique, in the
same spirit as earlier sessions' documented deferrals (A2 didn't add an unused `Stmt`
type; A4 didn't add unused IR control-flow variants): the concrete need here is
marshaling one `f64`, and a serialization library doesn't make that simpler, only
architecturally impossible under the current build. **This is a temporary
accommodation, not a permanent design goal** — none of the three built-in kinds should
have to stay dependency-free forever; the real fix is replacing `calc-runtime`'s ad hoc
bare-`rustc` build with a proper Cargo-driven one, which is exactly what A9's own
decision log already earmarked for A12 ("the link driver"). Once that lands,
`ipc_runtime.rs` should switch to a genuine `serde_json` protocol specifically to prove
the fix actually works.

## A surprise: `print` links into compiled programs too

A10's page ended by forecasting that A11 would be interpreter-only: IPC's real
implementation would be "plain Rust with no ABI boundary to cross," easy to wire into
the interpreter, but nothing was said about compiled backends. Staying
dependency-free turned that forecast around. Because `ipc_runtime.rs` uses only `std`,
it compiles cleanly under the *same* bare-`rustc` special build as `calc_add`/
`calc_mul` — so `calc_print` is a real `#[no_mangle] extern "C" fn`, landing in the
same static archive (`RUNTIME_LIB`), architecturally identical to kind 1's built-ins
rather than kind 2's (`sub`, which is only *declared* in Rust and *defined* in a
separately compiled C file). Both `cranelift_backend.rs` and `llvm_backend.rs`
declare and call a built-in purely from `calc_runtime::lookup(name)`'s `symbol`/
`arity` — neither hardcodes which built-ins exist — so **no backend code changed at
all**, and `calcc build --backend=cranelift`/`llvm` both produce a working executable
that calls `print`.

One real gap surfaced only when actually linking such a program, though.
`std::process::Command`'s Windows implementation reaches into more of the OS than
plain arithmetic ever did — piping a child's stdio uses native named-pipe I/O, and
`std`'s general networking code (unused here, but still present) needs Winsock —
and none of that comes for free the way it would from a normal `rustc`-produced
object file, because `link_stub.rs` builds its link command by hand, one library at a
time, with no compiler-generated `/DEFAULTLIB` directives to fall back on. The first
attempt at `calcc build --backend=cranelift` on a program using `print` failed to
link with over thirty `LNK2019` unresolved-external errors — `closesocket`,
`NtCreateFile`, `WSAStartup`, `GetUserProfileDirectoryW`, and so on. All of them
trace to exactly three import libraries: `ws2_32.lib` (Winsock), `ntdll.lib` (native
named-pipe/file I/O), and `userenv.lib` (`std::env::home_dir`). Adding those three to
`link_stub.rs`'s existing MSVC link line — alongside the `libcmt`/`libvcruntime`/
`libucrt` trio A9's page already covers — fixed it completely, with no other changes
needed anywhere.

That also answers a question A12 ("the link driver") was originally scoped to solve:
`link_stub.rs` already links `RUNTIME_LIB` (native Rust + IPC) and `FFI_LIB` (C-ABI
FFI) together unconditionally on every build, so a program using all three built-in
kinds at once — A12's own stated deliverable — already works as of this session. A12's
real remaining job shifts from "make IPC linkable" to what A9's decision log already
flagged as provisional on its own terms: replacing the hand-rolled bare-`rustc`/`cc`
build mechanism with something durable (see the previous section).

## A real Windows gotcha: the `python3` alias that isn't Python

The Python interpreter's conventional name is `python3` on Unix-like systems — what
roadmap.md's own illustrative script assumes, and what this session's code tries
first. On this project's own Windows development machine, though, `Command::new
("python3")` resolved to a working *shell* command (`python3 --version` succeeds in
Git Bash) but a **non-functional** child process from Rust: it spawns successfully,
then exits with code `9009` and the message "Python was not found; run without
arguments to install from the Microsoft Store." This is a real Windows feature —
"App execution aliases" — where `python3.exe` under
`...\WindowsApps\` is a placeholder stub the OS intercepts, meant to prompt a Store
install when no real Python is registered under that exact name. A real interpreter
was on `PATH` too, just registered as plain `python`, ahead of the stub in the
directories Rust's `Command::new` actually searches (it doesn't execute `.cmd` shims
the way a shell does, which is what let the shell-level `python3` succeed while
`Command::new("python3")` didn't).

The fix is a small, two-name fallback — try `python3`, and only trust the result if
the process actually exited successfully; otherwise try `python`:

```rust
fn run_print_script(x: f64) -> String {
    let output = try_python("python3", x)
        .or_else(|| try_python("python", x))
        .unwrap_or_else(|| panic!("couldn't run python3 or python for `print`"));
    String::from_utf8(output.stdout).expect("print.py's output should be valid UTF-8")
}
```

A spawn failure alone (the interpreter genuinely isn't on `PATH`) wouldn't have caught
this — the stub spawns fine, so the fallback has to check exit status, not just
whether `Command::output()` returned `Ok`.

## What it costs, and what's deliberately not here

- **A new, unavoidable run-time dependency.** Every `.calc` program using `print`
  needs a working Python 3 on the machine that *runs* it, whether that's
  `calcc run --interpret` or a compiled executable — spec.md §3's accepted tradeoff,
  made concrete for the first time.
- **Failure is a `panic!`.** A failed spawn or a non-zero exit from the script aborts
  the whole process (spec.md §14.7) — matching this codebase's existing style for
  invariants with no error-handling infrastructure yet, not a deliberate long-term
  answer to "what should an IPC built-in do when it fails."
- **No dependency enumeration**, same as A10: nothing records "`print` needs Python"
  anywhere machine-readable yet. That's spec.md §7.2's override-chain territory
  (`bindings.toml` → `calcc.toml` → CLI flag), meaningful only once `bindings.toml`
  (Part B) is real data.
- **Still not here**: call syntax (beyond `print`'s one fixed exception), a
  data-driven manifest, generation-time signature validation, a long-lived worker
  process, and the real link driver (A12, now narrower in scope than originally
  planned — see above).

## Try it

```bash
# bash / zsh / Git Bash
cd calc-lang
cargo build
printf 'print(1 + 2); 5' > prog.calc
./target/debug/calcc run --interpret prog.calc                          # result = 3.00 \n 5
./target/debug/calcc build --backend=cranelift prog.calc -o prog && ./prog; echo $?   # result = 3.00 \n 5
```

```powershell
# Windows PowerShell
cd calc-lang
cargo build
Set-Content -NoNewline -Encoding ascii -Path prog.calc -Value 'print(1 + 2); 5'
.\target\debug\calcc.exe run --interpret prog.calc                          # result = 3.00 / 5
.\target\debug\calcc.exe build --backend=cranelift prog.calc -o prog
if ($?) { .\prog.exe }
$LASTEXITCODE   # 5
```

`print(1 + 2); 5` — no wrapping `{ }` needed, per the `Program` rule above — first
computes `3` through the ordinary `add` built-in (A9), hands it to `print` as a
statement — which prints `result = 3.00` (via the real Python script — not a Rust
reimplementation of its formatting) and discards its own return value — then
evaluates the program's real result, `5`, which becomes the final answer.
`{ print(1 + 2); 5 }` (with the braces) still works identically — `Program` and
`Block` produce the exact same `Expr::Block` shape when there are statements to
sequence. Try uninstalling/renaming Python temporarily (or setting `PATH` to
exclude it) and rerunning: both `python3` and `python` fail to resolve, and
`ipc_runtime::call` panics with a clear "couldn't run python3 or python" message,
instead of silently doing nothing.
