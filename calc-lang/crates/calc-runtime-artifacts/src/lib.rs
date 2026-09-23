//! Where the link units for compiled `.calc` programs are, and what they need
//! (session A12). This crate's `build.rs` does the work — building `calc-runtime` as
//! a static archive with a nested Cargo build and recording rustc's own list of that
//! archive's native dependencies — and this library just hands the results to the
//! link driver (`calc-compiler/src/link.rs`).
//!
//! Nothing here is specific to calc-lang's built-ins: it would work unchanged for
//! any runtime crate, which is why Part B lifts it into the shared `dslgen-backend`.
//! See `calc-lang/docs/a12-the-link-driver.md`.

/// The target triple every artifact here was built for. Today always the host `calcc`
/// itself was built for; cross-compilation (roadmap.md C8) makes it selectable.
pub const TARGET: &str = env!("CALC_RUNTIME_TARGET");

/// Path to `calc-runtime`'s static archive: the native-Rust built-ins and the IPC
/// shim, plus everything they use from `std` and their dependencies.
pub const RUNTIME_LIB: &str = env!("CALC_RUNTIME_LIB");

/// rustc's own list of the native libraries [`RUNTIME_LIB`] needs on [`TARGET`], as
/// linker arguments (e.g. `ws2_32.lib ... /defaultlib:libcmt` on MSVC, `-lgcc_s -lc
/// ...` on Linux). Baked in at build time, so it can never disagree with the archive.
pub const RUNTIME_LINK_DEPS: &str = include_str!(env!("CALC_RUNTIME_LINK_DEPS_FILE"));

/// Directory holding FFI link units (`calc_ffi` today), each named by the library
/// name the manifest declares. Published by `calc-runtime`'s build script.
pub const LINK_UNITS_DIR: &str = env!("CALC_LINK_UNITS_DIR");

/// Directory holding IPC bundles' source directories (`print` today), each named by
/// the bundle name the manifest declares. `calcc build` packs the ones a program uses
/// and embeds them in the executable. Published by `calc-runtime`'s build script.
pub const IPC_BUNDLES_DIR: &str = env!("CALC_IPC_BUNDLES_DIR");

/// [`RUNTIME_LINK_DEPS`] split into individual linker arguments.
pub fn runtime_link_deps() -> impl Iterator<Item = &'static str> {
    RUNTIME_LINK_DEPS.split_whitespace()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runtime_archive_and_directories_exist() {
        assert!(std::path::Path::new(RUNTIME_LIB).is_file());
        assert!(std::path::Path::new(LINK_UNITS_DIR).is_dir());
        assert!(std::path::Path::new(IPC_BUNDLES_DIR).is_dir());
    }

    /// rustc always reports at least the C library for a `std`-using archive.
    #[test]
    fn runtime_link_deps_are_reported() {
        assert!(
            runtime_link_deps().next().is_some(),
            "{RUNTIME_LINK_DEPS:?}"
        );
    }
}
