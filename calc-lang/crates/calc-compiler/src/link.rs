//! The link driver (spec.md §4's `src/link.rs`; session A12): turns a backend's object
//! file into a runnable executable.
//!
//! Every built-in's symbol comes from a **link unit** — a static archive plus the list
//! of native libraries it needs — and linking is nothing more than handing the system
//! linker the object file, every link unit, and their dependencies:
//!
//! * `calc-runtime`'s archive (native-Rust built-ins and the IPC shim), with the
//!   dependency list rustc itself reported for it
//!   ([`calc_runtime_artifacts::RUNTIME_LINK_DEPS`]);
//! * one archive per FFI `library` the manifest (`calc_builtins`) declares, with the
//!   `link_deps` declared alongside it;
//! * one more object file the driver writes itself: the program's **embedded IPC
//!   bundles**, a packed blob (`calc_builtins::bundle_format`) defined as the data
//!   symbols `calc_ipc_bundles`/`calc_ipc_bundles_len`, which the IPC shim in the
//!   runtime archive reads and unpacks on first use.
//!
//! No library name is written in this file: what gets linked is entirely data, so
//! swapping in a different implementation never means editing the link driver.
//!
//! **Link order**: objects → runtime archive → FFI archives → dependency libraries.
//! Traditional Unix linkers (GNU `ld`, `gold`) read their inputs once, left to right,
//! and only pull a member out of an archive to resolve a symbol that is *already*
//! undefined at that point; so whatever uses a symbol must come before whatever
//! defines it. The program's object uses `calc_add`; the runtime archive uses
//! `calc_sub` (its interpreter table refers to every built-in); the FFI archive
//! defines `calc_sub`; everything uses the C library. (Object files, unlike archive
//! members, are always linked whole, so the bundle object's position doesn't matter.)
//! MSVC's `link.exe` and LLVM's `lld` remember every archive's symbols and don't
//! care, but one order that works for all of them is simplest.
//!
//! **Pre-link check**: before running the linker, every manifest built-in's symbol
//! must be exported by the link unit its kind names, and by no other — a missing or
//! duplicated symbol (easy to cause when swapping in your own library) is reported by
//! built-in name, rather than as a raw linker error.
//!
//! **Platforms**: the system C toolchain, found by the `cc` crate (it honors `CC` and
//! `CC_<target>`), is used as a linker driver, in one of two command-line flavors:
//! MSVC-style (`cl.exe`, clang-cl) or Unix-style (`cc`/`gcc`/`clang`). Tested on
//! x86_64 Windows MSVC and x86_64 Linux; macOS, MinGW and the BSDs take the same two
//! paths but are untested. Any other toolchain is an explicit error. The target is
//! always the one [`calc_runtime_artifacts::TARGET`] names — today the host;
//! cross-compilation is roadmap.md C8, a `wasm32` target (`wasm-ld`, a third flavor)
//! C1-wasm, dynamically linked FFI libraries C7. See
//! `calc-lang/docs/a12-the-link-driver.md`.

use std::fmt::Write as _;
use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::str::FromStr;

use calc_builtins::{BindingKind, Builtin, BUILTINS};
use calc_runtime_artifacts as artifacts;
use object::write::{Object, StandardSection, Symbol, SymbolSection};
use object::{SectionKind, SymbolFlags, SymbolKind, SymbolScope};
use target_lexicon::{Architecture, BinaryFormat, OperatingSystem, Triple};

/// The two command-line conventions this driver speaks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LinkFlavor {
    /// `cl.exe`-style: `name.lib` inputs, `/Fe:out`, linker options after `/link`.
    Msvc,
    /// `cc`-style: `-lname` libraries, `-o out`.
    Unix,
}

impl LinkFlavor {
    /// The file name a static library called `name` has in this flavor.
    fn archive_file_name(self, name: &str) -> String {
        match self {
            LinkFlavor::Msvc => format!("{name}.lib"),
            LinkFlavor::Unix => format!("lib{name}.a"),
        }
    }

