//! Compiles `native/calc_ffi.c` (session A10, spec.md §7 kind 2) and links it into
//! every ordinary consumer of this crate — the interpreter, this crate's own tests,
//! `calc-ir`'s tests, and `calcc` itself. Unlike `lib.rs`'s native-Rust built-ins,
//! `calc_sub`'s only definition is C, so even an in-process caller like the
//! interpreter needs a real link step to reach it; that's what this build script is
//! for. It has nothing to do with `calc-compiler/build.rs`'s own, separate compile of
//! the same source file, which exists purely to hand `link_stub.rs` an archive path
//! for linking *generated* `.calc` programs (a non-Cargo link `cc::Build::compile`
//! here doesn't help with).

fn main() {
    println!("cargo:rerun-if-changed=native/calc_ffi.c");
    // No `.static_crt(...)` override: this archive is auto-linked (by Cargo, via `cc`'s
    // own `cargo:rustc-link-lib` output) into ordinary Rust binaries — `calcc`, and
    // every crate's own test harness — which use rustc's own default (dynamic) CRT on
    // `*-msvc` targets. `calc-compiler/build.rs`'s separate build of this same source
    // sets `.static_crt(true)` instead, to match the *different*, explicitly static CRT
    // `link_stub.rs` links for the generated `.calc` program.
    cc::Build::new()
        .file("native/calc_ffi.c")
        .compile("calc_ffi");
}
