# Session A9 — Built-ins, kind 1: native Rust functions

Spec refs: spec.md §7 (native Rust half). Roadmap: roadmap.md "A9".

## Plan

1. New crate `calc-runtime`: `#[no_mangle] extern "C"` `calc_add`/`calc_mul`, a hardcoded
   `BUILTINS` manifest (name, symbol, arity, interpreter `eval`) and `lookup`.
2. Per user direction there is no new call syntax: `calc-ir`'s `lower` routes `+`/`*` to a new
   generic `Instr::CallBuiltin { dst, name, args }`; `-`/`/` stay `BinOp`. The interpreter runs it via
   `lookup(..).eval`. The node stays operator-agnostic so `dslgen` can later emit it from real call syntax.
3. Backends: Cranelift declares the symbol `Linkage::Import` and `call`s it (module threaded through
   `lower_block`/`lower_instr`); LLVM declares it with `add_function` and `build_call`s it.
4. Linking: `calc-compiler/build.rs` builds `calc-runtime/src/lib.rs` with `rustc --crate-type=staticlib`
   and exports the path (`CALC_RUNTIME_LIB`); `link_stub::link` always passes the archive.
5. Tests: lowering (`+`/`*` → `CallBuiltin`, `-`/`/` inline), a test emitting every `Instr` kind (its
   `match` has no wildcard), mixed-program backend-vs-interpreter tests, and "recipe 5" (call an imported
   function) for Cranelift and LLVM.
6. Docs: DECISIONS.md A9 entry; `calc-lang/docs/a9-native-rust-builtins.md` (calc-runtime, `extern "C"`,
   `#[no_mangle]`, ABI, linker, before/after calc-ir/Cranelift/LLVM/machine code, recipe 5) with two SVG
   diagrams; README entry 10; CLAUDE.md workspace layout.

## Outcome

Implemented as planned. Departures: none of substance; the planned `no_std` fallback for the runtime
archive wasn't needed (the MSVC link of the plain-std staticlib works, pulling in only `calc_add`/`calc_mul`).
Cranelift emits each built-in call as an address load + `callq *%reg` rather than a rel32 call
(first `movabsq` of an absolute address; the first Linux CI run warned `creating DT_TEXTREL in a PIE`
for it, so the backend now sets `is_pic=true` and reads the address from a GOT slot instead); LLVM `-O2` can no longer fold `(1 + 2) * 4` (recorded in DECISIONS.md). Verified locally with default
features, `--no-default-features --features backend-llvm` and `--all-features` (LLVM 21.1.1); `calcc build`
of `(1 + 2) * 4` exits 12 under both backends.
