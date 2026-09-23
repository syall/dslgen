# CLAUDE.md

Institutional knowledge for Claude Code sessions working on DSL-Generator. This
project follows the AI-native SDLC
([academy.claude.com/courses/ai-native-sdlc-playbook](https://academy.claude.com/courses/ai-native-sdlc-playbook)):
six stages, each with one artifact or convention below. Update this file — and
the relevant stage below — when a convention changes; keep entries short, detail
belongs in the docs they link to.

Cross-cutting: prefer small, focused, incremental changes over large rewrites —
carried through from [spec.md](spec.md) §2 and [intent.md](intent.md). Don't
restructure or generalize ahead of need.

## 1. Plan — intent.md

[intent.md](intent.md) captures why the project exists and the constraints any
design must honor. Update it when the underlying goal or a hard constraint
changes, not for implementation detail.

## 2. Design — spec.md

[spec.md](spec.md) is the formal design spec, derived from intent.md. Cite its
section(s) when a change implements spec'd behavior.

## 3. Build — plan mode, this file, skills, subagents

- Default to Claude Code's planning mode before non-trivial changes. Commit the
  approved plan as `plan.md` alongside that task's diff; update it in the same
  commit if implementation departs from it. [roadmap.md](roadmap.md) breaks the
  project into sessions — each session is one such task.
- `plan.md` lives at this fixed path and is overwritten by each session, not
  accumulated — it always reflects only the current/most recent session. Past
  sessions' plans aren't deleted, just no longer live: each commit permanently
  snapshots the `plan.md` it shipped with, so recover an old one with
  `git show <commit>:plan.md` or `git log -- plan.md` rather than looking for a
  separate file per session.
- Every session also adds its teaching-doc page to `calc-lang/docs/` (spec.md
  §12) in the same commit as the session's code — not deferred to a later
  cleanup pass. See roadmap.md's "How to read a session entry" for what the
  page should cover.
- This file is the project's institutional-knowledge file — the thing every
  session should read first.
- Skills (`.claude/skills/<name>/SKILL.md`) and subagents (`.claude/agents/`):
  none yet. Add a skill when one policy is being inconsistently enforced across
  sessions; add a subagent for a recurring job (e.g. verification). Don't
  pre-create either speculatively.

## 4. Test — verifying your work, CI evals

- While iterating: `cargo build && cargo test` from `calc-lang/` is enough — no
  need to run `fmt`/`check`/`clippy`/`doc`/`audit` on every loop.
- Before committing: run `cargo fmt` (fixes formatting, unlike CI's
  `--check`), then the same checks CI runs, in order:
  - `cargo check --workspace`
  - `cargo clippy --workspace --all-targets -- -D warnings` — `--all-targets`
    so test code gets linted too, not just lib/bin.
  - `RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps` — doc
    warnings matter here beyond hygiene: spec.md §12 commits to
    teaching-quality generated docs as a deliverable.
  - `cargo audit` (one-time setup: `cargo install cargo-audit --locked`) —
    checks `Cargo.lock` against the RustSec advisory database.
  - `cargo build --workspace`
  - `cargo test --workspace`
  So a commit never lands something CI would reject.
- [.github/workflows/agent-evals.yml](.github/workflows/agent-evals.yml) runs
  the same set on every push/PR to `main`. Keep new crates' `fmt`/`clippy`/
  `doc`-clean and tests runnable by plain `cargo test` so this stays the only
  CI step needed.

## 5. Deploy — not set up yet

No `REVIEW.md` PR-review policy, approval-gate hooks (`.claude/settings.json`),
or CI/CD deploy pipeline yet — there's nothing shipped for them to govern. Add
these once there's a real deploy target, not speculatively.

## 6. Maintain — not set up yet

No monitoring/metrics loop yet — there's no running system to monitor. When one
exists, the course's pattern is: deterministic threshold detection (no AI) that
escalates to Claude read-only, then to Claude acting through pre-approved routes,
with findings closing the loop as a fresh `intent.md` entry back into this same
pipeline.

## Workspace layout

`calc-lang/` is a Cargo workspace (resolver 2, edition 2021):

- `crates/calc-syntax` — parser frontend(s) behind the `ParserFrontend` trait
  (spec.md §5); has a `build.rs` for the LALRPOP grammar.
- `crates/calc-ir` — typed AST/IR types and lowering.
- `crates/calc-builtins` — the built-in manifest (spec.md §7), data only: each
  built-in's name, symbol, arity, binding kind, and that kind's declared link/run-time
  dependencies (the in-memory form of Part B's `bindings.toml`).
- `crates/calc-runtime` — the built-ins' real implementations: native-Rust `extern
  "C"` functions (kind 1), a tiny C library under `native/` that its `build.rs`
  compiles and publishes as the `calc_ffi` link unit (kind 2), and the IPC shim
  (kind 3, `serde_json`) plus IPC bundles under `ipc/` (e.g. `ipc/print/`, a
  multi-file Python project `calcc build` embeds in executables). `eval` is the
  interpreter's entry point.
- `crates/calc-runtime-artifacts` — builds `calc-runtime` as a static archive with a
  nested Cargo build and records rustc's list of its native dependencies; these are
  the link driver's inputs.
- `crates/calc-compiler` — the `calcc` compiler binary, codegen backends, the link
  driver (`src/link.rs`), and run-time dependency checks/bundling
  (`src/runtime_deps.rs`).
- `crates/calc-lsp` — the `calc-lsp` language server binary.
- `calc-lang/DECISIONS.md` — decision log for choices made while building
  `calc-lang` (Part A). Add an entry when a session makes a real design choice,
  especially one that picks between alternatives (e.g. parser library, memory
  strategy).
- `calc-lang/docs/` — the teaching-doc series (spec.md §12): one page per
  session, indexed by [calc-lang/docs/README.md](calc-lang/docs/README.md) in
  reading order.
