# Intent

*Plan-stage artifact (AI-native SDLC). Captures why this project exists and the
constraints any design must honor, ahead of formal specs and implementation plans.
Update this when the underlying goal or a hard constraint changes — not for
implementation detail, which belongs in spec.md and roadmap.md instead.*

## Problem

Building a small, purpose-built compiled language today means hand-writing a lexer,
parser, typechecker, codegen backend, and editor tooling (LSP) from scratch —
significant, repetitive engineering effort that discourages people from ever trying,
even when a purpose-built DSL would be the right tool for a problem.

## Goal

DSL-Generator lets an author supply a grammar, semantic actions, role annotations,
and built-in bindings, and get back a working, ahead-of-time-compiled compiler and a
baseline LSP for that DSL — without hand-writing a parser, typechecker, or codegen
backend themselves.

## Why this approach

- **Generate real compilers, not interpreters.** Native, ahead-of-time-compiled
  output is a hard requirement, not a stretch goal — the generated toolchain should
  be something people would actually ship.
- **Build on established libraries, don't reinvent them.** Parsing goes through an
  existing parser-generator (or a hand-rolled frontend the author supplies) behind a
  pluggable trait, and codegen goes through LLVM/Cranelift — DSL-Generator's value is
  the glue and the generic infrastructure built on top, not a competing parser or
  backend implementation.
- **Structural roles, not per-DSL special-casing.** Marking *what* a grammar rule
  means (identifier, keyword, control flow, scope) — not just how to build an AST
  from it — is what lets IR lowering, codegen, and the LSP stay generic across
  arbitrary DSLs instead of needing bespoke logic per language.
- **Prove it by hand first.** Before generalizing anything, build one real DSL
  (`calc-lang`) entirely by hand, hardcoding every piece. Only extract the generic
  `dslgen` meta-tool once there's a working concrete pipeline to extract it from —
  see roadmap.md's Part A/B/C split.

## Constraints

- Prefer small, focused, incremental changes over large rewrites, even where a
  bigger refactor might look more elegant in isolation (carried through into spec.md
  §2 and this project's day-to-day working style).
- Every built-in integration path (native Rust, C-ABI FFI, subprocess/IPC) is
  equally first-class — none is a second-class fallback.
- Diagnostics (grammar issues, semantic-action errors, binding mismatches, and
  per-DSL-program errors) must surface through both the compiler CLI and the
  generated LSP — never as opaque backend failures.

## Out of scope (for now)

- CI/CD pipelines, deployment gates, and production metrics tracking — there's no
  shipped artifact yet for these to govern. Revisit once Part A (`calc-lang`) has
  working code and a CI pipeline is actually needed.

## Source of truth once formalized

This document is the seed. spec.md is the formal, detailed design derived from it —
if the two disagree on rationale, treat that as a sign spec.md drifted from intent
and needs reconciling, not that this file is stale.
