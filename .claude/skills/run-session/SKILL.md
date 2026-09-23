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

**Where the teaching docs live.** Steps 0, 1, 6, and 9 all need the current location
of the per-session teaching-doc series (spec.md §12). As of Part A that's
`calc-lang/docs/`, named in CLAUDE.md's "Workspace layout" section — but CLAUDE.md is
explicit that it gets updated whenever a convention like this changes, and Part B's
whole job is generalizing Part A's per-DSL layout (roadmap.md B1–B7), so the location
may well move once generation is in play (e.g. to a per-generated-workspace `docs/`
rather than one fixed path in this repo). Before relying on a docs path, re-read
CLAUDE.md's current "Workspace layout" section rather than assuming `calc-lang/docs/`
— treat every `calc-lang/docs/` mention below as "wherever that section currently
points," not a hardcoded path.

## 0. Resolve the session

If no session ID was given: read roadmap.md top to bottom, and cross-check against
the teaching-doc series (see "Where the teaching docs live" above — one page per
completed session) and `git log` (one "Add <ID>: ..." commit per completed session) to
find the first session in roadmap order that has neither. Propose that one as the
default and ask the user to confirm — don't just
start on it, since sessions like A1-pest/A1-custom/A17/C1-wasm are explicitly optional
side branches the user may want to skip.

## 1. Read the session's entry and its prerequisites

Read the named session's full entry in roadmap.md: spec refs, prereqs, the Rust and
compiler/tooling learning goals, and the deliverable. The deliverable is the actual
spec for "done" — plan and implement against it directly.

Check every session listed under **Prereqs** has actually landed, the same way step 0
checks completion: a `<id>-*.md` page in the teaching-doc series and/or a
`DECISIONS.md` entry and/or an "Add <ID>: ..." commit in `git log`. If a prereq is
missing, stop and tell the user which one — don't build ahead of a dependency that
isn't there, since later sessions' plans routinely assume earlier ones' types/traits/
files exist.

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

Every session's deliverable includes a page in the teaching-doc series (spec.md
§12, and see "Where the teaching docs live" above for its current location) — this
is not a follow-up task, it ships in the same commit as the code. Name it
`<id-lowercase>-<slug>.md` matching the existing `a0-...`/`a1-...`/`a2-...` files'
convention, use this session's own code as the running example (not toy snippets),
and cover what the roadmap's "Compiler/tooling you'll learn" bullet describes, aimed
at a reader with no prior compiler background. Add a numbered entry for it to that
directory's own `README.md` reading-order list, one line describing what the page
covers, following the existing entries' style.

## 7. Verify

While implementing, `cargo build && cargo test` from `calc-lang/` is enough to check
progress. Before committing (step 8), run the full sequence CLAUDE.md §4 and CI
(agent-evals.yml) both require, in order, from `calc-lang/`:

```bash
cargo fmt
cargo check --workspace
cargo clippy --workspace --all-targets -- -D warnings
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps
cargo audit
cargo build --workspace
cargo test --workspace
```

All of it must pass — a commit should never land something CI would reject. (`cargo
audit` needs a one-time `cargo install cargo-audit --locked` if it isn't on PATH yet.)

## 8. Commit and push — only with the user's explicit go-ahead

This skill does not commit or push on its own authority. Commit and push are two
separate asks, each needing its own explicit confirmation — don't infer a push from
"commit it" alone, and don't infer either from "looks good."

Once the user has confirmed the commit (or has otherwise pre-authorized autonomous
commits for this session), stage and commit together in one commit: the code changes,
`plan.md`, any `DECISIONS.md` addition, and the new/updated teaching-doc files.
Follow the existing commit message style from `git log` — short imperative summary
naming the session, e.g. "Add A2: typed AST and semantic actions".

Only push if the user separately and explicitly asks for that too. If they do, push,
then move to step 9, then step 10.

## 9. Sync — or create — the "DSL-Generator Roadmap" Artifact — only after an explicit push

This step runs only when both step 8 confirmations happened and the push actually
went through — a commit that's still local isn't done in any sense a shared roadmap
view should reflect. If the session wasn't pushed, skip this step entirely.

The **DSL-Generator Roadmap** Artifact is a standalone HTML dashboard mirroring
roadmap.md's session list with progress bars and per-session status pills; it doesn't
read the repo live, so it only ever reflects reality if this step keeps it in sync.

1. Find it with `Artifact` `list` (scope "mine") if you don't already have its URL
   from a prior turn — titled "DSL-Generator Roadmap", favicon 🛠️.

**If it already exists:**

2. `Artifact` `read` it to get the current HTML — always edit the live version, never
   guess at its structure from memory.
3. Update it to reflect the session that was just pushed:
   - Turn its row from a plain `<div class="row ...">` into
     `<a class="row is-done" href="..." target="_blank" rel="noopener">`, linking to
     the session's teaching-doc page on GitHub, and add a `<span
     class="doc-icon">↗ docs</span>` at the end of its `row-desc`, matching how A0–A5's
     rows already link out — set its pill to `pill done">Done`. Build the link from
     the repo's actual remote (`git remote get-url origin`) and default branch rather
     than hardcoding — currently
     `https://github.com/syall/dslgen/blob/main/<doc-path>` — so this keeps working if
     the repo is ever renamed or forked.
   - Advance the `next` pill to whichever session immediately follows it in
     roadmap.md's order, skipping the optional side branches (`A1-pest`, `A1-custom`,
     `A17`, `C1-wasm`) the same way the footer's counts already do; the session that had `next`
     before becomes `done` (and gets linked per the bullet above), and whatever is now
     next changes from `todo` to `next` — it stays a plain, unlinked row until *it*
     has a doc page.
   - Recompute the top `stat-band` count and percentage, and the relevant Part's
     `part-count` and bar width — the denominator stays the 35-session required path,
     matching the footer's existing framing of the optional branches as uncounted.
   - If this session's Part doesn't yet have its `part-card` wrapped in an
     `<a class="part-link" href="...">` pointing at that Part's own docs index, check
     whether one now exists (e.g. Part A's card links to
     `calc-lang/docs/README.md` because that index exists; Parts B/C stay plain
     `<div class="part-card">` until each has its own index file to point to — don't
     link a Part card to an index that doesn't exist yet).
   - Update the `stat-next` line's session ID and description to match the new
     "next" row.
   - Update the footer's "Status is derived from git history (commits through
     `<sha>`, ...)" to the new commit's short SHA and message, and adjust the
     suggested-path sentence if the newly done session changes what "remains."
