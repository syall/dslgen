# Session A7 — LLVM codegen backend (via inkwell)

Spec refs: spec.md §8.1. Roadmap: roadmap.md "A7".

## Plan

1. Gate LLVM behind an off-by-default `backend-llvm` feature: optional
   `inkwell = { version = "0.10", default-features = false, features = ["llvm21-1", "target-x86"] }`
   in `calc-compiler/Cargo.toml`. CI (no LLVM) keeps testing the default
   configuration; the LLVM configuration is verified locally.
2. Add `src/llvm_backend.rs` with `compile_to_object(&Program) -> Vec<u8>`, mirroring
   the Cranelift backend: `calc_main() -> double` plus C `main() -> i32` using
   `llvm.fptosi.sat`. Temps are plain SSA values; `If` builds its merge `phi` by hand
   from per-branch value-table clones and each branch's final insertion block.
   `module.verify()` before emission.
3. `main.rs`: accept `--backend=llvm` (with a clear error when the feature is off);
   share one `build` function via a function pointer — no `Backend` trait (A8).
4. Reuse `link_stub::link` unchanged.
5. Tests: the four A6 programs plus a nested `if`, each checked against the
   interpreter and the Cranelift backend; a `phi` presence test; an `-O2` test.
6. Teaching doc `a7-llvm-codegen-backend.md` (incl. a dedicated `unsafe` section), README
   entry, DECISIONS.md entry, this file.
7. Verify with the full CLAUDE.md §4 sequence, default features and `--features backend-llvm`.

## Outcome

Implemented as planned. Departures / findings:

- `inkwell`'s builder constant-folds eagerly, so `2 + 3 * 4` is already `ret double 14`
  at "-O0" and constant `if` conditions are already `br i1 true`. The planned
  "optimizer folds straight-line arithmetic" test would have proved nothing, so it became
  `optimizer_collapses_a_constant_if` (phi + branch at O0, plain `ret` at O2).
- Per review, the doc was reworked the way A6's was: an `inkwell` -> LLVM C API mapping, a
  calc-ir / Cranelift / LLVM diagram (`docs/images/a7-ir-comparison.svg`), a line-by-line
  `phi` walk-through with real IR, and worked *recipes* (function, `if`/`else` + `phi`,
  call, module -> object file) in place of a per-call crash course. The recipes are built as
  `tests/llvm_recipes.rs` (feature-gated), which also records LLVM's verbatim errors for
  deliberate mistakes; the doc's "What LLVM enforces" table quotes them.
- No `unsafe` was needed in our code; the doc's `unsafe` section explains why.
- `LLVM_SYS_211_PREFIX` wasn't visible in the shell this session and was set inline.
- Real disassembly comparison (LLVM 9 bytes of code for `calc_main` vs Cranelift's ~0x75)
  is in the doc; objects are 1049 vs ~350 bytes.

Verification: the full CLAUDE.md §4 sequence (fmt, check, clippy `-D warnings`, doc
`-D warnings`, audit, build, test) passes with default features (4 calcc tests) and
check/clippy/doc/build/test pass again with `--features backend-llvm` (11 unit tests plus 11 in `tests/llvm_recipes.rs`).
`calcc build --backend=llvm` on `2 + 3 * 4` produced an executable exiting 14.
