//! calc-lang's built-in manifest (spec.md §7): what each built-in *is* — its
//! DSL-visible name, the linker symbol compiled code calls, its arity, which of the
//! three binding kinds backs it, and what that kind needs at link time and at run
//! time. Pure data (plus [`bundle_format`], the pack format IPC bundles travel in):
//! no implementations, no dependencies, no build script.
//!
//! Split out of `calc-runtime` in session A12 so the parts of the compiler that only
//! need to *describe* a built-in (both codegen backends, the link driver, `calcc`'s
//! run-time-requirements note) never depend on the crate holding the implementations,
//! and so a user swapping in their own implementation edits one declaration here
//! rather than compiler code. The implementations themselves live in `calc-runtime`,
//! which also exposes the interpreter's entry points (`calc_runtime::eval`).
//!
//! This table is hardcoded for now; the data-driven `bindings.toml` (roadmap.md B6)
//! parses into these same types. See `calc-lang/docs/a12-the-link-driver.md`.

pub mod bundle_format;

use std::fmt;

/// One entry of the built-in manifest.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Builtin {
    /// The name calc-lang's lowering uses (`+` lowers to `add`, and so on).
    pub name: &'static str,
    /// The C-ABI symbol compiled code calls; the link driver must find exactly one
    /// link unit exporting it.
    pub symbol: &'static str,
    /// Argument count. Every argument and the result are `f64`, calc-lang's only
    /// type, so an arity is a whole signature.
    pub arity: usize,
    /// Which binding kind backs it, with that kind's own dependency data.
    pub kind: BindingKind,
}

/// Which of spec.md §7's three binding kinds backs a built-in, carrying the data
/// each kind needs to be linked and run — the same fields a `bindings.toml`
/// `[[builtin.impl]]` block will declare (spec.md §7.1). Compiled code calls every
/// kind the same way (an ordinary linked `call`); what differs is *where* the
/// symbol is defined and what it drags in.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum BindingKind {
    /// Rust compiled into `calc-runtime`'s own static archive. Its link
    /// dependencies are whatever rustc reports for that archive, so nothing is
    /// declared here.
    NativeRust,
    /// A separately built C-ABI static library, linked as its own link unit.
    /// `library` is its link name (`calc_ffi` → `calc_ffi.lib` / `libcalc_ffi.a`);
    /// `link_deps` lists the system libraries it needs, since a prebuilt archive
    /// can't report its own dependencies the way rustc does for a Rust one.
    Ffi {
        library: &'static str,
        link_deps: &'static [LinkDep],
    },
    /// An external program speaking the JSON protocol (see `calc-runtime`'s
    /// `ipc_runtime.rs`), started by the shim in `calc-runtime`'s archive. Nothing
    /// extra to link. `commands` holds exactly one [`IpcCommand`] per target family;
    /// `bundle`, if set, names a directory of files the command needs (under
    /// `calc-runtime/ipc/`), which `calcc build` embeds in the executable.
    Ipc {
        commands: &'static [IpcCommand],
        bundle: Option<&'static str>,
    },
}

/// How to start an IPC built-in's program on one platform. Nothing here is specific
/// to a language: a Python project, a Node.js script, a Java jar and a compiled
/// helper binary are all just a `program` with `args`. In `program`, `args` and
/// `env` values, [`BUNDLE_PLACEHOLDER`] stands for the bundle's directory.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IpcCommand {
    /// The executable: a name looked up on `PATH` (`python3`), or a path inside the
    /// bundle (`{bundle}/helper`).
    pub program: &'static str,
    pub args: &'static [&'static str],
    /// Extra environment variables, e.g. `PYTHONPATH` or `NODE_PATH` pointing into
    /// the bundle.
    pub env: &'static [(&'static str, &'static str)],
    /// Arguments `calcc build` runs `program` with to check it works on the build
    /// machine (it must exit 0). For a `program` inside the bundle, `calcc build`
    /// first checks the file exists, then runs the probe if there is one.
    pub probe: Option<&'static [&'static str]>,
    /// The `target_family` (`"windows"` / `"unix"`) this command is for; `None`
    /// means every platform.
    pub only_on: Option<&'static str>,
}

