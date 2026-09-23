//! Links this crate's native (C) pieces into its Rust consumers, and publishes where
//! everything compiled `.calc` programs need from this crate lives (session A12).
//!
//! With the `link-natives` feature (the default):
//!
//! * `native/calc_ffi.c` — the C-ABI FFI built-in `calc_sub` (spec.md §7 kind 2) —
//!   is compiled and linked into this crate's Rust consumers (the interpreter, every
//!   crate's tests, `calcc` itself) with the C runtime ordinary Rust binaries use
//!   (dynamic, on `*-msvc`);
//! * `native/no_ipc_bundles.c`, an empty IPC bundle table, is linked into them too,
//!   so `calc_print`'s reference to the table resolves (compiled programs get the
//!   real table from the link driver instead);
//! * a static-CRT copy of `calc_ffi` is written to `$OUT_DIR/link-units/` as the
//!   **link unit** compiled programs link against, matching the static C runtime the
//!   link driver (`calc-compiler/src/link.rs`) uses.
//!
//! Published as Cargo metadata (`links = "calc_runtime"`), read by a dependent's
//! build script as `DEP_CALC_RUNTIME_<KEY>`: `manifest_dir` (so
//! `calc-runtime-artifacts` can build this crate as a static archive without a
//! relative path to it), `ipc_dir` (the IPC bundles' source directories), and, with
//! the feature, `link_units_dir`.
//!
//! This is the only place that knows how `calc_ffi.c` is built: the link driver
//! finds the archive by the library name the manifest declares
//! (`calc_builtins`'s `BindingKind::Ffi { library: "calc_ffi", .. }`) in that
//! directory, so swapping in another library never touches the compiler.

use std::env;
use std::path::PathBuf;

fn main() {
    println!("cargo:rerun-if-changed=native");
    let manifest_dir =
        PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("Cargo sets CARGO_MANIFEST_DIR"));
    println!("cargo:manifest_dir={}", manifest_dir.display());
    println!("cargo:ipc_dir={}", manifest_dir.join("ipc").display());
    if env::var_os("CARGO_FEATURE_LINK_NATIVES").is_none() {
        return;
    }

    cc::Build::new()
        .file("native/calc_ffi.c")
        .compile("calc_ffi");
    cc::Build::new()
        .file("native/no_ipc_bundles.c")
        .compile("calc_no_ipc_bundles");

    let out_dir = PathBuf::from(env::var("OUT_DIR").expect("Cargo sets OUT_DIR"));
    let link_units = out_dir.join("link-units");
    cc::Build::new()
        .file("native/calc_ffi.c")
        .out_dir(&link_units)
        .static_crt(true)
        // Built for compiled programs, not for this crate: don't let `cc` tell Cargo
        // to link this copy too (the one above already provides `calc_sub`).
        .cargo_metadata(false)
        .compile("calc_ffi");
    println!("cargo:link_units_dir={}", link_units.display());
}
