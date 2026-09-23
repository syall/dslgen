/* An empty IPC bundle table (session A12), linked only into calc-runtime's Rust
 * consumers — the interpreter and test harnesses — by this crate's build.rs.
 * `calc_print` reads its bundle through these two symbols; in a compiled .calc
 * program the link driver (calc-compiler/src/link.rs) defines them instead, in a
 * generated data object holding the program's real bundles. Rust consumers never
 * read them (the interpreter runs bundles from their source directory), but the
 * reference inside `calc_print` still has to resolve. */

const unsigned char calc_ipc_bundles[1] = {0};
const unsigned long long calc_ipc_bundles_len = 0;
