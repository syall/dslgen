//! What a compiled program needs *at run time*, prepared and checked at `calcc build`
//! time (session A12; spec.md §7). The link driver (`link.rs`) handles everything
//! linked in; this module handles the rest, which today means IPC built-ins:
//!
//! 1. **Check** each IPC built-in's command works on this machine, by running the
//!    manifest's probe (e.g. `python3 --version`), or, for a program shipped inside
//!    its bundle, checking the file is there. spec.md §7 wants an unreachable IPC
//!    executable reported at build time, not discovered by the program's users.
//! 2. **Pack** each bundle the program uses (`calc_builtins::bundle_format`), for the
//!    link driver to embed in the executable.
//! 3. **Report** exactly what the executable will need on the machine it runs on.
//!
//! Only built-ins the program actually calls count, found by walking its IR.

use std::fs;
use std::io;
use std::path::Path;
use std::process::Command;

use calc_builtins::bundle_format::{self, BundleFile};
use calc_builtins::{BindingKind, Builtin, RuntimeRequirement, BUNDLE_PLACEHOLDER};
use calc_ir::{Block, Instr, Program};
use calc_runtime_artifacts as artifacts;

/// The result of [`prepare`].
#[derive(Debug)]
pub struct Prepared {
    /// Every used IPC built-in's bundle, packed; hand to `link::link`.
    pub bundles: Vec<u8>,
    /// One line per run-time requirement, naming the built-in it's for.
    pub notes: Vec<String>,
}

/// Checks, packs and describes the run-time dependencies of `program`'s built-ins.
pub fn prepare(program: &Program) -> io::Result<Prepared> {
    let family = crate::link::target_family(artifacts::TARGET);
    let mut packed: Vec<(&str, Vec<BundleFile>)> = Vec::new();
    let mut notes = Vec::new();
    for builtin in used_builtins(program) {
        let BindingKind::Ipc { bundle, .. } = builtin.kind else {
            continue;
        };
        let bundle_dir = bundle.map(|name| Path::new(artifacts::IPC_BUNDLES_DIR).join(name));
        check_command(builtin, family, bundle_dir.as_deref())?;
        let mut sizes = String::new();
        if let (Some(name), Some(dir)) = (bundle, &bundle_dir) {
            let files = read_bundle(dir)?;
            let bytes: usize = files.iter().map(|f| f.bytes.len()).sum();
            sizes = format!(": {} files, {bytes} bytes", files.len());
            packed.push((name, files));
        }
        for requirement in builtin.runtime_requirements(family) {
            let detail = match requirement {
                RuntimeRequirement::BundleCache(_) => sizes.as_str(),
                RuntimeRequirement::CommandOnPath(_) => "",
            };
            notes.push(format!(
                "{requirement}{detail} (built-in `{}`)",
                builtin.name
            ));
        }
    }
    Ok(Prepared {
        bundles: bundle_format::pack(&packed),
        notes,
    })
}

/// The built-ins `program` calls, each once, in first-use order.
fn used_builtins(program: &Program) -> Vec<&'static Builtin> {
    fn walk(block: &Block, found: &mut Vec<&'static Builtin>) {
        for instr in &block.0 {
            match instr {
                Instr::CallBuiltin { name, .. } => {
                    let builtin =
                        calc_builtins::lookup(name).expect("lowering emits known built-ins");
                    if !found.iter().any(|b| b.name == builtin.name) {
                        found.push(builtin);
                    }
                }
                Instr::If {
                    then_block,
                    else_block,
                    ..
                } => {
                    walk(then_block, found);
                    walk(else_block, found);
                }
                Instr::Const { .. } | Instr::BinOp { .. } | Instr::Copy { .. } => {}
            }
        }
    }
    let mut found = Vec::new();
    walk(&program.body, &mut found);
    found
}