/// Stands for the bundle's directory in an [`IpcCommand`]'s `program`, `args` and
/// `env` values.
pub const BUNDLE_PLACEHOLDER: &str = "{bundle}";

/// An [`IpcCommand`] with the bundle directory filled in, ready to spawn.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedCommand {
    pub program: String,
    pub args: Vec<String>,
    pub env: Vec<(String, String)>,
}

impl IpcCommand {
    /// Whether the executable itself lives inside the bundle rather than on `PATH`.
    /// Only `program` decides that: `python3 {bundle}` still needs `python3` on
    /// `PATH`, however much of `args` or `env` points into the bundle.
    pub fn runs_from_bundle(&self) -> bool {
        self.program.contains(BUNDLE_PLACEHOLDER)
    }

    /// Whether anything in the command — `program`, `args` or `env` — refers to the
    /// bundle. Must be true exactly when the built-in declares a bundle: `{bundle}`
    /// without one would be filled in with an empty path, and a bundle nothing refers
    /// to would ship in every executable for nothing.
    pub fn uses_bundle(&self) -> bool {
        let refers = |s: &str| s.contains(BUNDLE_PLACEHOLDER);
        refers(self.program)
            || self.args.iter().any(|a| refers(a))
            || self.env.iter().any(|(_, v)| refers(v))
    }

    /// Fills [`BUNDLE_PLACEHOLDER`] in with `bundle_dir`.
    pub fn resolve(&self, bundle_dir: &str) -> ResolvedCommand {
        let fill = |s: &str| s.replace(BUNDLE_PLACEHOLDER, bundle_dir);
        ResolvedCommand {
            program: fill(self.program),
            args: self.args.iter().map(|a| fill(a)).collect(),
            env: self
                .env
                .iter()
                .map(|(k, v)| (k.to_string(), fill(v)))
                .collect(),
        }
    }
}

/// A system library an FFI link unit needs, optionally only on one platform.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LinkDep {
    /// Link name without prefix or extension (`m` → `-lm` / `m.lib`).
    pub name: &'static str,
    /// `Some("windows")` or `Some("unix")` restricts it to targets of that
    /// `target_family` (Rust's own `cfg(target_family)` vocabulary); `None` means
    /// every platform.
    pub only_on: Option<&'static str>,
}

impl LinkDep {
    /// Whether this dependency applies when linking for `target_family`.
    pub fn applies_to(&self, target_family: &str) -> bool {
        self.only_on.is_none_or(|family| family == target_family)
    }
}

/// Something a *produced executable* needs from the machine it runs on, beyond
/// itself. Reported by `calcc build` so a program's external dependencies are never
/// a surprise (spec.md §7: IPC built-ins trade away "zero run-time dependencies").
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuntimeRequirement {
    /// This executable must be on `PATH`.
    CommandOnPath(&'static str),
    /// The embedded bundle of this name is unpacked on first run, into a per-user
    /// cache directory that must be writable.
    BundleCache(&'static str),
}

impl fmt::Display for RuntimeRequirement {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RuntimeRequirement::CommandOnPath(program) => write!(f, "`{program}` on PATH"),
            RuntimeRequirement::BundleCache(bundle) => write!(
                f,
                "a writable per-user cache directory (or CALC_BUNDLE_CACHE) to unpack \
                 bundle `{bundle}` into on first run"
            ),
        }
    }
}

