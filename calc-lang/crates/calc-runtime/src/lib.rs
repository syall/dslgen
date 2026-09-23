//! The real implementations of calc-lang's built-ins (spec.md §7), for every kind
//! that's expressible in this crate. *What* each built-in is — name, symbol, arity,
//! kind, dependencies — lives in the `calc-builtins` manifest (session A12); this
//! crate is only the code behind it.
//!
//! * **Native Rust** (kind 1; session A9): `calc_add`/`calc_mul`, plain
//!   `#[no_mangle] extern "C"` functions right here.
//! * **C-ABI FFI** (kind 2; session A10): `calc_sub`, defined in `native/calc_ffi.c`
//!   and only *declared* here. With the default `link-natives` feature, this crate's
//!   `build.rs` compiles and links it for Rust callers, and also publishes it as a
//!   separate link unit for compiled programs.
//! * **Subprocess/IPC** (kind 3; sessions A11, A12): `calc_print`, a Rust shim
//!   (`ipc_runtime.rs`) that runs the multi-file Python project in `ipc/print/` and
//!   talks to it over a JSON protocol.
//!
//! Two consumers use the same functions, never copies of them:
//!
//! * the interpreter (`calc-ir`) calls them through [`eval`];
//! * compiled programs call their symbols, which the link driver finds in this crate
//!   built as a static archive by `calc-runtime-artifacts` (a real Cargo build, so
//!   this crate may use ordinary dependencies — `serde_json`, here — since A12).
//!
//! # Cargo features
//!
//! **`link-natives`** (on by default) decides where this crate's two C-side symbols
//! come from. Two of them are only *declared* in Rust: `calc_sub`, and
//! `calc_ipc_bundles`/`calc_ipc_bundles_len`, the IPC bundle table `calc_print` reads.
//!
//! | | `link-natives` on (default) | `link-natives` off |
//! |---|---|---|
//! | Who builds it this way | every ordinary dependent: `calc-ir` (the interpreter), `calcc`, every test harness | only `calc-runtime-artifacts`' nested build of the static archive (`--no-default-features`) |
//! | `calc_sub` | compiled from `native/calc_ffi.c` by `build.rs` and linked in | left undefined; the separate `calc_ffi` link unit supplies it when a program is linked |
//! | `calc_ipc_bundles` | an empty table, from `native/no_ipc_bundles.c` | left undefined; the link driver's generated bundle object supplies it |
//! | Also produced | the `calc_ffi` link unit for compiled programs (static C runtime), published as `link_units_dir` | nothing extra |
//!
//! Either way, `build.rs` publishes `manifest_dir` and `ipc_dir` for dependents.
//!
//! Leave it on in any ordinary dependent: turning it off there leaves those symbols
//! undefined, and linking that binary fails. It exists for the static archive, which
//! must leave them to the link units that really define them in a compiled program.
//! With the feature on, a static archive bundles the C libraries its crate links, so
//! it would carry its own `calc_sub`. The pre-link check would reject that as
//! exported twice; without the check, it would silently win over a swapped-in FFI
//! library. The nested build is a separate `cargo` invocation, so Cargo's feature
//! unification (a feature turned on for one dependent is on for every dependent in
//! that build) never mixes the two configurations.
//!
//! See `calc-lang/docs/a12-the-link-driver.md`.

mod ipc_runtime;

/// Adds two numbers. `extern "C"` fixes the calling convention to the platform's C
/// ABI so compiled code can call it; `#[no_mangle]` keeps the symbol named exactly
/// `calc_add` instead of Rust's decorated name.
#[no_mangle]
pub extern "C" fn calc_add(a: f64, b: f64) -> f64 {
    a + b
}

/// Multiplies two numbers; see [`calc_add`] for what the attributes do.
#[no_mangle]
pub extern "C" fn calc_mul(a: f64, b: f64) -> f64 {
    a * b
}

extern "C" {
    // `calc_sub`'s only definition is in `native/calc_ffi.c`. Declaring it `extern
    // "C"` (with no body) tells `rustc` "this symbol is resolved by the linker": by
    // this crate's `build.rs` for Rust callers, and by the separate `calc_ffi` link
    // unit for compiled programs. Rustdoc doesn't render docs on extern blocks,
    // hence a plain comment rather than `///`.
    fn calc_sub(a: f64, b: f64) -> f64;
}

/// Prints its argument via a subprocess — the `print` project under `ipc/print/`,
/// run by `ipc_runtime` with the command the manifest declares for this platform —
/// and returns the value the subprocess sends back (its argument, unchanged). This is
/// the symbol compiled programs call, so it runs the copy of the project embedded in
/// the executable.
#[no_mangle]
pub extern "C" fn calc_print(x: f64) -> f64 {
    ipc_runtime::call("print", x, ipc_runtime::BundleSource::Embedded)
}

/// The interpreter's entry point for the built-in named `name`: a function calling
/// the *same* implementation compiled code links against, never a reimplementation
/// of it, so the interpreter and compiled code can't silently disagree. (For `print`
/// that's the same shim and the same Python project as [`calc_print`], run from its
/// source directory rather than from a copy embedded in an executable.) The caller
/// checks `args.len()` against the manifest's arity.
pub fn eval(name: &str) -> Option<fn(&[f64]) -> f64> {
    let f: fn(&[f64]) -> f64 = match name {
        "add" => |args| calc_add(args[0], args[1]),
        "mul" => |args| calc_mul(args[0], args[1]),
        "sub" => |args| unsafe { calc_sub(args[0], args[1]) },
        "print" => |args| ipc_runtime::call("print", args[0], ipc_runtime::BundleSource::Source),
        _ => return None,
    };
    Some(f)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every manifest entry has an implementation here and vice versa, so the
    /// manifest and this crate can't drift apart.
    #[test]
    fn every_manifest_entry_has_an_implementation() {
        for builtin in calc_builtins::BUILTINS {
            assert!(
                eval(builtin.name).is_some(),
                "no eval for `{}`",
                builtin.name
            );
        }
        assert!(eval("nope").is_none());
    }

    #[test]
    fn eval_calls_the_native_rust_functions() {
        assert_eq!(eval("add").unwrap()(&[1.0, 2.0]), 3.0);
        assert_eq!(eval("mul").unwrap()(&[3.0, 4.0]), 12.0);
    }

    /// `sub`'s `eval` calls the real, C-defined `calc_sub`, not a Rust
    /// reimplementation of subtraction.
    #[test]
    fn eval_calls_the_real_ffi_function() {
        assert_eq!(eval("sub").unwrap()(&[5.0, 3.0]), 2.0);
    }

    /// `print`'s `eval` runs the real, subprocess-backed implementation.
    #[test]
    fn eval_calls_the_real_ipc_function() {
        assert_eq!(eval("print").unwrap()(&[2.5]), 2.5);
    }
}