    /// The linker argument naming a system library called `name`.
    fn library_arg(self, name: &str) -> String {
        match self {
            LinkFlavor::Msvc => format!("{name}.lib"),
            LinkFlavor::Unix => format!("-l{name}"),
        }
    }
}

/// One static archive in the link, with the native libraries it needs.
#[derive(Debug)]
struct LinkUnit {
    /// `None` for `calc-runtime`'s archive, `Some(library)` for an FFI library.
    library: Option<&'static str>,
    path: PathBuf,
    /// Linker arguments for the libraries this archive needs.
    deps: Vec<String>,
}

impl LinkUnit {
    fn describe(&self) -> String {
        match self.library {
            None => "the calc-runtime archive".to_string(),
            Some(library) => format!("library `{library}`"),
        }
    }
}

/// Links `object_bytes` (as produced by a `Backend`) into a runnable executable at
/// `output_path`, embedding `ipc_bundles` (a blob from
/// `calc_builtins::bundle_format::pack`; `runtime_deps::prepare` builds it). Returns
/// the executable's actual path: `cl.exe` always gives its output an extension, so on
/// MSVC an extensionless `output_path` lands at `output_path` + `.exe`.
pub fn link(object_bytes: &[u8], ipc_bundles: &[u8], output_path: &Path) -> io::Result<PathBuf> {
    let target = artifacts::TARGET;
    let compiler = cc::Build::new()
        .target(target)
        .host(target)
        .opt_level(0)
        .cargo_metadata(false)
        .get_compiler();
    let flavor = if compiler.is_like_msvc() {
        LinkFlavor::Msvc
    } else if compiler.is_like_gnu() || compiler.is_like_clang() {
        LinkFlavor::Unix
    } else {
        return Err(io::Error::other(format!(
            "unsupported toolchain `{}`: calcc links with an MSVC-style (cl.exe) or \
             Unix-style (cc, gcc, clang) driver; point CC at one",
            compiler.path().display()
        )));
    };

    let units = link_units(flavor, target_family(target));
    check_symbols(BUILTINS, &units)?;

    // Each toolchain recognizes an object file by its own extension.
    let object_extension = match flavor {
        LinkFlavor::Msvc => "obj",
        LinkFlavor::Unix => "o",
    };
    let object_path = output_path.with_extension(object_extension);
    std::fs::write(&object_path, object_bytes)?;
    let cleanup = TempFile(&object_path);
    let bundles_path = output_path.with_extension(format!("bundles.{object_extension}"));
    std::fs::write(&bundles_path, bundle_object(target, ipc_bundles)?)?;
    let bundles_cleanup = TempFile(&bundles_path);

    let exe_path = if flavor == LinkFlavor::Msvc && output_path.extension().is_none() {
        output_path.with_extension("exe")
    } else {
        output_path.to_path_buf()
    };

    let mut cmd = Command::new(compiler.path());
    // Only the environment `cc` discovered is wanted (for MSVC, crucially `LIB`, so
    // the linker finds system libraries) — not `compiler.args()`, which are
    // *compile* flags (e.g. `/c`, which would stop short of linking).
    for (key, value) in compiler.env() {
        cmd.env(key, value);
    }
    cmd.args(link_args(
        flavor,
        &[&object_path, &bundles_path],
        &units,
        &exe_path,
    ));

    let output = cmd.output()?;
    drop(cleanup);
    drop(bundles_cleanup);
    if !output.status.success() {
        return Err(io::Error::other(format!(
            "linker exited with {}\ncommand: {}\n{}{}",
            output.status,
            command_line(&cmd),
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr),
        )));
    }
    Ok(exe_path)
}

/// `"windows"` or `"unix"`: the `target_family` a `LinkDep::only_on` or an
/// `IpcCommand::only_on` is compared to.
pub(crate) fn target_family(target: &str) -> &'static str {
    let triple = Triple::from_str(target).expect("TARGET is a valid triple");
    if triple.operating_system == OperatingSystem::Windows {
        "windows"
    } else {
        "unix"
    }
}

