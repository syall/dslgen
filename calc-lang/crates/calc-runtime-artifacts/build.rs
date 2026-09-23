//! Builds `calc-runtime` as a static archive for linking into compiled `.calc`
//! programs, and records exactly which native libraries that archive needs.
//!
//! Cargo only builds a dependency as a Rust library (`.rlib`), which no C-style
//! linker can read, so this runs a *nested* Cargo build: `cargo rustc
//! --crate-type staticlib` on `calc-runtime`. Being a real Cargo build, it resolves
//! `calc-runtime`'s dependencies normally (`calc-builtins`, `serde_json`, ...) —
//! unlike the bare `rustc` call it replaces (sessions A9–A11), which forced every
//! built-in implementation to be dependency-free.
//!
//! `--print native-static-libs` makes rustc write the list of system libraries the
//! archive needs on this target (what `std` and every dependency link against) to
//! `calc_runtime.link-deps`. The link driver passes that list verbatim — so the
//! archive and its dependencies always travel together, and nobody maintains the
//! list by hand.
//!
//! Environment handed to the crate's library (`src/lib.rs` reads each with `env!`):
//! `CALC_RUNTIME_TARGET`, `CALC_RUNTIME_LIB`, `CALC_RUNTIME_LINK_DEPS_FILE`,
//! `CALC_LINK_UNITS_DIR`, `CALC_IPC_BUNDLES_DIR`.

use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn main() {
    let out_dir = PathBuf::from(env::var("OUT_DIR").expect("Cargo sets OUT_DIR"));
    let target = env::var("TARGET").expect("Cargo sets TARGET");
    let msvc = env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc");
    // All published by `calc-runtime`'s own build script (`links = "calc_runtime"`).
    let runtime_dir = PathBuf::from(dep_metadata("MANIFEST_DIR"));
    let link_units_dir = dep_metadata("LINK_UNITS_DIR");
    let ipc_dir = dep_metadata("IPC_DIR");

    let target_dir = out_dir.join("runtime-target");
    let deps_file = out_dir.join("calc_runtime.link-deps");
    let mut cargo = Command::new(env::var("CARGO").expect("Cargo sets CARGO"));
    cargo
        .arg("rustc")
        .arg("--manifest-path")
        .arg(runtime_dir.join("Cargo.toml"))
        .args(["--lib", "--crate-type", "staticlib", "--release"])
        // No `link-natives`: the archive only *references* `calc_sub` and the IPC
        // bundle table. The separate `calc_ffi` link unit provides the first — the
        // same position a user's own FFI library would be in — and the link driver's
        // generated bundle object the second.
        .arg("--no-default-features")
        .args(["--locked", "--offline", "--target", &target])
        // A separate target directory: the outer build holds a lock on its own, so
        // sharing it would deadlock.
        .arg("--target-dir")
        .arg(&target_dir)
        .arg("--")
        .arg("--print")
        .arg(format!("native-static-libs={}", deps_file.display()))
        // A build script inherits the outer build's environment. Keep the nested build
        // independent of it: no clippy wrapper (under `cargo clippy`), no outer
        // RUSTFLAGS (either spelling) or target directory.
        .env_remove("RUSTC_WORKSPACE_WRAPPER")
        .env_remove("RUSTFLAGS")
        .env_remove("CARGO_TARGET_DIR")
        .env_remove("CARGO_ENCODED_RUSTFLAGS")
        // A static archive has no unwinding story at the C boundary; abort instead,
        // as the bare-`rustc` build before this one did.
        .env("CARGO_PROFILE_RELEASE_PANIC", "abort");
    if msvc {
        // Build everything against the static C runtime the link driver uses. rustc
        // then names that runtime in the deps list itself (`/defaultlib:libcmt`)
        // instead of the link driver hardcoding it.
        cargo.env("CARGO_ENCODED_RUSTFLAGS", "-Ctarget-feature=+crt-static");
    }
    let status = cargo
        .status()
        .expect("failed to run cargo to build calc-runtime's static archive");
    assert!(
        status.success(),
        "cargo failed to build calc-runtime's static archive"
    );

    let release_dir = target_dir.join(&target).join("release");
    let archive = release_dir.join(if msvc {
        "calc_runtime.lib"
    } else {
        "libcalc_runtime.a"
    });
    assert!(archive.exists(), "no archive at {}", archive.display());
    assert!(
        fs::read_to_string(&deps_file).is_ok_and(|deps| !deps.trim().is_empty()),
        "rustc wrote no native-static-libs list to {}",
        deps_file.display()
    );

    // Rerun whenever any source file of the nested build changes — its own
    // dep-info file lists them all (this crate's and its path dependencies'
    // sources), so nothing here names them.
    let dep_info = release_dir.join(if msvc {
        "calc_runtime.d"
    } else {
        "libcalc_runtime.d"
    });
    for path in dep_info_paths(&dep_info) {
        println!("cargo:rerun-if-changed={path}");
    }
    // Dep-info lists only path dependencies' sources; registry dependencies change
    // through the manifest or the workspace lockfile instead.
    println!(
        "cargo:rerun-if-changed={}",
        runtime_dir.join("Cargo.toml").display()
    );
    if let Some(lockfile) = runtime_dir
        .ancestors()
        .map(|dir| dir.join("Cargo.lock"))
        .find(|lockfile| lockfile.exists())
    {
        println!("cargo:rerun-if-changed={}", lockfile.display());
    }

    println!("cargo:rustc-env=CALC_RUNTIME_TARGET={target}");
    println!("cargo:rustc-env=CALC_RUNTIME_LIB={}", archive.display());
    println!(
        "cargo:rustc-env=CALC_RUNTIME_LINK_DEPS_FILE={}",
        deps_file.display()
    );
    println!("cargo:rustc-env=CALC_LINK_UNITS_DIR={link_units_dir}");
    println!("cargo:rustc-env=CALC_IPC_BUNDLES_DIR={ipc_dir}");
}

/// A value `calc-runtime`'s build script published with `cargo:<key>=<value>`.
fn dep_metadata(key: &str) -> String {
    let var = format!("DEP_CALC_RUNTIME_{key}");
    env::var(&var).unwrap_or_else(|_| panic!("calc-runtime didn't publish {var}"))
}

/// The prerequisites listed in a Makefile-style dep-info file (`out: dep dep ...`,
/// spaces in paths escaped as `\ `). Returns nothing if the file doesn't exist.
fn dep_info_paths(file: &Path) -> Vec<String> {
    let Ok(text) = fs::read_to_string(file) else {
        return Vec::new();
    };
    let Some(line) = text.lines().next() else {
        return Vec::new();
    };
    // `": "` rather than `':'`: Windows paths contain a drive-letter colon.
    let Some((_, deps)) = line.split_once(": ") else {
        return Vec::new();
    };
    let mut paths = Vec::new();
    let mut current = String::new();
    let mut chars = deps.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\\' if chars.peek() == Some(&' ') => current.push(chars.next().unwrap()),
            ' ' => paths.extend((!current.is_empty()).then(|| std::mem::take(&mut current))),
            _ => current.push(c),
        }
    }
    paths.extend((!current.is_empty()).then_some(current));
    paths
}
