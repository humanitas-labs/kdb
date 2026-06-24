---
id: 66
title: "kdb check scoping — index build ignores subtree scope (slow in monorepos)"
status: proposed
priority: medium
labels:
  - perf
---

# iss-0066 :: `kdb check` Scoping — Index Build Ignores Subtree Scope

## Problem

`kdb check [path]` scopes its *output* to the given subtree but still builds the
**whole-workspace vault index** to do it. In a meta-workspace (one `.kdb` root
spanning many repos — e.g. the digimata monorepo), this parses every markdown
file in the workspace even when you only asked about one repo.

Measured on the digimata workspace (2026.06.24):

- `kdb check /…/projects/kdb` → **31.5 s**, parsing **5,780 `.md` files** across
  marina / kairos / acme / etc.
- `kdb codemap check` (repo-scoped by design) → **0.35 s** on the same repo.

The index build is whole-workspace so that cross-subtree links (`../../marina/x.md`)
can be validated. But in a multi-repo workspace that's rarely what you want, and
it makes the common "check this repo" case pay for the entire monorepo.

## Direction

Mirror what `kdb codemap` does: default the index-build scope to the **nearest
enclosing git repository** of the target (the kdb workspace may span many repos),
falling back to the workspace root when no repo is found. Cross-repo links would
then resolve only within the repo by default; an opt-in flag (e.g.
`--workspace`) could restore whole-workspace validation when genuinely needed.

Open questions:

- Does any current workflow rely on cross-repo link validation? If so, the
  default flip needs a flag and a note.
- Should the same enclosing-repo default apply to other workspace-wide commands
  (`kdb index`, `kdb fmt` with no path)? Probably — same meta-workspace tax.

## Context

Surfaced while shipping iss-0065 (`kdb codemap`), which already defaults to the
enclosing git repo for exactly this reason. This issue brings the older
whole-workspace commands in line.