/// Every link unit, in link order: the runtime archive, then each FFI library the
/// manifest declares (once, even if several built-ins share it). All of them, not
/// just the ones a program calls: the runtime archive's interpreter table refers to
/// every built-in, so every FFI symbol is needed whichever archive members get used.
fn link_units(flavor: LinkFlavor, target_family: &str) -> Vec<LinkUnit> {
    let mut units = vec![LinkUnit {
        library: None,
        path: PathBuf::from(artifacts::RUNTIME_LIB),
        deps: artifacts::runtime_link_deps().map(str::to_string).collect(),
    }];
    for builtin in BUILTINS {
        let BindingKind::Ffi { library, link_deps } = builtin.kind else {
            continue;
        };
        if units.iter().any(|u| u.library == Some(library)) {
            continue;
        }
        units.push(LinkUnit {
            library: Some(library),
            path: Path::new(artifacts::LINK_UNITS_DIR).join(flavor.archive_file_name(library)),
            deps: link_deps
                .iter()
                .filter(|dep| dep.applies_to(target_family))
                .map(|dep| flavor.library_arg(dep.name))
                .collect(),
        });
    }
    units
}

/// The link unit a built-in's symbol must come from.
fn owning_library(builtin: &Builtin) -> Option<&'static str> {
    match builtin.kind {
        BindingKind::NativeRust | BindingKind::Ipc { .. } => None,
        BindingKind::Ffi { library, .. } => Some(library),
    }
}

/// The pre-link check: every built-in's symbol is exported by its own link unit and
/// by no other. Units whose symbol table can't be read (a BSD/macOS-style archive)
/// are left to the linker.
fn check_symbols(builtins: &[Builtin], units: &[LinkUnit]) -> io::Result<()> {
    let mut problems = String::new();
    let mut tables = Vec::new();
    for unit in units {
        match std::fs::read(&unit.path) {
            Ok(bytes) => tables.push(archive_symbols(&bytes)),
            Err(err) => {
                let _ = writeln!(
                    problems,
                    "{} not found at {}: {err}",
                    unit.describe(),
                    unit.path.display()
                );
                tables.push(None);
            }
        }
    }
    for builtin in builtins {
        let owner = owning_library(builtin);
        for (unit, table) in units.iter().zip(&tables) {
            let Some(symbols) = table else { continue };
            let exported = exports(symbols, builtin.symbol);
            if unit.library == owner && !exported {
                let _ = writeln!(
                    problems,
                    "built-in `{}` needs symbol `{}` from {} ({}), but it isn't exported there",
                    builtin.name,
                    builtin.symbol,
                    unit.describe(),
                    unit.path.display()
                );
            } else if unit.library != owner && exported {
                let _ = writeln!(
                    problems,
                    "symbol `{}` (built-in `{}`) is also exported by {} ({}); \
                     each built-in must come from exactly one link unit",
                    builtin.symbol,
                    builtin.name,
                    unit.describe(),
                    unit.path.display()
                );
            }
        }
    }
    if problems.is_empty() {
        Ok(())
    } else {
        Err(io::Error::other(problems.trim_end().to_string()))
    }
}