/// Confirms `builtin`'s command for `family` is well-formed and can run: it refers to
/// `{bundle}` exactly when the built-in declares a bundle; a program inside the
/// bundle exists there; and its probe, if any, exits 0.
fn check_command(builtin: &Builtin, family: &str, bundle_dir: Option<&Path>) -> io::Result<()> {
    let command = builtin.ipc_command(family).ok_or_else(|| {
        io::Error::other(format!(
            "built-in `{}` declares no {family} command",
            builtin.name
        ))
    })?;
    let bundle = match builtin.kind {
        BindingKind::Ipc { bundle, .. } => bundle,
        _ => None,
    };
    match (command.uses_bundle(), bundle) {
        (true, None) => {
            return Err(io::Error::other(format!(
                "built-in `{}`'s {family} command refers to {BUNDLE_PLACEHOLDER}, but the \
                 built-in declares no bundle",
                builtin.name
            )))
        }
        (false, Some(bundle)) => {
            return Err(io::Error::other(format!(
                "built-in `{}` declares bundle `{bundle}`, but its {family} command never \
                 refers to {BUNDLE_PLACEHOLDER}",
                builtin.name
            )))
        }
        _ => {}
    }
    let bundle_dir = bundle_dir
        .map(|d| d.display().to_string())
        .unwrap_or_default();
    let resolved = command.resolve(&bundle_dir);
    if command.runs_from_bundle() && !Path::new(&resolved.program).is_file() {
        return Err(io::Error::other(format!(
            "built-in `{}` runs `{}`, which isn't in its bundle",
            builtin.name, resolved.program
        )));
    }
    let Some(probe) = command.probe else {
        return Ok(());
    };
    let shown = std::iter::once(command.program)
        .chain(probe.iter().copied())
        .collect::<Vec<_>>()
        .join(" ");
    let failure = match Command::new(&resolved.program)
        .args(probe)
        .envs(resolved.env.iter().map(|(k, v)| (k, v)))
        .output()
    {
        Ok(output) if output.status.success() => return Ok(()),
        Ok(output) => format!(
            "it exited with {}: {}",
            output.status,
            String::from_utf8_lossy(&output.stderr).trim()
        ),
        Err(err) => format!("it couldn't be started: {err}"),
    };
    Err(io::Error::other(format!(
        "built-in `{}` needs `{}` at run time, but `{shown}` failed on this machine ({failure}); \
         install it, or change the command in the built-in manifest",
        builtin.name, command.program
    )))
}

/// Every file under `dir`, as bundle records: paths relative and `/`-separated,
/// sorted so the packed bundle (and its hash) doesn't depend on directory order.
/// Symlinks are followed, so the bundle holds real files on every platform.
fn read_bundle(dir: &Path) -> io::Result<Vec<BundleFile>> {
    fn visit(root: &Path, dir: &Path, files: &mut Vec<BundleFile>) -> io::Result<()> {
        for entry in fs::read_dir(dir)? {
            let path = entry?.path();
            let metadata = fs::metadata(&path)?;
            if metadata.is_dir() {
                visit(root, &path, files)?;
                continue;
            }
            let relative = path.strip_prefix(root).expect("walked from root");
            files.push(BundleFile {
                path: relative
                    .components()
                    .map(|c| c.as_os_str().to_string_lossy())
                    .collect::<Vec<_>>()
                    .join("/"),
                mode: file_mode(&metadata),
                bytes: fs::read(&path)?,
            });
        }
        Ok(())
    }
    let mut files = Vec::new();
    visit(dir, dir, &mut files).map_err(|err| {
        io::Error::other(format!("couldn't read bundle {}: {err}", dir.display()))
    })?;
    files.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(files)
}

#[cfg(unix)]
fn file_mode(metadata: &fs::Metadata) -> u32 {
    use std::os::unix::fs::PermissionsExt;
    metadata.permissions().mode() & 0o777
}

/// Windows files carry no Unix permission bits; bundles packed here are unpacked as
/// ordinary readable files.
#[cfg(not(unix))]
fn file_mode(_: &fs::Metadata) -> u32 {
    0o644
}

#[cfg(test)]
mod tests {
    use calc_builtins::IpcCommand;
    use calc_syntax::lalrpop_frontend::LalrpopFrontend;
    use calc_syntax::{resolve, ParserFrontend};

    use super::*;

    fn lower(src: &str) -> Program {
        let ast = LalrpopFrontend.parse(src).expect("should parse");
        resolve(&ast).expect("should resolve");
        calc_ir::lower(&ast)
    }

    #[test]
    fn a_program_without_ipc_builtins_needs_nothing_at_run_time() {
        let prepared = prepare(&lower("1 + 2 * 3 - 4")).unwrap();
        assert!(prepared.notes.is_empty());
        assert_eq!(bundle_format::parse(&prepared.bundles), Some(Vec::new()));
    }

