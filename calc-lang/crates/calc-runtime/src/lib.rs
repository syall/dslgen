//! Native Rust built-ins for calc-lang (spec.md §7, kind 1; session A9).
//!
//! This crate is the one place a built-in's behavior is written down. It serves two
//! consumers from the same source:
//!
//! * the interpreter (`calc-ir`) calls [`Builtin::eval`] like any Rust function;
//! * compiled programs call the `#[no_mangle] extern "C"` function named by
//!   [`Builtin::symbol`], which `calcc build`'s linker step finds in a static library
//!   built from this very file (see `calc-compiler/build.rs`).
//!
//! **This file must stay self-contained** (no dependencies, no `use` of other crates):
//! `build.rs` compiles it on its own with plain `rustc`.
//!
//! The manifest is hardcoded for now; a data-driven `bindings.toml` is Part B's job
//! (roadmap.md B-series). See `calc-lang/docs/a9-native-rust-builtins.md`.

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

/// One entry of the built-in manifest: the DSL-visible name, the linker symbol
/// compiled code calls, the argument count, and a Rust-side entry point for the
/// interpreter.
pub struct Builtin {
    pub name: &'static str,
    pub symbol: &'static str,
    pub arity: usize,
    /// Interpreter entry point; `args.len()` always equals `arity`.
    pub eval: fn(&[f64]) -> f64,
}

/// Every built-in calc-lang knows about. All take and return `f64`, calc-lang's only
/// type, so a signature is just an arity.
pub static BUILTINS: &[Builtin] = &[
    Builtin {
        name: "add",
        symbol: "calc_add",
        arity: 2,
        eval: |args| calc_add(args[0], args[1]),
    },
    Builtin {
        name: "mul",
        symbol: "calc_mul",
        arity: 2,
        eval: |args| calc_mul(args[0], args[1]),
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
    fn eval_matches_the_extern_functions() {
        assert_eq!((lookup("add").unwrap().eval)(&[1.0, 2.0]), 3.0);
        assert_eq!((lookup("mul").unwrap().eval)(&[3.0, 4.0]), 12.0);
    }
}
