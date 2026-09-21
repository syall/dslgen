//! Builds `calc-runtime` into a static library the linker can consume (spec.md §7,
//! session A9). Cargo only builds `calc-runtime` as a Rust library (`.rlib`) for
//! `calc-ir`'s interpreter, which no C-style linker can read; so this script also runs
//! `rustc --crate-type=staticlib` on the same source file and tells the compiler
//! where the result is via the `CALC_RUNTIME_LIB` environment variable
//! (`link_stub` reads it with `env!`). `calc-runtime/src/lib.rs` is deliberately
//! dependency-free so plain `rustc` can build it on its own.

use std::env;
use std::path::PathBuf;
use std::process::Command;

fn main() {
    let runtime_src = PathBuf::from("../calc-runtime/src/lib.rs");
    println!("cargo:rerun-if-changed={}", runtime_src.display());

    let out_dir = PathBuf::from(env::var("OUT_DIR").expect("Cargo sets OUT_DIR"));
    // MSVC's linker looks for `.lib`; Unix-style linkers expect `lib<name>.a`.
    let archive_name = if env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc") {
        "calc_runtime.lib"
    } else {
        "libcalc_runtime.a"
    };
    let archive = out_dir.join(archive_name);

    let rustc = env::var("RUSTC").unwrap_or_else(|_| "rustc".to_string());
    let status = Command::new(rustc)
        .args(["--crate-type=staticlib", "--crate-name=calc_runtime"])
        .args(["--edition=2021", "-C", "opt-level=2", "-C", "panic=abort"])
        .arg("--target")
        .arg(env::var("TARGET").expect("Cargo sets TARGET"))
        .arg("-o")
        .arg(&archive)
        .arg(&runtime_src)
        .status()
        .expect("failed to run rustc to build calc-runtime's static library");
    assert!(status.success(), "rustc failed to build calc-runtime");

    println!("cargo:rustc-env=CALC_RUNTIME_LIB={}", archive.display());
}
