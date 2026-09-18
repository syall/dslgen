# calc-lang teaching docs

This is DSL-Generator's teaching-documentation series (spec.md §12): one page per
build session, written so someone with no prior compiler background can follow it to
understand what that stage of a real, working compiler does and why — using
`calc-lang`'s actual code as the running example, not toy snippets.

Read in order:

1. [a0-workspace-and-parser-decision.md](a0-workspace-and-parser-decision.md) — why a
   Cargo workspace split into four crates, and why the parsing layer is a trait rather
   than a hardcoded library choice.

More pages land as later sessions in [../../roadmap.md](../../roadmap.md) land.
