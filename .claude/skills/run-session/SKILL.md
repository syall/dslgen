---
name: run-session
description: Runs one roadmap.md session (e.g. "A3", "B2", "A1-pest") end-to-end for the DSL-Generator project, following the repo's own documented SDLC in CLAUDE.md — look up the session, plan it, implement it, verify it, and prepare plan.md/DECISIONS.md/docs updates for commit. Use this whenever the user says "do session X", "run A4", "let's build the next session", "continue the roadmap", or names a roadmap.md session ID and wants it implemented, not just explained.
---

# Run a roadmap.md session

Executes one session from [roadmap.md](../../../roadmap.md) the same way sessions
A0–A2 were already built in this repo: plan → implement → verify → document →
(confirmed) commit. This is not new policy — it's CLAUDE.md §3/§4's existing "Build"
and "Test" process, packaged so it's applied the same way every time instead of
reinvented per session.

Argument: a session ID from roadmap.md (`A0`, `A1`, `A1-pest`, `A3`, `B2`, `C4`, ...).
If none was given, don't guess — figure out the next unstarted session (see step 0)
and ask the user to confirm before doing anything else.

## 0. Resolve the session

If no session ID was given: read roadmap.md top to bottom, and cross-check against
`calc-lang/docs/` (one page per completed session) and `git log` (one "Add <ID>: ..."
commit per completed session) to find the first session in roadmap order that has
neither. Propose that one as the default and ask the user to confirm — don't just
start on it, since sessions like A1-pest/A1-custom/A17/B10 are explicitly optional
side branches the user may want to skip.

## 1. Read the session's entry and its prerequisites

Read the named session's full entry in roadmap.md: spec refs, prereqs, the Rust and
compiler/tooling learning goals, and the deliverable. The deliverable is the actual
spec for "done" — plan and implement against it directly.

Check every session listed under **Prereqs** has actually landed, the same way step 0
checks completion: a `calc-lang/docs/<id>-*.md` page and/or a `DECISIONS.md` entry
and/or an "Add <ID>: ..." commit in `git log`. If a prereq is missing, stop and tell
the user which one — don't build ahead of a dependency that isn't there, since later
sessions' plans routinely assume earlier ones' types/traits/files exist.

Then read the spec.md section(s) the entry cites. The roadmap tells you *what* to
build; spec.md tells you *why* — cite the relevant section(s) in the plan and in code
comments/docs where it clarifies a non-obvious choice, per CLAUDE.md's "Design" stage.

## 2. Plan

Enter Claude Code's plan mode before writing any non-trivial code (CLAUDE.md §3).
Scope the plan to exactly this session's deliverable — no more. This project's
cross-cutting principle (spec.md §2, intent.md, and CLAUDE.md's header note) is small,
focused, incremental change: don't add scaffolding for a future session's needs just
because the roadmap mentions them. The real A2 session is the model for this restraint
— it explicitly did *not* add a `Stmt` AST node the roadmap's own template text
mentioned, because nothing yet needed one, and logged that deferral rather than
silently doing extra work or silently skipping the roadmap's wording.

If the session offers real alternatives (e.g. "optional" sessions comparing frontends,
or an open question from spec.md §14), the plan should name the choice and pick one
with a stated reason, mirroring calc-lang/DECISIONS.md's existing entries — that
reasoning is what step 5 turns into a DECISIONS.md entry.

Get the user's approval on the plan before implementing.

## 3. Implement

Build exactly the approved plan. If reality forces a departure (an API doesn't exist,
a test reveals the plan was wrong), make the smallest reasonable change and note the
departure — it goes in plan.md's Outcome section in step 4, not silently absorbed.

## 4. Write plan.md

`plan.md` lives at the fixed repo-root path and is **overwritten**, not accumulated —
CLAUDE.md is explicit that old sessions' plans are recovered via `git show
<commit>:plan.md` or `git log -- plan.md`, not kept as separate per-session files. Use
the two-section shape the existing plan.md (session A2) already establishes:

```markdown
# Session <ID> — <title>

Spec refs: spec.md §... . Roadmap: roadmap.md "<ID>".

## Plan

<the approved plan from step 2, as numbered steps>

## Outcome

<implemented as planned, or note departures; what `cargo build && cargo test` showed>
```

## 5. Update DECISIONS.md, if this session made a real choice

Only add an entry if the session actually picked between alternatives or deliberately
deferred something the roadmap text implies (like A2's `Stmt` deferral). Routine
implementation of an already-decided design doesn't need an entry. Model tone and
structure on the existing A0/A2 entries in [calc-lang/DECISIONS.md](../../../calc-lang/DECISIONS.md):
state the choice, the reasoning, and what was deliberately deferred and why.

## 6. Write the session's teaching-doc page

Every session's deliverable includes a page in `calc-lang/docs/` (spec.md §12) — this
is not a follow-up task, it ships in the same commit as the code. Name it
`calc-lang/docs/<id-lowercase>-<slug>.md` matching the existing `a0-...`/`a1-...`/
`a2-...` files, use this session's own code as the running example (not toy snippets),
and cover what the roadmap's "Compiler/tooling you'll learn" bullet describes, aimed
at a reader with no prior compiler background. Add a numbered entry for it to
[calc-lang/docs/README.md](../../../calc-lang/docs/README.md)'s reading-order list,
one line describing what the page covers, following the existing three entries.

## 7. Verify

Run `cargo build && cargo test` from `calc-lang/`. Both must pass before this session
counts as done — this is CLAUDE.md §4's only required check, and it's also what CI
(agent-evals.yml) runs on every push, so a failure here is a failure there.

## 8. Commit — only with the user's go-ahead

This skill does not commit on its own authority. Once the user has reviewed the diff
(or has otherwise pre-authorized autonomous commits for this session), stage and
commit together in one commit: the code changes, `plan.md`, any `DECISIONS.md`
addition, and the new/updated `calc-lang/docs/` files. Follow the existing commit
message style from `git log` — short imperative summary naming the session, e.g. "Add
A2: typed AST and semantic actions".
