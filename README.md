# dslgen

[![test](https://github.com/syall/dslgen/actions/workflows/agent-evals.yml/badge.svg)](https://github.com/syall/dslgen/actions/workflows/agent-evals.yml)
[![License: MIT OR Apache-2.0](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue.svg)](#license)

DSL-Generator lets an author supply a grammar, semantic actions, role
annotations, and built-in bindings, and get back a working, ahead-of-time-
compiled compiler and a baseline LSP for that DSL — without hand-writing a
parser, typechecker, or codegen backend themselves.

From those inputs, DSL-Generator produces a self-contained Rust workspace
containing a generated parser, a semantic-analysis library (AST, IR lowering,
symbol table), a compiler binary that lowers to native code via pluggable
codegen backends (LLVM and/or Cranelift), and a language server binary with
diagnostics, semantic highlighting, and symbol navigation out of the box.

See [intent.md](intent.md) for why this project exists and the constraints
any design must honor, and [spec.md](spec.md) for the formal design derived
from it.

## Status

Early and under active development. The project is being built in stages (see
[roadmap.md](roadmap.md)): first hand-building a single concrete DSL,
[`calc-lang`](calc-lang), entirely by hand, before extracting the generic
`dslgen` meta-tool from that working pipeline. There is no generic generator
yet — only `calc-lang`'s hand-built parser frontend.

## Repository layout

- [intent.md](intent.md) — why this project exists and what constraints any
  design must honor.
- [spec.md](spec.md) — the formal design spec, derived from intent.md.
- [roadmap.md](roadmap.md) — the session-by-session build order.
- [calc-lang/](calc-lang) — the hand-built DSL used to prove out the design
  before it's generalized. A Cargo workspace (resolver 2, edition 2021):
  - `crates/calc-syntax` — parser frontend(s) behind a `ParserFrontend` trait.
  - `crates/calc-ir` — typed AST/IR types and lowering.
  - `crates/calc-compiler` — the `calcc` compiler binary and codegen backends.
  - `crates/calc-lsp` — the `calc-lsp` language server binary.
  - `calc-lang/DECISIONS.md` — decision log for choices made while building
    `calc-lang`.

## Building

```sh
cd calc-lang
cargo build
cargo test
```

## Contributing

This project is still finding its shape — issues and discussion are welcome,
but expect the design to move quickly while Part A (`calc-lang`) is being
built by hand. See [roadmap.md](roadmap.md) for the current stage of work.

## License

Licensed under either of

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE) or
  <http://www.apache.org/licenses/LICENSE-2.0>)
- MIT license ([LICENSE-MIT](LICENSE-MIT) or
  <http://opensource.org/licenses/MIT>)

at your option.
