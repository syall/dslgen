//! The `Backend` trait and runtime backend selection (spec.md §8.1, session A8).
//!
//! Every codegen backend turns `calc-ir`'s `Program` into object-file bytes; `calcc`
//! only ever talks to that contract, so it doesn't know which backends exist. Which
//! ones are *compiled in* is decided by Cargo features (`backend-cranelift`,
//! `backend-llvm`); which one *runs* is decided by `--backend=<name>` via [`select`].
//! Adding a backend later means one new `impl Backend` and one `#[cfg]`-gated line in
//! [`enabled`] — nothing upstream of codegen changes. See
//! `calc-lang/docs/a8-backend-trait-and-feature-gating.md`.

use std::fmt;

use calc_ir::Program;

#[cfg(not(any(feature = "backend-cranelift", feature = "backend-llvm")))]
compile_error!(
    "enable at least one codegen backend feature: `backend-cranelift` or `backend-llvm`"
);

/// A backend failure, reported to the user by `calcc`. Today's backends panic on
/// internal bugs rather than returning this (see `DECISIONS.md`'s A8 entry); it's
/// here so the trait's contract already allows failure.
#[derive(Debug, PartialEq, Eq)]
pub struct BackendError(pub String);

impl fmt::Display for BackendError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// The contract every codegen backend satisfies: consume the mid-level IR, produce
/// an object file's bytes.
pub trait Backend {
    /// The name `--backend=<name>` selects this backend by.
    fn name(&self) -> &'static str;

    /// Compiles `program` to the bytes of a native object file.
    fn compile(&self, program: &Program) -> Result<Vec<u8>, BackendError>;
}

/// Every backend name `calcc` knows about, compiled in or not — lets [`select`] tell
/// "unknown backend" apart from "known, but this build left it out".
const KNOWN: [&str; 2] = ["cranelift", "llvm"];

/// The backends compiled into this build, in a fixed order.
pub fn enabled() -> Vec<&'static dyn Backend> {
    vec![
        #[cfg(feature = "backend-cranelift")]
        &crate::cranelift_backend::CraneliftBackend,
        #[cfg(feature = "backend-llvm")]
        &crate::llvm_backend::LlvmBackend,
    ]
}

/// Picks the backend for `--backend=<name>`. With no name, the sole enabled backend
/// is the implicit default; if several are enabled the user must choose.
pub fn select(name: Option<&str>) -> Result<&'static dyn Backend, BackendError> {
    select_from(&enabled(), name)
}

fn select_from<'a>(
    enabled: &[&'a dyn Backend],
    name: Option<&str>,
) -> Result<&'a dyn Backend, BackendError> {
    let names: Vec<&str> = enabled.iter().map(|b| b.name()).collect();
    match name {
        Some(name) => enabled
            .iter()
            .find(|b| b.name() == name)
            .copied()
            .ok_or_else(|| {
                if KNOWN.contains(&name) {
                    BackendError(format!(
                        "this build has no {name} backend; rebuild with `--features backend-{name}` (enabled: {})",
                        names.join(", ")
                    ))
                } else {
                    BackendError(format!(
                        "unknown backend `{name}` (enabled: {})",
                        names.join(", ")
                    ))
                }
            }),
        None => match enabled {
            [only] => Ok(*only),
            _ => Err(BackendError(format!(
                "several backends are enabled; pick one with --backend=<{}>",
                names.join("|")
            ))),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Fake(&'static str);

    impl Backend for Fake {
        fn name(&self) -> &'static str {
            self.0
        }
        fn compile(&self, _: &Program) -> Result<Vec<u8>, BackendError> {
            Ok(Vec::new())
        }
    }

    #[test]
    fn a_sole_backend_is_the_implicit_default() {
        let only = Fake("cranelift");
        let picked = select_from(&[&only], None).unwrap();
        assert_eq!(picked.name(), "cranelift");
    }

    #[test]
    fn several_backends_need_an_explicit_choice() {
        let (a, b) = (Fake("cranelift"), Fake("llvm"));
        let err = select_from(&[&a, &b], None).err().unwrap();
        assert!(err.0.contains("cranelift|llvm"), "{err}");
        assert_eq!(select_from(&[&a, &b], Some("llvm")).unwrap().name(), "llvm");
    }

    #[test]
    fn a_known_but_disabled_backend_says_how_to_enable_it() {
        let a = Fake("cranelift");
        let err = select_from(&[&a], Some("llvm")).err().unwrap();
        assert!(err.0.contains("--features backend-llvm"), "{err}");
    }

    #[test]
    fn an_unknown_backend_lists_whats_enabled() {
        let a = Fake("cranelift");
        let err = select_from(&[&a], Some("wasm")).err().unwrap();
        assert!(err.0.contains("unknown backend `wasm`"), "{err}");
        assert!(err.0.contains("enabled: cranelift"), "{err}");
    }
}