    /// `print` used twice, in both branches of an `if`: one bundle, one set of notes,
    /// naming exactly the interpreter this platform will run.
    #[test]
    fn a_program_calling_print_embeds_its_bundle_and_reports_it() {
        let prepared = prepare(&lower("{ if 1 { print(1); 2 } else { print(3); 4 } }")).unwrap();
        let bundles = bundle_format::parse(&prepared.bundles).unwrap();
        assert_eq!(bundles.len(), 1);
        let paths: Vec<_> = bundles[0].files.iter().map(|f| f.path.as_str()).collect();
        assert_eq!(paths, ["__main__.py", "formatting.py"]);

        let python = if cfg!(windows) { "python" } else { "python3" };
        assert_eq!(prepared.notes.len(), 2);
        assert_eq!(
            prepared.notes[0],
            format!("`{python}` on PATH (built-in `print`)")
        );
        assert!(
            prepared.notes[1].contains("bundle `print`"),
            "{:?}",
            prepared.notes
        );
        assert!(
            prepared.notes[1].contains("first run: 2 files, "),
            "{:?}",
            prepared.notes
        );
    }

    fn fake(
        program: &'static str,
        probe: Option<&'static [&'static str]>,
        bundle: Option<&'static str>,
    ) -> Builtin {
        let commands: &'static [IpcCommand] = Box::leak(Box::new([IpcCommand {
            program,
            args: &[],
            env: &[],
            probe,
            only_on: None,
        }]));
        Builtin {
            name: "fake",
            symbol: "calc_fake",
            arity: 1,
            kind: BindingKind::Ipc { commands, bundle },
        }
    }

    /// The build fails, naming the built-in and the command, instead of the
    /// executable failing later on someone else's machine.
    #[test]
    fn a_command_that_doesnt_run_fails_the_build() {
        let builtin = fake("calc-no-such-interpreter", Some(&["--version"]), None);
        let err = check_command(&builtin, "unix", None)
            .unwrap_err()
            .to_string();
        assert!(
            err.starts_with(
                "built-in `fake` needs `calc-no-such-interpreter` at run time, but \
                 `calc-no-such-interpreter --version` failed"
            ),
            "{err}"
        );
    }

    #[test]
    fn a_program_inside_the_bundle_must_exist_there() {
        let dir = Path::new(artifacts::IPC_BUNDLES_DIR).join("print");
        let present = fake("{bundle}/__main__.py", None, Some("print"));
        check_command(&present, "unix", Some(&dir)).unwrap();
        let missing = fake("{bundle}/missing", None, Some("print"));
        let err = check_command(&missing, "unix", Some(&dir))
            .unwrap_err()
            .to_string();
        assert!(err.contains("which isn't in its bundle"), "{err}");
    }

    /// A program inside the bundle is still probed when it declares a probe. A `.py`
    /// file isn't something the OS can run directly, so this probe fails — proving it
    /// ran rather than being skipped after the file-exists check.
    #[test]
    fn a_program_inside_the_bundle_is_probed_too() {
        let dir = Path::new(artifacts::IPC_BUNDLES_DIR).join("print");
        let builtin = fake("{bundle}/__main__.py", Some(&["--version"]), Some("print"));
        let err = check_command(&builtin, "unix", Some(&dir))
            .unwrap_err()
            .to_string();
        assert!(err.contains("failed on this machine"), "{err}");
    }

    #[test]
    fn referring_to_a_bundle_the_builtin_doesnt_declare_fails_the_build() {
        let err = check_command(&fake("{bundle}/helper", None, None), "unix", None)
            .unwrap_err()
            .to_string();
        assert!(
            err.contains("refers to {bundle}, but the built-in declares no bundle"),
            "{err}"
        );
    }

    #[test]
    fn a_bundle_nothing_refers_to_fails_the_build() {
        let dir = Path::new(artifacts::IPC_BUNDLES_DIR).join("print");
        let err = check_command(&fake("python3", None, Some("print")), "unix", Some(&dir))
            .unwrap_err()
            .to_string();
        assert!(
            err.contains("declares bundle `print`, but its unix command never refers to {bundle}"),
            "{err}"
        );
    }
}
