---
name: ship
description: >-
  End-to-end flow for pushing a change to vivo-cli: branch off main, run
  Conventional Commits, predict the release-please semver bump, push, and
  open a PR. Use when the user says "ship this", "push this change", "open
  a PR for this", or asks to create a branch + commit + PR for pending
  vivo-cli work. Always branches from origin/main first since main only
  accepts changes via PR merge (direct pushes break release-please).
user-invocable: true
---

# Ship — branch, commit, predict version bump, PR

This repo (`vivo-cli`) is released via
[Release Please](https://github.com/googleapis/release-please): the version
bump on the *next* release is derived entirely from the Conventional Commit
types that land on `main`. Getting the commit type right isn't just style —
it decides whether the next release is a patch, minor, or (post-1.0) major.

Run this flow whenever the user wants pending changes pushed as a PR.

## Phase 0 — Preflight

```bash
cd vivo-cli   # this repo's git root; do not run git commands from the monorepo root
git status
cargo build && cargo test && cargo clippy
```

Fix any build/test/clippy failures before proposing commits — don't ship
red code.

## Phase 1 — Branch from main

`main` requires PR merges, not direct pushes, for release-please to work.
Never commit directly on `main`, and never stack this work on an unrelated
branch.

```bash
git fetch origin main --quiet
git branch --show-current
```

If the current branch is `main` or is an unrelated/stale branch (already
merged, or carrying commits not part of this change), create a fresh branch
from `origin/main`:

```bash
git checkout -b <type>/<short-kebab-description> origin/main
```

Uncommitted working-tree edits carry over automatically. Pick `<type>` to
match the dominant Conventional Commit type of the change (see Phase 2).

## Phase 2 — Analyse and plan commits (Conventional Commits)

Follow the same discipline as the `conventional-commits:commit` skill:

```bash
git diff HEAD
git diff --cached
```

Group changed files into logical units and draft one Conventional Commit
per unit:

```
<type>[optional scope]: <description>

[optional body]

[optional footer(s)]
```

- Types: `feat`, `fix`, `docs`, `style`, `refactor`, `perf`, `test`,
  `build`, `ci`, `chore`, `revert`
- Imperative present tense, no capital first letter, no trailing period,
  first line under 72 chars
- Breaking change: `!` after type/scope, or a `BREAKING CHANGE:` footer
- Never add `Co-Authored-By` or AI attribution footers
- Only commit when the user has approved the plan (or said "go"/"ship it")

## Phase 3 — Predict the semver bump

Before presenting the plan, compute what release-please will actually do
with these commit types, using this repo's own config:

```bash
cat release-please-config.json      # release-type, bump-minor-pre-major, etc.
cat .release-please-manifest.json   # current version
```

Apply release-please's real bump rules — **don't just assume
feat=minor/fix=patch**, the pre-1.0 flags change that:

1. Take the current version `X.Y.Z` from the manifest.
2. Look across every commit in the plan (not just this PR's most dramatic
   one — release-please aggregates all commits since the last release) and
   find the **highest-impact** type:
   - Any `!` or `BREAKING CHANGE:` footer → breaking
   - Else any `feat` → feature
   - Else any `fix` or `perf` → patch-worthy
   - Else (`docs`/`style`/`refactor`/`test`/`build`/`ci`/`chore` only) →
     no version bump triggered
3. Resolve the actual bump using the package config:
   - If `X == 0` (pre-1.0) **and breaking**: bump-major-pre-major is not
     set in this repo's config → bumps **minor** (`X.(Y+1).0`), not major.
   - If `X == 0` **and feature** with `bump-minor-pre-major: true` (this
     repo's setting) → bumps **minor** (`X.(Y+1).0`).
   - If `X == 0` and feature with `bump-minor-pre-major` false/unset →
     would bump **patch** instead (not this repo's current config, but
     re-check `release-please-config.json` in case it changes).
   - If `X >= 1`: breaking → major; feature → minor; patch-worthy → patch.
   - Patch-worthy only (no feat, no breaking) → bumps **patch**
     (`X.Y.(Z+1)`) regardless of major version.
   - No qualifying type → merging this PR **will not** cut a new release
     by itself.

State the predicted next version explicitly in the plan, e.g.:
`Predicted next release: 0.12.0 → 0.13.0 (feat commit present, bump-minor-pre-major is on)`.
If nothing in the plan qualifies, say so plainly so the user isn't
surprised when no release fires after merge.

## Phase 4 — Confirm

Present the commit plan (message + files + why) **and** the predicted
version bump together. Ask:

> "Does this look right? You can ask me to merge, split, reorder, or
> rename any commit — or say **go** to commit and open the PR."

Wait for explicit approval before staging or committing anything.

## Phase 5 — Execute commits

For each approved commit, in order:

```bash
git add <exact files for this commit>
git diff --cached --stat
git commit -m "$(cat <<'EOF'
<type>[scope]: <description>

[body]
EOF
)"
```

Never use `git add -A` / `git add .` — stage only the files named in the
plan. Never skip hooks (`--no-verify`) or amend existing commits.

## Phase 6 — Push and open the PR

```bash
git push -u origin <branch-name>
```

```bash
gh pr create --title "<same as primary commit's subject, or a summary if multiple>" --body "$(cat <<'EOF'
## Summary
<1-3 bullets on what changed and why>

## Release impact
Predicted next version: <X.Y.Z> → <X'.Y'.Z'> (<reason>)

## Test plan
<checklist of what was verified — build/test/clippy, manual runs, etc.>
EOF
)"
```

Report the PR URL back to the user. Do not merge it — merging main is a
separate, explicit action the user must request.
