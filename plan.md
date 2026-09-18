# Session A6 — Cranelift codegen backend

Spec refs: spec.md §8.1. Roadmap: roadmap.md "A6".

## Plan

1. Add `cranelift-codegen`, `cranelift-frontend`, `cranelift-module`,
   `cranelift-object`, `cranelift-native`, `target-lexicon`, and `cc` to
   `calc-compiler/Cargo.toml` (codegen backends live in `calc-compiler` per
   CLAUDE.md's Workspace layout note). No version pins beyond what `cargo add`
   resolves to the latest stable line (`0.135.2` for the Cranelift crates — the
   `0.136.0-rc.1` visible in a bare `cargo search` is a pre-release, not what
   `cargo add` picks by default).

2. `crates/calc-compiler/src/cranelift_backend.rs` (new module): IR → object file
   bytes.
   - Every `calc_ir::Temp` becomes an `F64` Cranelift `Variable` via
     `FunctionBuilder::declare_var`/`def_var`/`use_var`, letting Cranelift's own
     SSA-construction machinery (Braun et al.) resolve `If`'s merge point instead
     of hand-threading block parameters — a direct fit for `Instr::Copy`'s "two
     static definition sites" (`DECISIONS.md`'s A4 "phi via copies" entry).
   - `Const`/`BinOp`/`Copy` lower straightforwardly; `If` compares the condition
     against `f64const(0.0)` with `FloatCC::NotEqual` (matches the interpreter's
     nonzero-is-truthy rule, NaN included) and branches into freshly created
     `then`/`else` blocks that each jump to one shared, later-sealed `merge` block.
   - The whole `Program` compiles to `calc_main() -> f64`; a second, tiny
     `main() -> i32` (C ABI) in the same `ObjectModule` calls it and returns a
     saturating float→int conversion of the result as the process's exit code —
     the only way to observe a compiled program's answer before A9's built-ins
     exist.
   - `pub fn compile_to_object(program: &Program) -> Vec<u8>`.

3. `crates/calc-compiler/src/link_stub.rs` (new module): an explicitly temporary
   stand-in for A12's real link driver. Uses the `cc` crate's toolchain discovery
   (driven by `target_lexicon::HOST` since there's no cross-compilation story) to
   invoke the system C toolchain purely as a linker against the one object file,
   branching on `Tool::is_like_msvc()` for `cl.exe` vs. Unix-style `cc`/`gcc`/
   `clang` argument syntax. `pub fn link(object_bytes: &[u8], output_path: &Path)
   -> io::Result<PathBuf>` returns the executable's *actual* path, since `cl.exe`
   always appends `.exe`/`.dll` to `/Fe` targets — relevant because roadmap.md's own
   `-o program` example is extensionless.

4. Extend `calc-compiler/src/main.rs`: factor the existing parse → resolve → lower
   prefix out of `run_interpret` into a shared `compile_to_ir`, and add a
   `build --backend=cranelift <path> -o <out>` CLI arm that calls
   `cranelift_backend::compile_to_object` then `link_stub::link`, printing the
   final executable path on success.

5. Tests (inline in `cranelift_backend.rs`, per A4/A5's `#[cfg(test)] mod tests`
   convention): compile straight-line arithmetic and both `if`/`else` branches (plus
   the `let`-bound-if program A4/A5 already use as their running example) through
   the real `compile_to_object` + `link_stub::link` pipeline, run the resulting
   executable, and assert its exit code against `calc_ir::interpret`'s result for
   the same source — the interpreter as this backend's correctness oracle, per
   spec.md §9.1.

6. `DECISIONS.md`: new A6 entries for the `Variable`/SSA-construction choice over
   manual phi-threading, and the `main() -> i32` exit-code convention +
   `cc`-crate link stub as deliberately provisional (pending A9/A12).

7. `calc-lang/docs/a6-cranelift-codegen-backend.md` (new teaching-doc page) + a new
   line in `calc-lang/docs/README.md`'s reading-order list.

## Outcome

Implemented as planned, with one departure discovered while getting the link step
working on Windows: a hand-built object file carries none of the `/DEFAULTLIB`
linker directives a real `cl.exe`-compiled object would, so `mainCRTStartup` (the
CRT routine that actually calls `main`) was unresolved until the static-CRT trio
(`libcmt.lib`, `libvcruntime.lib`, `libucrt.lib`) was added explicitly to the MSVC
link line in `link_stub.rs`. This is noted inline as MSVC-specific; the Unix branch
needed no equivalent change; `cc`/`gcc` already pull in the CRT/libc startup
automatically for a bare object file exposing `main`.

Also added `#[derive(Hash)]` to `calc_ir::Temp` (`crates/calc-ir/src/ir.rs`) — needed
to key a `HashMap<Temp, Variable>`, and a one-line, behavior-preserving addition to
an existing type rather than a design change.

A later double-check of this session found one more thing worth fixing:
`collect_temps` can list the same `Temp` more than once (an `If`'s `dst` is
collected once for the `If` itself and again via each branch's `Copy` sharing
that `dst` — and `program.result` is always some instruction's own `dst`, so
this happens in every program compiled by this backend, not just an edge case).
The original `vars.insert(temp, builder.declare_var(...))` loop declared a
fresh, immediately-orphaned Cranelift `Variable` on every repeat — never a
correctness bug (only the last-declared `Variable` for a given `Temp` is ever
read, since all declarations finish before `lower_block` starts reading/writing
any of them), but wasted SSA-construction bookkeeping and quietly contradicted
`VarMap`'s own doc comment. Changed to `vars.entry(temp).or_insert_with(...)`
so each `Temp` gets exactly one `Variable`, verified by re-running the full
test suite (unchanged results, as expected since behavior wasn't actually
affected).

`cargo build --workspace && cargo test --workspace` from `calc-lang/`: all tests
pass, including the 4 new `cranelift_backend` tests, which each build a real object
file, link it into an executable via `link_stub::link`, run it, and check its exit
code — genuine end-to-end verification, not just "it compiled." Manually verified
`calcc build --backend=cranelift <path> -o <out>` end-to-end for straight-line
arithmetic, both `if`/`else` branches, and the `let`-bound-if program, confirming
exit codes matched `calcc run --interpret`'s printed results in each case, and that
`link_stub` cleans up its intermediate object file and leaves only the final `.exe`.
