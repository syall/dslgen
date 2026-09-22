/* calc-lang's one C-ABI FFI built-in (spec.md §7, kind 2; session A10): a tiny C
 * library, compiled and linked in by calc-runtime's own build.rs (for the
 * interpreter and other ordinary Rust consumers) and, separately, by
 * calc-compiler's build.rs (for link_stub.rs to link into compiled programs).
 * See calc-runtime/src/lib.rs's BUILTINS entry for "sub". */

double calc_sub(double a, double b) {
    return a - b;
}