/// A data-only object file for `target` defining `calc_ipc_bundles` (the packed
/// bytes) and `calc_ipc_bundles_len` (a `u64`), which the IPC shim in the runtime
/// archive declares `extern`. A linker resolves data symbols exactly like function
/// symbols; the blob just ends up in the executable's read-only data. No
/// relocations: the blob is flat bytes, so nothing in it depends on where it's
/// loaded.
fn bundle_object(target: &str, blob: &[u8]) -> io::Result<Vec<u8>> {
    let triple = Triple::from_str(target).expect("TARGET is a valid triple");
    let format = match triple.binary_format {
        BinaryFormat::Coff => object::BinaryFormat::Coff,
        BinaryFormat::Elf => object::BinaryFormat::Elf,
        BinaryFormat::Macho => object::BinaryFormat::MachO,
        other => return Err(io::Error::other(format!("no object format for {other}"))),
    };
    let architecture = match triple.architecture {
        Architecture::X86_64 => object::Architecture::X86_64,
        Architecture::Aarch64(_) => object::Architecture::Aarch64,
        other => {
            return Err(io::Error::other(format!(
                "no object architecture for {other}"
            )))
        }
    };
    let mut obj = Object::new(format, architecture, object::Endianness::Little);
    if format == object::BinaryFormat::Elf {
        // Says "this object doesn't need an executable stack"; without it, GNU ld
        // warns and marks the whole program's stack executable.
        obj.add_section(
            Vec::new(),
            b".note.GNU-stack".to_vec(),
            SectionKind::Elf(object::elf::SHT_PROGBITS),
        );
    }
    let section = obj.section_id(StandardSection::ReadOnlyData);
    let len = (blob.len() as u64).to_le_bytes();
    // At least one byte, so the data symbol always has an address of its own.
    let data = if blob.is_empty() { &[0u8][..] } else { blob };
    for (name, bytes, align) in [
        (&b"calc_ipc_bundles"[..], data, 1),
        (&b"calc_ipc_bundles_len"[..], &len[..], 8),
    ] {
        let symbol = obj.add_symbol(Symbol {
            name: name.to_vec(),
            value: 0,
            size: 0,
            kind: SymbolKind::Data,
            scope: SymbolScope::Linkage,
            weak: false,
            section: SymbolSection::Undefined,
            flags: SymbolFlags::None,
        });
        obj.add_symbol_data(symbol, section, bytes, align);
    }
    obj.write().map_err(io::Error::other)
}

/// The full argument list, in the documented link order.
fn link_args(
    flavor: LinkFlavor,
    objects: &[&Path],
    units: &[LinkUnit],
    exe_path: &Path,
) -> Vec<String> {
    let mut args: Vec<String> = objects.iter().map(|o| o.display().to_string()).collect();
    args.extend(units.iter().map(|u| u.path.display().to_string()));
    let deps = units.iter().flat_map(|u| &u.deps);
    match flavor {
        LinkFlavor::Msvc => {
            // `cl.exe` accepts `.lib` files as inputs, but options meant for the
            // linker itself (rustc reports the C runtime as `/defaultlib:libcmt`)
            // must follow `/link`.
            let (options, libraries): (Vec<&String>, Vec<&String>) =
                deps.partition(|d| d.starts_with('/'));
            args.extend(libraries.into_iter().cloned());
            args.push("/nologo".to_string());
            args.push(format!("/Fe:{}", exe_path.display()));
            if !options.is_empty() {
                args.push("/link".to_string());
                args.extend(options.into_iter().cloned());
            }
        }
        LinkFlavor::Unix => {
            args.extend(deps.cloned());
            args.push("-o".to_string());
            args.push(exe_path.display().to_string());
        }
    }
    args
}

