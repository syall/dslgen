//! A minimal, explicitly temporary "link driver" — just enough to turn a backend's
//! object file into a runnable executable so `calcc build` and its tests can prove
//! that object file is actually correct. Session A12 ("The link driver") replaces
//! this with the real thing per spec.md §4's `src/link.rs`. This stub only ever
//! links the one object file a backend produces, plus `calc-runtime`'s static
//! library (native Rust + IPC kinds, sessions A9/A11) and the FFI archive (session
//! A10) — every built-in kind lands in one of those two archives, so a program
//! using all three kinds together links with no further changes here.
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

/// Path to the C-ABI FFI built-ins' static library (`calc_sub`; session A10), built by
/// `build.rs` from `calc-runtime/native/calc_ffi.c`. Always passed alongside
/// `RUNTIME_LIB`, for the same reason: unused archive members cost nothing.
const FFI_LIB: &str = env!("CALC_FFI_LIB");

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
            // session A11: `calc_print` (kind 3, IPC) is the first built-in whose real
            // implementation uses more of `std` than plain arithmetic — spawning a
            // subprocess and piping its stdio pulls in Windows' native named-pipe I/O
            // (`ntdll.lib`), Winsock (`ws2_32.lib`, via `std`'s networking code, even
            // though `print` itself never opens a socket), and the user-profile lookup
            // `std::env::home_dir` uses (`userenv.lib`) — none of which a hand-built
            // object file's `/DEFAULTLIB` directives can pull in the way a normal
            // `rustc`-produced one would. Found by hitting `LNK2019` on all three when
            // `RUNTIME_LIB` first gained a symbol whose implementation needed them.
            .arg("ws2_32.lib")
            .arg("ntdll.lib")
            .arg("userenv.lib")
            .arg(RUNTIME_LIB)
            .arg(FFI_LIB)
            .arg(format!("/Fe:{}", exe_path.display()));
    } else {
        cmd.arg(&object_path)
            .arg(RUNTIME_LIB)
            .arg(FFI_LIB)
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

    use super::{link, FFI_LIB, RUNTIME_LIB};

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

    /// Same check for the FFI archive (session A10), named `calc_compiler_calc_ffi`
    /// (not `calc_ffi`) since it's `calc-compiler/build.rs`'s own independent build of
    /// `calc-runtime/native/calc_ffi.c`, distinct from the archive `calc-runtime`'s own
    /// `build.rs` links into ordinary Rust consumers.
    #[test]
    fn ffi_archive_is_a_static_library_with_the_platform_name() {
        let bytes = std::fs::read(FFI_LIB).expect("build.rs wrote the archive");
        assert!(bytes.starts_with(b"!<arch>\n"), "not an ar archive");

        let name = Path::new(FFI_LIB).file_name().unwrap();
        let expected = if cfg!(target_env = "msvc") {
            "calc_compiler_calc_ffi.lib"
        } else {
            "libcalc_compiler_calc_ffi.a"
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

    /// Checks that `archive`'s symbol table (or, if it isn't GNU/MSVC-shaped, its raw
    /// bytes) lists `symbol` under its exact, unmangled name. macOS prefixes symbols
    /// with an underscore, so that spelling counts too.
    fn archive_exports(archive: &[u8], symbol: &str) -> bool {
        let underscored = format!("_{symbol}");
        match archive_symbols(archive) {
            Some(symbols) => symbols.iter().any(|s| s == symbol || *s == underscored),
            None => contains(archive, format!("{symbol}\0").as_bytes()),
        }
    }

    /// The linker matches names literally, so each native-Rust built-in's `symbol` must
    /// be listed in `RUNTIME_LIB`'s symbol table as exactly that string. A Rust-mangled
    /// name like `_ZN12calc_runtime8calc_add17h..E` is a different entry, so a missing
    /// `#[no_mangle]` fails here.
    #[test]
    fn runtime_archive_exports_every_native_rust_builtin_under_its_unmangled_name() {
        let bytes = std::fs::read(RUNTIME_LIB).expect("build.rs wrote the archive");
        for builtin in calc_runtime::BUILTINS {
            if builtin.kind != calc_runtime::BindingKind::NativeRust {
                continue;
            }
            assert!(
                archive_exports(&bytes, builtin.symbol),
                "`{}` is not exported by the runtime archive",
                builtin.symbol
            );
        }
    }

    /// Same check for IPC-kind built-ins (session A11): `calc_print` is a real,
    /// self-contained `#[no_mangle] extern "C"` definition in `calc-runtime`'s
    /// `src/lib.rs` (see that file's `BindingKind::Ipc` doc comment), so — unlike
    /// FFI's `calc_sub`, defined in a separate C file — it lands in `RUNTIME_LIB`
    /// alongside the native-Rust built-ins, not `FFI_LIB`.
    #[test]
    fn runtime_archive_exports_every_ipc_builtin_under_its_unmangled_name() {
        let bytes = std::fs::read(RUNTIME_LIB).expect("build.rs wrote the archive");
        for builtin in calc_runtime::BUILTINS {
            if builtin.kind != calc_runtime::BindingKind::Ipc {
                continue;
            }
            assert!(
                archive_exports(&bytes, builtin.symbol),
                "`{}` is not exported by the runtime archive",
                builtin.symbol
            );
        }
    }

    /// Same check for FFI-kind built-ins (session A10) against `FFI_LIB` instead —
    /// their `symbol` is defined in `calc-runtime/native/calc_ffi.c`, not in the Rust
    /// runtime archive.
    #[test]
    fn ffi_archive_exports_every_ffi_builtin_under_its_unmangled_name() {
        let bytes = std::fs::read(FFI_LIB).expect("build.rs wrote the archive");
        for builtin in calc_runtime::BUILTINS {
            if builtin.kind != calc_runtime::BindingKind::Ffi {
                continue;
            }
            assert!(
                archive_exports(&bytes, builtin.symbol),
                "`{}` is not exported by the FFI archive",
                builtin.symbol
            );
        }
    }

    /// The whole link step on its own, for every backend compiled into this build: an
    /// object file that leaves `calc_add`/`calc_mul` (native Rust, `RUNTIME_LIB`) and
    /// `calc_sub` (C-ABI FFI, `FFI_LIB`; session A10) undefined must link against both
    /// archives with this platform's linker command line and run correctly. (`calcc`'s
    /// other tests cover this too, but through codegen; a failure here points at
    /// linking rather than at a backend.)
    #[test]
    fn links_an_object_that_calls_builtins_from_both_archives() {
        let ast = LalrpopFrontend
            .parse("(1 + 2) * 4 - 3")
            .expect("should parse");
        resolve(&ast).expect("should resolve");
        let program = calc_ir::lower(&ast);

        for backend in crate::backend::enabled() {
            let object = backend.compile(&program).expect("backend compiles");
            let out = std::env::temp_dir().join(format!("calc_a9_link_test_{}", backend.name()));
            let exe = link(&object, &out).expect("link should succeed");
            let status = Command::new(&exe).status().expect("executable should run");
            let _ = std::fs::remove_file(&exe);
            assert_eq!(status.code(), Some(9), "{} backend", backend.name());
        }
    }

    /// The IPC kind (session A11), through a real linked executable: an object file
    /// that leaves `calc_print` undefined must link against `RUNTIME_LIB` (where it
    /// lives, alongside the native-Rust built-ins) and, once run, actually spawn
    /// `print.py` and relay its output — not just resolve the symbol. This is what
    /// motivated the extra `ws2_32.lib`/`ntdll.lib`/`userenv.lib` above: without
    /// them this test's link step fails with `LNK2019`, even though the plain
    /// arithmetic in the test above links fine with just the CRT libs.
    #[test]
    fn links_and_runs_a_program_that_calls_the_ipc_builtin() {
        let ast = LalrpopFrontend
            .parse("{ print(1 + 2); 5 }")
            .expect("should parse");
        resolve(&ast).expect("should resolve");
        let program = calc_ir::lower(&ast);

        for backend in crate::backend::enabled() {
            let object = backend.compile(&program).expect("backend compiles");
            let out = std::env::temp_dir().join(format!("calc_a11_link_test_{}", backend.name()));
            let exe = link(&object, &out).expect("link should succeed");
            let output = Command::new(&exe).output().expect("executable should run");
            let _ = std::fs::remove_file(&exe);
            let stdout = String::from_utf8_lossy(&output.stdout).replace("\r\n", "\n");
            assert_eq!(stdout, "result = 3.00\n", "{} backend", backend.name());
            assert_eq!(output.status.code(), Some(5), "{} backend", backend.name());
        }
    }
}
