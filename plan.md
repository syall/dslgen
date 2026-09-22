# Session A10 — Built-ins, kind 2: C-ABI FFI

Spec refs: spec.md §7 (FFI half). Roadmap: roadmap.md "A10". Prereqs: A9.

## Plan

1. `crates/calc-runtime/native/calc_ffi.c`: a tiny C library defining
   `calc_sub(a, b) = a - b`.
2. `calc-runtime` gains its own `build.rs` (`cc::Build::compile`), so Cargo auto-links
   `calc_sub` into every ordinary consumer (interpreter, this crate's and `calc-ir`'s
   tests, `calcc` itself). `lib.rs` declares
   `extern "C" { fn calc_sub(a: f64, b: f64) -> f64; }` and `"sub"`'s `eval` calls it
   via `unsafe` — the real implementation, not a Rust reimplementation of subtraction
   (a design gap found and fixed during plan review; see Outcome).
3. `calc-runtime`'s manifest gains `pub enum BindingKind { NativeRust, Ffi }` and a
   `kind` field on `Builtin`; `add`/`mul` tagged `NativeRust`, new `"sub"` entry tagged
   `Ffi`.
4. `calc-compiler/build.rs` independently compiles the same C source a second time
   (via `cc`, named `calc_compiler_calc_ffi` to avoid a same-named archive), purely to
   expose `CALC_FFI_LIB` for `link_stub.rs` — mirrors A9's own second build of
   `calc-runtime.rs`, for the same reason (`link_stub.rs` is a non-Cargo link).
5. `link_stub.rs` links `FFI_LIB` alongside `RUNTIME_LIB`; its symbol-export test
   splits by `BindingKind` (native-Rust symbols checked against `RUNTIME_LIB`, FFI
   symbols against `FFI_LIB`); its integration test extends to `"(1 + 2) * 4 - 3"`
   (exit code 9) to exercise both archives in one link.
6. `ast_to_ir::builtin_for`: `Sub => Some("sub")`; only `Div` stays inline. Updates the
   two tests that hardcoded the old add/mul-only split, and the "every `Instr` kind"
   test's sample program (swaps its inline-`BinOp` example from `-` to `/`, since `-`
   no longer produces one).
7. Backend mixed-op tests' doc comments/expressions updated (`"(1 + 2) * 4 - 6 / 2"`,
   still 9) so they keep exercising an inline `BinOp` now that `-` is a builtin.
8. `CLAUDE.md` workspace layout, `DECISIONS.md` A10 entry,
   `calc-lang/docs/a10-c-abi-ffi-builtins.md` + README entry.

## Outcome

Implemented as planned, with two real departures surfaced during review/implementation
and recorded in DECISIONS.md:

- **Design gap, caught before implementation.** The first draft's `eval` for `"sub"`
  was a hand-written Rust closure (`args[0] - args[1]`) — a second implementation of
  subtraction, reintroducing the "two implementations can disagree" risk `calc-runtime`
  was built to avoid in A9. Fixed by giving `calc-runtime` its own `build.rs` so `eval`
  calls the real linked `calc_sub` instead. This is genuinely new work, not a repeat of
  A9's pattern: A9 never needed calc-runtime to have a build script, since Rust-to-Rust
  calls within one Cargo build need no linking step at all.
- **MSVC CRT mismatch, found while testing.** `cc`'s default (dynamic CRT) archive
  conflicted with `link_stub.rs`'s explicit static-CRT link (`LNK4098`). Setting
  `.static_crt(true)` on *calc-runtime's* build fixed that but broke the *other*
  consumer (ordinary Cargo-linked binaries expect the dynamic CRT rustc defaults to on
  `*-msvc`) — which, via rustc's `linker_messages` lint, would have failed
  `cargo clippy --all-targets -- -D warnings`. Fixed by setting `.static_crt(true)`
  only on calc-compiler's independent build, leaving calc-runtime's own build at `cc`'s
  default.

Verified locally: `cargo test --workspace` and `--features backend-llvm` (LLVM 21.1.1)
both green, no warnings; `cargo fmt`/`check`/`clippy -D warnings`/`doc -D warnings`/
`audit`/`build`/`test` all pass. Manually ran `(1 + 2) * 4 - 6 / 2` through
`calcc run --interpret`, `--backend=cranelift`, and `--backend=llvm` — all exit 9.
