---
id: 65
title: "codemap index — colocated maps + derived index"
status: done
priority: high
labels:
  - feat
---

# iss-0065 :: Codemap Index — Colocated Maps + Derived Index

> Plan: [`.plan/KDB-0005-codemap-index.md`](../.plan/KDB-0005-codemap-index.md)

## Problem

A codebase needs orientation maps an agent (or human) reads *first* to know what's here and where to go. Two failure modes today:

1. **No map** — orientation is re-derived from scratch every time.
2. **One central map** — a single root doc rots silently. It lives away from the code it describes, so a rename or refactor orphans it and nobody notices until they trust a stale claim. The template's `commit`/`confidence` fields exist precisely because staleness is *the* failure mode.

An earlier proposal was a single root `CODEMAP.md` plus an auto-generated per-file dump. The per-file dump is now largely covered by `kdb outline`/`kdb fmt`; the valuable half was the *authored overview* — but as a single central doc it inherits failure mode 2.

We now have a per-domain map template (`~/.claude/templates/codemap.md`). What's missing is the system that decides **where maps live, how they stay trusted, and how you get a system-level view across them** — without reintroducing silent drift.

## Solution

Three cooperating layers. **This issue is layer 2 only.**

1. **Colocated domain maps — source of truth.** A `CODEMAP.md` lives next to each domain/package subtree (`src/auth/CODEMAP.md`), authored against the template. Proximity is the staleness defense: the map sits in the diff when you touch the code, so it rots loudly. Maps move with the code and are individually addressable files.

2. **`kdb codemap` — deterministic index/freshness layer (this issue).** Walks the tree, reads each map's frontmatter, checks coverage + staleness, and assembles a derived index. Pure derivation, no prose, no LLM — runs in milliseconds, can't drift.

3. **Codemap workflow — authoring (separate, a Claude Code Workflow).** Fans out over domains to author the colocated maps. Its sole job is producing the `CODEMAP.md` files; the index is then whatever kdb deterministically assembles from them.

**Governing principle:** anything derivable from the filesystem + frontmatter is kdb's job (deterministic, can't drift, ms); anything requiring reading comprehension is the workflow's job. The two never overlap — the workflow *produces* the maps, kdb *reads* them.

### The contract

The interface between layers is the `CODEMAP.md` frontmatter (already defined by the template):

```yaml
domain: …      repo: …      root: …      owner: …
updated: YYYY.MM.DD      commit: <short SHA>      confidence: high|medium|low
```

kdb treats this frontmatter as a first-class indexed object, the way it already indexes headings and symbols. (`confidence` is carried for human readers but not modeled or linted — staleness is kdb's trust signal.)

### Command surface (sketch — details in the plan)

```text
kdb codemap ls      [path] [--json]                 # discover maps, list domain/root/updated/staleness
kdb codemap check   [path] [--stale] [--orphans] [--strict]
                                                    # coverage gaps, stale maps (commit drift), dangling roots
kdb codemap render  [path]                          # assemble the derived index (domains table + staleness + coverage)
```

- **Staleness** = `git diff --name-only <commit>..HEAD -- <root>` non-empty ⇒ N files changed under the map's scope since it was written. Honest and precise; falls back to `updated:` age if git/commit unavailable.
- **`check --strict`** exits non-zero for CI/pre-commit — the cheap, frequent freshness gate (vs. the heavy, occasional workflow regen).
- **`render`** is deterministic and stands on its own as the repo's entrypoint index — a table of domains linking into each map, with a staleness column and coverage gaps. No LLM enrichment.

## Out of scope

- Authoring maps — that's the workflow.
- **Cross-domain dependency graph.** Considered and dropped: `kdb graph` is unimplemented (iss-0021), and a derived module graph isn't needed for the index to be useful. The frontmatter table + staleness + coverage is the index. Revisit as a separate issue if it ever earns its keep.
- `file:line` anchor validation inside map *bodies* (validating every citation resolves) — valuable but heavier; see open questions.
- Owning where the rendered index ultimately lives — `render` writes to stdout; the caller decides placement.

## Open questions

- Coverage heuristic: what makes a subtree "significant enough" to warrant its own map (min code-file count? configurable)? Pairs with how the workflow partitions.
- Anchor validation later — should `check` eventually parse map bodies and confirm `file:line` anchors resolve (cross-ref `kdb outline`)?
- Nested maps (a map under another map's root) — longest-prefix enclosure handles coverage; confirm `check` reports them as intended sub-domains, not orphans.