impl Builtin {
    /// The IPC command for `target_family`, for an IPC built-in: the one entry of
    /// `commands` whose `only_on` matches. `None` for other kinds, or if the manifest
    /// declares no command for that platform.
    pub fn ipc_command(&self, target_family: &str) -> Option<&'static IpcCommand> {
        let BindingKind::Ipc { commands, .. } = self.kind else {
            return None;
        };
        commands
            .iter()
            .find(|c| c.only_on.is_none_or(|family| family == target_family))
    }

    /// What a program calling this built-in needs at run time on `target_family`:
    /// exactly what will be used, never a list of alternatives. One method for every
    /// kind, so a new kind of run-time dependency (e.g. a shared library, once FFI
    /// built-ins can be linked dynamically — roadmap.md C7) is reported the same way.
    pub fn runtime_requirements(&self, target_family: &str) -> Vec<RuntimeRequirement> {
        let BindingKind::Ipc { bundle, .. } = self.kind else {
            return Vec::new();
        };
        let mut requirements = Vec::new();
        if let Some(command) = self.ipc_command(target_family) {
            if !command.runs_from_bundle() {
                requirements.push(RuntimeRequirement::CommandOnPath(command.program));
            }
        }
        if let Some(bundle) = bundle {
            requirements.push(RuntimeRequirement::BundleCache(bundle));
        }
        requirements
    }
}

/// The platforms a built-in's IPC commands must cover: every `target_family`
/// calc-lang links for.
pub const TARGET_FAMILIES: &[&str] = &["unix", "windows"];

/// The environment `print`'s Python commands run with. Without it, Python writes
/// compiled bytecode (`__pycache__/`) next to the modules it imports — into the
/// bundle's source directory when the interpreter runs it from there, where
/// `calcc build` would then pack it into every executable.
const PYTHON_ENV: &[(&str, &str)] = &[("PYTHONDONTWRITEBYTECODE", "1")];

/// Every built-in calc-lang knows about.
pub static BUILTINS: &[Builtin] = &[
    Builtin {
        name: "add",
        symbol: "calc_add",
        arity: 2,
        kind: BindingKind::NativeRust,
    },
    Builtin {
        name: "mul",
        symbol: "calc_mul",
        arity: 2,
        kind: BindingKind::NativeRust,
    },
    Builtin {
        name: "sub",
        symbol: "calc_sub",
        arity: 2,
        kind: BindingKind::Ffi {
            library: "calc_ffi",
            // Plain arithmetic in C needs nothing beyond the C runtime, which the
            // link driver always supplies.
            link_deps: &[],
        },
    },
    Builtin {
        name: "print",
        symbol: "calc_print",
        arity: 1,
        kind: BindingKind::Ipc {
            // A multi-file Python project, run as `python <bundle dir>` (Python runs
            // a directory's `__main__.py`). One interpreter name per platform, the
            // name each platform's standard install provides — not a list to try:
            // `python3` on Unix; `python` on Windows, where `python3` may be a
            // do-nothing "app execution alias" (found in session A11).
            commands: &[
                IpcCommand {
                    program: "python3",
                    args: &["{bundle}"],
                    env: PYTHON_ENV,
                    probe: Some(&["--version"]),
                    only_on: Some("unix"),
                },
                IpcCommand {
                    program: "python",
                    args: &["{bundle}"],
                    env: PYTHON_ENV,
                    probe: Some(&["--version"]),
                    only_on: Some("windows"),
                },
            ],
            bundle: Some("print"),
        },
    },
];