/// `cmd` as a copy-pasteable line, for error messages. (`Command`'s `Debug` output
/// would also dump every environment variable set on it.)
fn command_line(cmd: &Command) -> String {
    std::iter::once(cmd.get_program())
        .chain(cmd.get_args())
        .map(|arg| {
            let arg = arg.to_string_lossy();
            if arg.contains(' ') {
                format!("\"{arg}\"")
            } else {
                arg.into_owned()
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// The symbol names listed in an `ar` archive's symbol table, when it has the
/// GNU/MSVC layout: a first member named `/` holding a big-endian `u32` count, that
/// many `u32` member offsets, then that many NUL-terminated names. (BSD/macOS
/// archives name the first member differently, so this returns `None` for them.)
///
/// This table is what a linker consults to decide which archive member defines an
/// undefined symbol — so reading it answers "does this archive export `calc_sub`?"
/// exactly the way the linker will.
fn archive_symbols(ar: &[u8]) -> Option<Vec<String>> {
    if !ar.starts_with(b"!<arch>\n") || !ar.get(8..68)?.starts_with(b"/ ") {
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

/// Whether `symbols` lists `symbol` under its exact, unmangled name — or with the
/// leading underscore macOS adds.
fn exports(symbols: &[String], symbol: &str) -> bool {
    symbols
        .iter()
        .any(|s| s == symbol || s.strip_prefix('_') == Some(symbol))
}

/// Deletes the wrapped path when dropped, so the intermediate object file doesn't
/// linger next to `calcc build`'s requested output whether or not linking succeeds.
struct TempFile<'a>(&'a Path);

impl Drop for TempFile<'_> {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(self.0);
    }
}

#[cfg(test)]
mod tests {
    use calc_builtins::LinkDep;
    use calc_syntax::lalrpop_frontend::LalrpopFrontend;
    use calc_syntax::{resolve, ParserFrontend};

    use super::*;

    fn flavor() -> LinkFlavor {
        if cfg!(target_env = "msvc") {
            LinkFlavor::Msvc
        } else {
            LinkFlavor::Unix
        }
    }

    fn host_units() -> Vec<LinkUnit> {
        link_units(flavor(), target_family(artifacts::TARGET))
    }

    /// A GNU-style archive holding only a symbol table listing `names`, shaped like
    /// the ones Linux produces, where the first name follows a non-NUL offset byte.
    fn gnu_archive(names: &[&str]) -> Vec<u8> {
        let mut table = Vec::new();
        table.extend((names.len() as u32).to_be_bytes());
        for _ in names {
            table.extend(0x0000_00d8u32.to_be_bytes());
        }
        for name in names {
            table.extend(name.as_bytes());
            table.push(0);
        }
        let mut ar = b"!<arch>\n".to_vec();
        ar.extend(
            format!(
                "{:<16}{:<12}{:<6}{:<6}{:<8}{:<10}`\n",
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
        ar
    }

    #[test]
    fn reads_the_symbol_table_of_a_gnu_style_archive() {
        assert_eq!(
            archive_symbols(&gnu_archive(&["calc_add", "calc_mul"])),
            Some(vec!["calc_add".to_string(), "calc_mul".to_string()])
        );
        assert_eq!(archive_symbols(b"not an archive"), None);
    }

    /// The runtime archive is a real static library, and every FFI link unit sits in
    /// `LINK_UNITS_DIR` under the file name this platform's linker expects.
    #[test]
    fn link_units_are_static_libraries_with_the_platform_names() {
        let units = host_units();
        assert_eq!(units[0].library, None);
        assert_eq!(units[1].library, Some("calc_ffi"));
        for unit in &units {
            let bytes = std::fs::read(&unit.path).expect("link unit exists");
            assert!(
                bytes.starts_with(b"!<arch>\n"),
                "{} is not an ar archive",
                unit.describe()
            );
        }
        let expected = if cfg!(target_env = "msvc") {
            "calc_ffi.lib"
        } else {
            "libcalc_ffi.a"
        };
        assert_eq!(units[1].path.file_name().unwrap(), expected);
    }

    /// The real link units pass the pre-link check: each built-in's symbol, under
    /// its exact unmangled name, is exported by its own unit only — in particular,
    /// the runtime archive only *references* `calc_sub`, it doesn't define it.
    #[test]
    fn every_builtin_is_exported_by_exactly_its_own_link_unit() {
        check_symbols(BUILTINS, &host_units()).expect("real link units are consistent");
        let runtime = std::fs::read(artifacts::RUNTIME_LIB).unwrap();
        let symbols = archive_symbols(&runtime).expect("GNU/MSVC-style symbol table");
        assert!(exports(&symbols, "calc_add"));
        assert!(!exports(&symbols, "calc_sub"));
    }

    fn fake_unit(library: Option<&'static str>, archive: &[u8], name: &str) -> LinkUnit {
        let path = std::env::temp_dir().join(format!("calc_a12_fake_{name}.a"));
        std::fs::write(&path, archive).unwrap();
        LinkUnit {
            library,
            path,
            deps: Vec::new(),
        }
    }

    const SUB: Builtin = Builtin {
        name: "sub",
        symbol: "calc_sub",
        arity: 2,
        kind: BindingKind::Ffi {
            library: "calc_ffi",
            link_deps: &[],
        },
    };

    #[test]
    fn pre_link_check_reports_a_missing_symbol_by_builtin() {
        let units = [
            fake_unit(None, &gnu_archive(&["calc_add"]), "missing_rt"),
            fake_unit(
                Some("calc_ffi"),
                &gnu_archive(&["calc_other"]),
                "missing_ffi",
            ),
        ];
        let err = check_symbols(&[SUB], &units).unwrap_err().to_string();
        assert!(
            err.contains("built-in `sub` needs symbol `calc_sub` from library `calc_ffi`"),
            "{err}"
        );
    }

    #[test]
    fn pre_link_check_reports_a_symbol_exported_twice() {
        let units = [
            fake_unit(None, &gnu_archive(&["calc_sub"]), "dup_rt"),
            fake_unit(Some("calc_ffi"), &gnu_archive(&["calc_sub"]), "dup_ffi"),
        ];
        let err = check_symbols(&[SUB], &units).unwrap_err().to_string();
        assert!(
            err.contains("is also exported by the calc-runtime archive"),
            "{err}"
        );
    }

    #[test]
    fn pre_link_check_reports_a_missing_link_unit() {
        let units = [LinkUnit {
            library: Some("calc_ffi"),
            path: PathBuf::from("no/such/calc_ffi.lib"),
            deps: Vec::new(),
        }];
        let err = check_symbols(&[SUB], &units).unwrap_err().to_string();
        assert!(err.contains("library `calc_ffi` not found at"), "{err}");
    }

    #[test]
    fn link_args_follow_the_documented_order() {
        let units = [
            LinkUnit {
                library: None,
                path: PathBuf::from("rt.a"),
                deps: vec!["-lc".into()],
            },
            LinkUnit {
                library: Some("calc_ffi"),
                path: PathBuf::from("ffi.a"),
                deps: vec!["-lm".into()],
            },
        ];
        assert_eq!(
            link_args(
                LinkFlavor::Unix,
                &[Path::new("p.o"), Path::new("p.bundles.o")],
                &units,
                Path::new("p")
            ),
            [
                "p.o",
                "p.bundles.o",
                "rt.a",
                "ffi.a",
                "-lc",
                "-lm",
                "-o",
                "p"
            ]
        );
    }

    #[test]
    fn msvc_linker_options_go_after_slash_link() {
        let units = [LinkUnit {
            library: None,
            path: PathBuf::from("rt.lib"),
            deps: vec!["ws2_32.lib".into(), "/defaultlib:libcmt".into()],
        }];
        assert_eq!(
            link_args(
                LinkFlavor::Msvc,
                &[Path::new("p.obj")],
                &units,
                Path::new("p.exe")
            ),
            [
                "p.obj",
                "rt.lib",
                "ws2_32.lib",
                "/nologo",
                "/Fe:p.exe",
                "/link",
                "/defaultlib:libcmt"
            ]
        );
    }

    #[test]
    fn ffi_link_deps_are_filtered_by_platform() {
        let deps = [
            LinkDep {
                name: "m",
                only_on: Some("unix"),
            },
            LinkDep {
                name: "ws2_32",
                only_on: Some("windows"),
            },
        ];
        let kept: Vec<_> = deps
            .iter()
            .filter(|d| d.applies_to(target_family("x86_64-unknown-linux-gnu")))
            .map(|d| LinkFlavor::Unix.library_arg(d.name))
            .collect();
        assert_eq!(kept, ["-lm"]);
        assert_eq!(target_family("x86_64-pc-windows-msvc"), "windows");
    }

    /// A linker failure reports the exact command line and the linker's own output,
    /// so it can be reproduced by hand.
    #[test]
    fn a_linker_failure_shows_the_command_and_its_output() {
        let out = std::env::temp_dir().join("calc_a12_garbage");
        let err = link(b"not an object file", &no_bundles(), &out)
            .unwrap_err()
            .to_string();
        assert!(err.starts_with("linker exited with"), "{err}");
        assert!(err.contains("command: "), "{err}");
        assert!(err.contains(artifacts::RUNTIME_LIB), "{err}");
    }

    fn no_bundles() -> Vec<u8> {
        calc_builtins::bundle_format::pack(&[])
    }

    /// The generated bundle object is a real object file for this target, defining
    /// both symbols the IPC shim declares.
    #[test]
    fn the_bundle_object_defines_the_table_symbols() {
        use object::{Object as _, ObjectSymbol as _};
        let bytes = bundle_object(artifacts::TARGET, &no_bundles()).unwrap();
        let file = object::File::parse(&*bytes).expect("a valid object file");
        let names: Vec<String> = file
            .symbols()
            .filter(|s| s.is_global() && s.is_definition())
            .map(|s| s.name().unwrap().trim_start_matches('_').to_string())
            .collect();
        assert!(names.contains(&"calc_ipc_bundles".to_string()), "{names:?}");
        assert!(
            names.contains(&"calc_ipc_bundles_len".to_string()),
            "{names:?}"
        );
    }

    /// Compiles `src` with every enabled backend, links it (embedding its IPC
    /// bundles), copies *only* the executable into an otherwise empty directory, and
    /// runs it there with `cache` as its bundle cache.
    fn compile_link_run(
        src: &str,
        name: &str,
        cache: &Path,
    ) -> Vec<(String, std::process::Output)> {
        let ast = LalrpopFrontend.parse(src).expect("should parse");
        resolve(&ast).expect("should resolve");
        let program = calc_ir::lower(&ast);
        let prepared = crate::runtime_deps::prepare(&program).expect("run-time deps check out");
        crate::backend::enabled()
            .iter()
            .map(|backend| {
                let object = backend.compile(&program).expect("backend compiles");
                let dir = std::env::temp_dir().join(format!("calc_a12_{name}_{}", backend.name()));
                let _ = std::fs::remove_dir_all(&dir);
                std::fs::create_dir_all(dir.join("run")).unwrap();
                let exe = link(&object, &prepared.bundles, &dir.join("prog"))
                    .expect("link should succeed");
                let moved = dir.join("run").join(exe.file_name().unwrap());
                std::fs::rename(&exe, &moved).unwrap();
                let output = Command::new(&moved)
                    .env("CALC_BUNDLE_CACHE", cache)
                    .output()
                    .expect("executable should run");
                let _ = std::fs::remove_dir_all(&dir);
                (backend.name().to_string(), output)
            })
            .collect()
    }

    /// Native Rust (`+`, `*`) and FFI (`-`) built-ins, linked from their two archives.
    #[test]
    fn links_an_object_that_calls_builtins_from_both_archives() {
        let cache = std::env::temp_dir().join("calc_a12_arith_cache");
        for (backend, output) in compile_link_run("(1 + 2) * 4 - 3", "arith", &cache) {
            assert_eq!(output.status.code(), Some(9), "{backend} backend");
        }
    }

    /// The deliverable: one program using all three built-in kinds at once — `*`
    /// and `+` (native Rust), `-` (FFI), `print` (IPC) — through every backend, run
    /// from a directory holding nothing but the executable: `print`'s multi-file
    /// project comes out of the executable itself, unpacked into the cache.
    #[test]
    fn links_and_runs_a_program_using_all_three_builtin_kinds() {
        let cache = std::env::temp_dir().join(format!("calc_a12_cache_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&cache);
        for (backend, output) in
            compile_link_run("{ print(2 * 3 + 4 - 1); 7 }", "all_kinds", &cache)
        {
            let stdout = String::from_utf8_lossy(&output.stdout);
            assert_eq!(stdout, "result = 9.00\n", "{backend} backend: {output:?}");
            assert_eq!(output.status.code(), Some(7), "{backend} backend");
        }
        let unpacked: Vec<_> = std::fs::read_dir(cache.join("calc-bundles"))
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(unpacked.len(), 1, "{unpacked:?}");
        assert!(unpacked[0].starts_with("print-"), "{unpacked:?}");
        let _ = std::fs::remove_dir_all(&cache);
    }
}
