//! A minimal, explicitly temporary "link driver" — just enough to turn
//! `cranelift_backend`'s object file into a runnable executable so `calcc build`
//! and its tests can prove that object file is actually correct. Session A12
//! ("The link driver") replaces this with the real thing: assembling FFI/IPC
//! runtime libraries alongside the object file, per spec.md §4's `src/link.rs`.
//! This stub only ever links the one object file `cranelift_backend` produces.
//!
//! Finding a working system linker across platforms is exactly the problem the
//! widely-used `cc` crate already solves (it's what `rustc` itself and most
//! `build.rs` scripts use), so this reuses it rather than hand-rolling MSVC/Unix
//! toolchain discovery. `cc::Build` is normally driven from a build script, where
//! Cargo sets `TARGET`/`HOST`/`OPT_LEVEL` env vars for it to read; called from this
//! binary at runtime, none of those exist, so they're supplied explicitly using the
//! host triple this binary itself was built for (`target_lexicon::HOST`) — this
//! project has no cross-compilation story, so "the triple running this code" and
//! "the triple to link for" are always the same.

use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Links `object_bytes` (as produced by `cranelift_backend::compile_to_object`)
/// into a runnable executable, invoking the system's C toolchain purely as a
/// linker driver (MSVC's `cl.exe` or a Unix-style `cc`/`gcc`/`clang`) — the object
/// file's own `main` symbol becomes the executable's entry point. Returns the
/// executable's actual path: on MSVC, `cl.exe`'s `/Fe` always names an `.exe`, so
/// an extensionless `output_path` (e.g. roadmap.md's own `-o program` example)
/// lands at `output_path` + `.exe`, not literally `output_path` — this is computed
/// up front rather than left to `cl.exe` to decide so the caller always knows
/// exactly what file was produced.
pub fn link(object_bytes: &[u8], output_path: &Path) -> io::Result<PathBuf> {
    let host = target_lexicon::HOST.to_string();
    let compiler = cc::Build::new()
        .target(&host)
        .host(&host)
        .opt_level(0)
        .cargo_metadata(false)
        .get_compiler();

    // MSVC's `cl.exe` only recognizes an input as an object file by its `.obj`
    // extension (anything else is accepted with just a warning); Unix-style
    // drivers are similarly permissive about `.o`. Using the extension each
    // toolchain actually expects keeps that warning from showing up at all.
    let object_extension = if compiler.is_like_msvc() { "obj" } else { "o" };
    let object_path = output_path.with_extension(object_extension);
    std::fs::write(&object_path, object_bytes)?;
    let cleanup = TempFile(&object_path);

    // `cl.exe` always names its `/Fe` output with a `.exe`/`.dll` extension, adding
    // one itself if `output_path` doesn't already have one — computed here rather
    // than left implicit so the path this function returns is always the real one.
    let exe_path = if compiler.is_like_msvc() && output_path.extension().is_none() {
        output_path.with_extension("exe")
    } else {
        output_path.to_path_buf()
    };

    let mut cmd = Command::new(compiler.path());
    // `compiler.args()` carries *compile*-oriented default flags (e.g. `/c`,
    // which would stop the MSVC driver short of linking) — only the discovered
    // environment (crucially `LIB`/`INCLUDE` for MSVC, so the linker can find the
    // C runtime import libraries) is wanted here, not those args.
    for (key, value) in compiler.env() {
        cmd.env(key, value);
    }
    if compiler.is_like_msvc() {
        // A hand-built object file (unlike one `cl.exe` itself produced) carries no
        // `/DEFAULTLIB` directives, so the CRT startup routine `main` needs
        // (`mainCRTStartup`, which calls `main` and then exits the process) has to
        // be pulled in explicitly — the static-CRT trio below is self-contained
        // (no CRT DLL dependency in the resulting executable).
        cmd.arg(&object_path)
            .arg("/nologo")
            .arg("libcmt.lib")
            .arg("libvcruntime.lib")
            .arg("libucrt.lib")
            .arg(format!("/Fe:{}", exe_path.display()));
    } else {
        cmd.arg(&object_path).arg("-o").arg(&exe_path);
    }

    let status = cmd.status()?;
    drop(cleanup);
    if !status.success() {
        return Err(io::Error::other(format!("linker exited with {status}")));
    }
    Ok(exe_path)
}

/// Deletes the wrapped path when dropped, so the intermediate object file doesn't
/// linger next to `calcc build`'s requested output regardless of whether linking
/// succeeds.
struct TempFile<'a>(&'a Path);

impl Drop for TempFile<'_> {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(self.0);
    }
}
