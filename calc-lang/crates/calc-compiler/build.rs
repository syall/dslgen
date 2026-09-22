//! Builds two static libraries `link_stub.rs` links into every compiled `.calc`
//! program, exposing each one's path via an environment variable (`link_stub` reads
//! both with `env!`):
//!
//! * **`CALC_RUNTIME_LIB`** (spec.md §7 kind 1; session A9): Cargo only builds
//!   `calc-runtime` as a Rust library (`.rlib`), which no C-style linker can read; so
//!   this script runs `rustc --crate-type=staticlib` on the same source file.
//!   `calc-runtime/src/lib.rs` is deliberately dependency-free so plain `rustc` can
//!   build it on its own, outside Cargo's dependency machinery.
//! * **`CALC_FFI_LIB`** (spec.md §7 kind 2; session A10): `calc-runtime`'s own
//!   `build.rs` already compiles `native/calc_ffi.c` and links it into every ordinary
//!   consumer of that crate (so the interpreter can call the real `calc_sub`) — but
//!   that never goes through a system linker `link_stub.rs` can address by path; this
//!   script independently compiles the same C source, via the `cc` crate this time
//!   (a C file, unlike `calc-runtime.rs`, is exactly what `cc` is for), purely to get
//!   an archive path to hand to `link_stub.rs` for the *generated program's* separate,
//!   non-Cargo link.

use std::env;
use std::path::PathBuf;
use std::process::Command;

/// MSVC's linker looks for `<name>.lib`; Unix-style linkers expect `lib<name>.a`.
fn archive_name(name: &str) -> String {
    if env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc") {
        format!("{name}.lib")
    } else {
        format!("lib{name}.a")
    }
}

fn main() {
    let out_dir = PathBuf::from(env::var("OUT_DIR").expect("Cargo sets OUT_DIR"));

    let runtime_src = PathBuf::from("../calc-runtime/src/lib.rs");
    println!("cargo:rerun-if-changed={}", runtime_src.display());
    let runtime_archive = out_dir.join(archive_name("calc_runtime"));
    let rustc = env::var("RUSTC").unwrap_or_else(|_| "rustc".to_string());
    let status = Command::new(rustc)
        .args(["--crate-type=staticlib", "--crate-name=calc_runtime"])
        .args(["--edition=2021", "-C", "opt-level=2", "-C", "panic=abort"])
        .arg("--target")
        .arg(env::var("TARGET").expect("Cargo sets TARGET"))
        .arg("-o")
        .arg(&runtime_archive)
        .arg(&runtime_src)
        .status()
        .expect("failed to run rustc to build calc-runtime's static library");
    assert!(status.success(), "rustc failed to build calc-runtime");
    println!(
        "cargo:rustc-env=CALC_RUNTIME_LIB={}",
        runtime_archive.display()
    );

    let ffi_src = PathBuf::from("../calc-runtime/native/calc_ffi.c");
    println!("cargo:rerun-if-changed={}", ffi_src.display());
    // Named distinctly from calc-runtime's own "calc_ffi" archive (same source, two
    // independent builds) so `calcc`'s own link never has two same-named static
    // libraries offering a definition of `calc_sub` — harmless either way, since a
    // linker only pulls in whichever one first resolves the symbol, but there's no
    // reason to rely on that when a distinct name sidesteps the question entirely.
    cc::Build::new()
        .file(&ffi_src)
        .out_dir(&out_dir)
        // Unlike calc-runtime's own build of this same source (which stays on `cc`'s
        // default, dynamic CRT to match ordinary Rust binaries), this archive is only
        // ever fed to `link_stub.rs`'s explicit static-CRT link line (`libcmt.lib` and
        // friends, below) for the *generated* `.calc` program, so it must match that.
        .static_crt(true)
        .compile("calc_compiler_calc_ffi");
    let ffi_archive = out_dir.join(archive_name("calc_compiler_calc_ffi"));
    println!("cargo:rustc-env=CALC_FFI_LIB={}", ffi_archive.display());
}