4. `Artifact` `publish` with the same `url` so it updates in place rather than
   creating a duplicate dashboard.

**If `list` turns up nothing titled "DSL-Generator Roadmap":**

2. Build it fresh from roadmap.md and the repo's actual state (`git log`, and which
   sessions have a page in the teaching-doc series — see "Where the teaching docs
   live" above) — don't invent completion status, derive it the same way step 0 does.
   Follow the `Artifact` tool's own process for a new page (quickstart, then the
   `artifact-design` skill) rather than skipping straight to HTML.
3. Shape the dashboard the same way the existing one already proved out, so a
   recreated dashboard looks and behaves like the one it's replacing rather than like
   a fresh design: an eyebrow/title header naming roadmap.md and the AI-native SDLC
   Build stage; a top stat band with the required-path count/percentage, a progress
   bar, and a "Next up" pointer; three Part cards (A/B/C) each with their own
   session-count and bar; a collapsible per-part session list where each row shows the
   session ID, a one-line description, and a status pill (`done` / `next` / `todo` /
   `opt` for the optional side branches); and a footer stating the git commit the
   status was derived from and which sessions remain on the suggested minimum path
   (roadmap.md's own "Suggested minimum path" section). Use theme-aware CSS custom
   properties (light + `prefers-color-scheme: dark`) rather than hardcoded colors, per
   `artifact-design`.
   Also carry over the linking convention the existing dashboard settled on (see the
   "already exists" branch above): every `done` row links out to its teaching-doc page
   on GitHub with a "↗ docs" marker, and a Part card only becomes a link once that
   Part actually has a docs index file to point to — as of now that's Part A only, via
   `calc-lang/docs/README.md`. Don't link a `next`/`todo`/`opt` row or a Part without
   an index; a link to a page that doesn't exist is worse than no link.
4. `Artifact` `publish` it with title "DSL-Generator Roadmap" and a fitting favicon (the
   existing one used 🛠️). Tell the user the new URL — it's needed for every future
   sync, and there's no other record of it once published.

## 10. Refresh the GitHub Pages snapshot — every time a session is pushed

The `docs` branch's root `index.html` is a public, standalone mirror of the
Artifact — it's what's actually live at `https://syall.github.io/dslgen/` once
GitHub Pages is configured with Source: `docs` / `(root)` (a one-time repo-settings
step; skip re-suggesting it once it's already been done).

Unlike a one-off publish, the user has made this step's `docs`-branch push a
standing part of this workflow: run it automatically right after step 9, every time
step 8's `main` push happens, with no separate per-session confirmation. This
pre-authorization is scoped narrowly to this one action (refreshing this snapshot on
the `docs` branch after a session lands) — it isn't a general license to push
anywhere else without asking, and `main`'s push in step 8 still needs its own
explicit go-ahead each time.

Steps:

1. Make sure step 9 already ran (or `Artifact` `read` the live dashboard) so this
   snapshot reflects real, current state rather than stale data — the GitHub Pages
   copy should always be a snapshot *of* the Artifact, not a second source of truth
   maintained independently.
2. Work in an isolated git worktree for the `docs` branch (`git worktree add
   <tmp-path> docs`, or `git worktree add <tmp-path> --orphan docs` the first time
   the branch doesn't exist yet) so this never disturbs whatever's in progress on
   `main` — the branch is an unrelated root history, not a merge target.
3. Turn the Artifact's HTML into a standalone page at that worktree's `index.html`
   (root, not a nested `docs/` folder — the Pages source is already `docs` branch
   `/` root): give it a proper `<!DOCTYPE html><head>` with `<title>` and a
   `<meta name="description">`, the same Google Fonts `<link>`s and CSS custom
   properties (incl. `prefers-color-scheme: dark`) the artifact uses, and drop
   anything specific to the Claude Artifact platform's own chrome.
4. Commit in that worktree with a message naming the commit it's synced to (the
   existing `index.html` was committed as "Add roadmap status page for GitHub
   Pages" citing `main@<sha>` — follow that pattern for the SHA it now reflects),
   then push the `docs` branch, per the standing pre-authorization above.
5. Remove the temporary worktree once pushed.

If the user ever wants this wired into CI instead of run through this skill, that's
new scope (a GitHub Actions workflow) — don't build it speculatively per CLAUDE.md's
"don't restructure or generalize ahead of need"; only add it if asked.