/// Finds a built-in by its DSL-visible name.
pub fn lookup(name: &str) -> Option<&'static Builtin> {
    BUILTINS.iter().find(|b| b.name == name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lookup_finds_declared_builtins_only() {
        assert_eq!(lookup("add").map(|b| b.symbol), Some("calc_add"));
        assert!(lookup("nope").is_none());
    }

    #[test]
    fn names_and_symbols_are_unique() {
        for (i, a) in BUILTINS.iter().enumerate() {
            for b in &BUILTINS[i + 1..] {
                assert_ne!(a.name, b.name);
                assert_ne!(a.symbol, b.symbol);
            }
        }
    }

    /// No guessing at run time: every IPC built-in has exactly one command per
    /// platform.
    #[test]
    fn every_ipc_builtin_has_exactly_one_command_per_platform() {
        for builtin in BUILTINS {
            let BindingKind::Ipc { commands, .. } = builtin.kind else {
                continue;
            };
            for family in TARGET_FAMILIES {
                let matching = commands
                    .iter()
                    .filter(|c| c.only_on.is_none_or(|f| f == *family))
                    .count();
                assert_eq!(matching, 1, "`{}` on {family}", builtin.name);
            }
        }
    }

    /// `{bundle}` appears in a command exactly when the built-in declares a bundle.
    #[test]
    fn every_ipc_command_uses_its_bundle_iff_it_has_one() {
        for builtin in BUILTINS {
            let BindingKind::Ipc { commands, bundle } = builtin.kind else {
                continue;
            };
            for command in commands {
                assert_eq!(
                    command.uses_bundle(),
                    bundle.is_some(),
                    "`{}`: {command:?}",
                    builtin.name
                );
            }
        }
    }

    #[test]
    fn uses_bundle_looks_at_program_args_and_env_but_runs_from_bundle_only_at_program() {
        let plain = IpcCommand {
            program: "node",
            args: &["script.js"],
            env: &[],
            probe: None,
            only_on: None,
        };
        assert!(!plain.uses_bundle());
        let in_args = IpcCommand {
            args: &["{bundle}/index.js"],
            ..plain
        };
        assert!(in_args.uses_bundle() && !in_args.runs_from_bundle());
        let in_env = IpcCommand {
            env: &[("NODE_PATH", "{bundle}/node_modules")],
            ..plain
        };
        assert!(in_env.uses_bundle() && !in_env.runs_from_bundle());
        let in_program = IpcCommand {
            program: "{bundle}/helper",
            ..plain
        };
        assert!(in_program.uses_bundle() && in_program.runs_from_bundle());
    }

    #[test]
    fn runtime_requirements_name_exactly_what_will_run() {
        assert!(lookup("add")
            .unwrap()
            .runtime_requirements("unix")
            .is_empty());
        assert!(lookup("sub")
            .unwrap()
            .runtime_requirements("windows")
            .is_empty());
        let print = lookup("print").unwrap();
        assert_eq!(
            print.runtime_requirements("unix"),
            [
                RuntimeRequirement::CommandOnPath("python3"),
                RuntimeRequirement::BundleCache("print")
            ]
        );
        assert_eq!(
            print.runtime_requirements("windows")[0],
            RuntimeRequirement::CommandOnPath("python")
        );
    }

    /// A program shipped inside the bundle needs no `PATH` entry.
    #[test]
    fn a_program_inside_the_bundle_is_not_a_path_requirement() {
        const HELPER: Builtin = Builtin {
            name: "helper",
            symbol: "calc_helper",
            arity: 1,
            kind: BindingKind::Ipc {
                commands: &[IpcCommand {
                    program: "{bundle}/helper",
                    args: &[],
                    env: &[],
                    probe: None,
                    only_on: None,
                }],
                bundle: Some("helper"),
            },
        };
        assert_eq!(
            HELPER.runtime_requirements("unix"),
            [RuntimeRequirement::BundleCache("helper")]
        );
    }

    #[test]
    fn resolve_fills_the_bundle_directory_into_program_args_and_env() {
        let command = IpcCommand {
            program: "{bundle}/bin/node",
            args: &["{bundle}/index.js", "--flag"],
            env: &[("NODE_PATH", "{bundle}/node_modules")],
            probe: None,
            only_on: None,
        };
        assert_eq!(
            command.resolve("/cache/b"),
            ResolvedCommand {
                program: "/cache/b/bin/node".into(),
                args: vec!["/cache/b/index.js".into(), "--flag".into()],
                env: vec![("NODE_PATH".into(), "/cache/b/node_modules".into())],
            }
        );
    }

    #[test]
    fn link_deps_filter_by_target_family() {
        let everywhere = LinkDep {
            name: "m",
            only_on: None,
        };
        let windows_only = LinkDep {
            name: "ws2_32",
            only_on: Some("windows"),
        };
        assert!(everywhere.applies_to("unix"));
        assert!(everywhere.applies_to("windows"));
        assert!(windows_only.applies_to("windows"));
        assert!(!windows_only.applies_to("unix"));
    }
}
