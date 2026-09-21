# Session A8 — The `Backend` trait and feature-gated backend selection

Spec refs: spec.md §8.1, §14 #4. Roadmap: roadmap.md "A8".

## Plan

1. `src/backend.rs` (new): `trait Backend { name, compile -> Result<Vec<u8>, BackendError> }`,
   `BackendError`, `enabled()` (each entry `#[cfg(feature = …)]`-gated) and
   `select(Option<&str>)`: explicit name → that backend; known-but-disabled name → "rebuild with
   `--features …`" error; no name → the sole enabled backend, else an error listing choices.
   `compile_error!` if no backend feature is on.
2. `CraneliftBackend` / `LlvmBackend` unit structs implement the trait, wrapping the existing
   `compile_to_object` functions. Internal panics stay panics (conversion to `BackendError` deferred).
3. Cargo features: `backend-cranelift` (default) makes the `cranelift-*` crates optional;
   `backend-llvm` unchanged. `target-lexicon` stays unconditional (`link_stub` uses it).
4. `main.rs`: gate `mod cranelift_backend`; parse `build [--backend=<name>] <path> -o <out>`;
   `build` dispatches through `backend::select`; drop the per-backend match arms.
5. Gate tests: `#![cfg(feature = "backend-cranelift")]` on `tests/cranelift_recipes.rs`; the LLVM
   tests' Cranelift comparison only when both features are on; unit tests for `select`.
6. DECISIONS.md A8 entry (§14 #4 default = Cranelift-only; trait design; deferrals; CI).
7. Teaching doc `calc-lang/docs/a8-backend-trait-and-feature-gating.md` incl. a "How Cargo
   features work" section, and README entry 9.
8. One CI job per feature combination in `agent-evals.yml` — a backend-agnostic `checks` job (fmt, check, audit, build),
   `test-cranelift`, `test-llvm` and `test-all-features` (each: clippy, doc, test; LLVM 21 from apt.llvm.org) —
   added on request; no roadmap session had scheduled it.

## Outcome

Implemented as planned. Departures: `target-lexicon` is *not* made optional (the plan listed it
under Cranelift, but `link_stub` needs it in an LLVM-only build). Verified locally against a real
LLVM 21.1.1 install: default, `--no-default-features --features backend-llvm`, and
`--all-features` all pass tests and clippy; `--no-default-features` alone hits the `compile_error!`;
`cargo tree` shows zero Cranelift crates in the LLVM-only graph and zero `inkwell` in the default.
`calcc build` works with the implicit default in both single-backend builds. The two LLVM
workflow jobs can only be confirmed on GitHub: the first run failed to link
(`install-llvm-action`'s LLVM is built against libc++, `llvm-sys` links libstdc++), so they now install
LLVM 21 from apt.llvm.org; that setup is unconfirmed until its CI run.
