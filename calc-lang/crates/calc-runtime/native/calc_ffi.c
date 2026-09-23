/* calc-lang's one C-ABI FFI built-in (spec.md §7, kind 2; session A10): a tiny C
 * library, compiled by calc-runtime's own build.rs twice — once linked into
 * ordinary Rust consumers (the interpreter), once published as the `calc_ffi`
 * link unit the link driver (calc-compiler/src/link.rs) links into compiled
 * programs. See calc-builtins' BUILTINS entry for "sub". */

double calc_sub(double a, double b) {
    return a - b;
}
