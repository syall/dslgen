//! A minimal, explicitly temporary "link driver" — just enough to turn
//! `cranelift_backend`'s object file into a runnable executable so `calcc build`
//! and its tests can prove that object file is actually correct. Session A12
//! ("The link driver") replaces this with the real thing: assembling FFI/IPC
//! runtime libraries alongside the object file, per spec.md §4's `src/link.rs`.
//! This stub only ever links the one object file a backend produces, plus
//! `calc-runtime`'s static library (session A9).
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

/// Path to `calc-runtime`'s static library, built by `build.rs` (session A9) and baked
/// in at compile time. Always passed to the linker: it only pulls in archive members
/// whose symbols the object file actually leaves undefined (e.g. `calc_add`), so
/// programs that use no built-ins are unaffected.
const RUNTIME_LIB: &str = env!("CALC_RUNTIME_LIB");

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
            .arg(RUNTIME_LIB)
            .arg(format!("/Fe:{}", exe_path.display()));
    } else {
        cmd.arg(&object_path)
            .arg(RUNTIME_LIB)
            .arg("-o")
            .arg(&exe_path);
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

#[cfg(test)]
mod tests {
    use std::path::Path;
    use std::process::Command;

    use calc_syntax::lalrpop_frontend::LalrpopFrontend;
    use calc_syntax::{resolve, ParserFrontend};

    use super::{link, RUNTIME_LIB};

    fn contains(haystack: &[u8], needle: &[u8]) -> bool {
        haystack.windows(needle.len()).any(|w| w == needle)
    }

    /// `build.rs` must produce a real static library (an `ar` archive) under the file
    /// name the platform's linker expects: `lib<name>.a` for Unix-style linkers (what
    /// Linux CI uses), `<name>.lib` for MSVC.
    #[test]
    fn runtime_archive_is_a_static_library_with_the_platform_name() {
        let bytes = std::fs::read(RUNTIME_LIB).expect("build.rs wrote the archive");
        assert!(bytes.starts_with(b"!<arch>\n"), "not an ar archive");

        let name = Path::new(RUNTIME_LIB).file_name().unwrap();
        let expected = if cfg!(target_env = "msvc") {
            "calc_runtime.lib"
        } else {
            "libcalc_runtime.a"
        };
        assert_eq!(name, expected);
    }

    /// The symbol names listed in an `ar` archive's symbol table, when it has the
    /// GNU/MSVC layout: a first member named `/` holding a big-endian `u32` count, that
    /// many `u32` member offsets, then that many NUL-terminated names. (BSD/macOS
    /// archives name the first member differently, so this returns `None` for them.)
    ///
    /// Parsing the table, rather than searching the archive's bytes for `name\0`, matters:
    /// the first name follows the last offset, not a NUL, so a byte search for `\0name\0`
    /// misses it. On MSVC that goes unnoticed because a `.lib` repeats its names in a
    /// second table, but a Linux archive lists each name once.
    fn archive_symbols(ar: &[u8]) -> Option<Vec<String>> {
        if !ar.get(8..68)?.starts_with(b"/ ") {
            return None;
        }
        let data = ar.get(68..)?;
        let count = u32::from_be_bytes(data.get(..4)?.try_into().ok()?) as usize;
        let names = data.get(4 + 4 * count..)?;
        Some(
            names
                .split(|&b| b == 0)
                .take(count)
                .map(|n| String::from_utf8_lossy(n).into_owned())
                .collect(),
        )
    }

    /// A hand-built GNU-style archive holding only a symbol table, shaped like the ones
    /// Linux CI produces, where the first name follows a non-NUL offset byte.
    #[test]
    fn reads_the_symbol_table_of_a_gnu_style_archive() {
        let mut table = Vec::new();
        table.extend(2u32.to_be_bytes());
        table.extend(0x0000_00d8u32.to_be_bytes());
        table.extend(0x0000_00d8u32.to_be_bytes());
        table.extend(b"calc_add\0calc_mul\0");

        let mut ar = b"!<arch>
"
        .to_vec();
        ar.extend(
            format!(
                "{:<16}{:<12}{:<6}{:<6}{:<8}{:<10}`
",
                "/",
                0,
                0,
                0,
                0,
                table.len()
            )
            .bytes(),
        );
        ar.extend(&table);

        assert_eq!(
            archive_symbols(&ar),
            Some(vec!["calc_add".to_string(), "calc_mul".to_string()])
        );
    }

    /// The linker matches names literally, so each built-in's `symbol` must be listed in
    /// the archive's symbol table as exactly that string. A Rust-mangled name like
    /// `_ZN12calc_runtime8calc_add17h..E` is a different entry, so a missing
    /// `#[no_mangle]` fails here. macOS prefixes symbols with an underscore. If the
    /// archive isn't GNU/MSVC-shaped, fall back to searching for the NUL-terminated name.
    #[test]
    fn runtime_archive_exports_every_builtin_under_its_unmangled_name() {
        let bytes = std::fs::read(RUNTIME_LIB).expect("build.rs wrote the archive");
        let listed = archive_symbols(&bytes);
        for builtin in calc_runtime::BUILTINS {
            let underscored = format!("_{}", builtin.symbol);
            let found = match &listed {
                Some(symbols) => symbols
                    .iter()
                    .any(|s| s == builtin.symbol || *s == underscored),
                None => contains(&bytes, format!("{}\0", builtin.symbol).as_bytes()),
            };
            assert!(
                found,
                "`{}` is not exported by the runtime archive",
                builtin.symbol
            );
        }
    }

    /// The whole link step on its own, for every backend compiled into this build:
    /// an object file that leaves `calc_add`/`calc_mul` undefined must link against the
    /// runtime archive with this platform's linker command line and run correctly.
    /// (`calcc`'s other tests cover this too, but through codegen; a failure here
    /// points at linking rather than at a backend.)
    #[test]
    fn links_an_object_that_calls_builtins_from_the_runtime_archive() {
        let ast = LalrpopFrontend.parse("(1 + 2) * 4").expect("should parse");
        resolve(&ast).expect("should resolve");
        let program = calc_ir::lower(&ast);

        for backend in crate::backend::enabled() {
            let object = backend.compile(&program).expect("backend compiles");
            let out = std::env::temp_dir().join(format!("calc_a9_link_test_{}", backend.name()));
            let exe = link(&object, &out).expect("link should succeed");
            let status = Command::new(&exe).status().expect("executable should run");
            let _ = std::fs::remove_file(&exe);
            assert_eq!(status.code(), Some(12), "{} backend", backend.name());
        }
    }
}
