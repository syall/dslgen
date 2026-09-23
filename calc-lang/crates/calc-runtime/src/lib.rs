//! calc-lang's built-in manifest (spec.md §7): every built-in's name/symbol/arity, plus
//! whichever kinds' real implementations are expressible directly in this crate.
//!
//! Native Rust built-ins (kind 1; session A9) are defined right here as plain
//! `#[no_mangle] extern "C"` functions. C-ABI FFI built-ins (kind 2; session A10) are
//! defined in `native/calc_ffi.c`, declared here via an `extern "C"` block, and linked
//! in by this crate's own `build.rs` (using the `cc` crate). Subprocess/IPC built-ins
//! (kind 3; session A11) are defined in `ipc_runtime.rs`, wrapped here the same way
//! kind 1's functions are — a real, self-contained Rust definition, not an `extern`
//! declaration — because unlike kind 2's C library, there's no separate ABI boundary
//! to cross; spawning a subprocess is plain Rust. [`Builtin::eval`] calls the *same*
//! function compiled code calls, for all three kinds, never a reimplementation. This
//! crate serves multiple consumers from the same source:
//!
//! * the interpreter (`calc-ir`) calls [`Builtin::eval`];
//! * compiled programs call the symbol named by [`Builtin::symbol`], which
//!   `calcc build`'s linker step finds in static libraries `calc-compiler/build.rs`
//!   independently builds from this crate's sources (`src/lib.rs` via a raw `rustc`
//!   invocation for kind 1, `native/calc_ffi.c` via `cc` for kind 2).
//!
//! **`src/lib.rs` must stay self-contained** (no `use` of other crates): the kind-1
//! static-library build above compiles this file on its own with plain `rustc`, outside
//! Cargo's dependency machinery. `native/calc_ffi.c` and this crate's own `build.rs`
//! aren't subject to that constraint — they're irrelevant to that special build. This
//! extends to `ipc_runtime.rs` too, since `rustc` follows this file's `mod` declarations
//! regardless of Cargo: it's written using only `std`, deliberately not `serde_json`
//! despite roadmap.md's A11 entry suggesting it (see `ipc_runtime.rs`'s own doc comment
//! and `calc-lang/DECISIONS.md`'s A11 entry for the alternatives considered and why this
//! dependency-free requirement is a temporary accommodation, not a permanent design goal
//! for any of the three binding kinds).
//!
//! The manifest is hardcoded for now, including which kind backs each row; a
//! data-driven `bindings.toml` (Part B, roadmap.md B6) replaces this whole table, so
//! this crate's scope narrows back down to "native Rust built-ins' bodies" once that
//! lands. See `calc-lang/docs/a9-native-rust-builtins.md` and
//! `calc-lang/docs/a10-c-abi-ffi-builtins.md`.

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
    // `calc_sub`'s only definition is in `native/calc_ffi.c`, compiled and linked in by
    // this crate's `build.rs`. Declaring it `extern "C"` (with no body) tells `rustc`
    // "this symbol is resolved by the linker", the same C-ABI mechanism `calc_add`'s
    // `extern "C"` uses, just crossed in the other direction (calling into C, not
    // exposing Rust to it). Rustdoc doesn't render docs on extern blocks, hence a plain
    // comment rather than `///`.
    fn calc_sub(a: f64, b: f64) -> f64;
}

/// Prints its argument via a subprocess (`ipc_runtime::call`, backed by
/// `native/print.py`) and returns it unchanged. `print` is called as a statement
/// (`calc-syntax::Stmt::Print`, session A11), so calc-lang itself never reads this
/// return value — kept anyway, matching the other two kinds' built-ins, rather
/// than adding a genuinely `void`-returning call path for one built-in whose
/// return value is unreachable either way (see `calc-lang/DECISIONS.md`'s A11
/// entry for the tradeoff). `#[no_mangle] extern "C"` for the same reason as
/// [`calc_add`]/[`calc_mul`]: a real, self-contained definition right here, not an
/// `extern` declaration — kind 3 has no separate ABI boundary to cross the way
/// kind 2's C library does.
#[no_mangle]
pub extern "C" fn calc_print(x: f64) -> f64 {
    ipc_runtime::call(x)
}

/// Which of spec.md §7's three binding kinds backs a built-in. This determines where
/// its `symbol` is actually *defined* — not how compiled code calls it, which is an
/// ordinary linked `call` either way.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BindingKind {
    /// Defined right here in `src/lib.rs`, as a plain `#[no_mangle] extern "C"` Rust
    /// function (session A9).
    NativeRust,
    /// Defined in `native/calc_ffi.c`, a separately compiled C library (session A10).
    Ffi,
    /// Backed by an external subprocess (`native/print.py`; session A11). In this
    /// session's implementation it happens to share kind 1's build mechanics (a
    /// self-contained Rust definition, no separate compiled library) — that's
    /// coincidental, not a defining property of the kind: `bindings.toml` (Part B)
    /// will still record it as a distinct kind, and nothing here assumes an IPC
    /// built-in must stay dependency-free forever (see `ipc_runtime.rs`'s doc
    /// comment and `calc-lang/DECISIONS.md`'s A11 entry).
    Ipc,
}

/// One entry of the built-in manifest: the DSL-visible name, the linker symbol
/// compiled code calls, the argument count, which kind backs it, and a Rust-side entry
/// point for the interpreter.
pub struct Builtin {
    pub name: &'static str,
    pub symbol: &'static str,
    pub arity: usize,
    pub kind: BindingKind,
    /// Interpreter entry point; `args.len()` always equals `arity`. Always calls the
    /// same real implementation `symbol` names — never a reimplementation of it — so
    /// the interpreter and compiled code can't silently disagree on what a built-in
    /// does.
    pub eval: fn(&[f64]) -> f64,
}

/// Every built-in calc-lang knows about. All take and return `f64`, calc-lang's only
/// type, so a signature is just an arity.
pub static BUILTINS: &[Builtin] = &[
    Builtin {
        name: "add",
        symbol: "calc_add",
        arity: 2,
        kind: BindingKind::NativeRust,
        eval: |args| calc_add(args[0], args[1]),
    },
    Builtin {
        name: "mul",
        symbol: "calc_mul",
        arity: 2,
        kind: BindingKind::NativeRust,
        eval: |args| calc_mul(args[0], args[1]),
    },
    Builtin {
        name: "sub",
        symbol: "calc_sub",
        arity: 2,
        kind: BindingKind::Ffi,
        eval: |args| unsafe { calc_sub(args[0], args[1]) },
    },
    Builtin {
        name: "print",
        symbol: "calc_print",
        arity: 1,
        kind: BindingKind::Ipc,
        eval: |args| calc_print(args[0]),
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

    /// `sub`'s `eval` calls the real, C-defined `calc_sub` (linked in by this crate's
    /// own `build.rs`), not a Rust reimplementation of subtraction.
    #[test]
    fn eval_calls_the_real_ffi_function() {
        let sub = lookup("sub").unwrap();
        assert_eq!(sub.kind, BindingKind::Ffi);
        assert_eq!((sub.eval)(&[5.0, 3.0]), 2.0);
    }

    /// `print`'s `eval` calls the real, subprocess-backed `calc_print` (session
    /// A11), same "call the real implementation" principle as the FFI test above.
    #[test]
    fn eval_calls_the_real_ipc_function() {
        let print = lookup("print").unwrap();
        assert_eq!(print.kind, BindingKind::Ipc);
        assert_eq!((print.eval)(&[2.5]), 2.5);
    }
}
