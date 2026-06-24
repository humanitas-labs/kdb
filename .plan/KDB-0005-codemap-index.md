---
title: "KDB-0005 — Codemap index: ls / check / render"
date: 2026-06-24
status: done
affects: "kdb codemap — colocated map discovery, freshness, derived index"
---

> Issue: [`.issues/iss-0065-codemap-index.md`](../.issues/iss-0065-codemap-index.md)

## Context

kdb indexes files and code symbols (`outline`, `fmt`, `refs`, `deps`) and now carries a relational layer (`projects`/`cycles`/`tasks`, iss-0063). It has no notion of *codemaps* — the authored, per-domain orientation docs described by `~/.claude/templates/codemap.md`.

The architecture (iss-0065): colocated `CODEMAP.md` files are the source of truth; a separate Claude Code workflow authors them; **kdb is the deterministic index/freshness layer over them**. This plan implements that kdb layer — three read-only subcommands, no LLM, no state beyond what it derives on the fly.

**Scope decision (2026.06.24):** ships `ls` / `check` / `render` only. A cross-domain dependency **graph is out of scope and dropped** — `kdb graph` is unimplemented (iss-0021) and a derived module graph isn't needed for a useful index. The index is the frontmatter table + staleness + coverage. No dependency on the deps engine. (If a graph ever earns its keep, it's a separate issue alongside iss-0021's renderer.)

**Frontmatter parsing:** `serde_yaml` is **added as a dependency** (it was not already present — the existing `tasks_import.rs` parser is hand-rolled). Frontmatter deserializes directly into `CodemapDoc` via serde derive.

Relevant existing infra to build on:

- **Walker** — `kdb fmt` already walks supported code files honoring ignore rules; reuse it for discovery and coverage.
- **Command module conventions** — single-file command modules exist (`src/tasks.rs`, `src/cycles.rs`, `src/projects.rs`); multi-concern features get a folder (`src/deps/`, `src/render/`, `src/fmt/`). Codemap spans discovery/git/render, so it warrants `src/codemap/`.
- **No git usage yet** — staleness needs git; shell out via `std::process::Command` with graceful fallback (kdb must keep working in a non-git tree).

## Design

### 1. Discovery + frontmatter (`ls`)

- Walk `path` (default workspace root) for files named `CODEMAP.md`, honoring the same ignore rules as `kdb fmt`.
- Parse YAML frontmatter into a `CodemapDoc` struct:

  ```rust
  struct CodemapDoc {
      file: PathBuf,        // path to the CODEMAP.md, root-relative
      domain: String,
      repo: Option<String>,
      root: PathBuf,        // subtree the map covers, root-relative
      owner: Option<String>,
      updated: Option<String>,    // YYYY.MM.DD
      commit: Option<String>,     // short SHA
  }
  ```

- `root` is normalized root-relative. If absent, default to the map file's own directory (the common colocated case).
- `kdb codemap ls [path] [--json]` prints a table (domain · root · updated · staleness) or the `CodemapDoc` array as JSON. JSON is the workflow's consumption format.
- **`confidence` is not modeled.** It's an authored self-assessment, not a derived signal; kdb's trust signal is staleness (§2). Frontmatter may carry it for human readers, but kdb neither types nor lints it.
- Frontmatter parsing uses `serde_yaml` (added as a dependency — not previously in the tree). Malformed/missing frontmatter ⇒ a `ParseProblem` surfaced by `check`, not a hard error in `ls`.

### 2. Coverage + staleness (`check`)

`kdb codemap check [path] [--stale] [--orphans] [--strict]` runs lints and prints findings grouped by kind. With no filter flags, run all; `--stale`/`--orphans` narrow. `--strict` ⇒ exit 1 if any finding (for CI/pre-commit).

Findings:

- **Dangling** — `root` (or the map's dir) no longer exists, or points outside the workspace.
- **Orphan / coverage gap** — a code subtree not covered by any map `root`, above a significance threshold. Heuristic: a directory with ≥ `--min-files` (default 5) supported code files, none of whose ancestors is a map `root`. Reported as a candidate domain (pairs with the workflow's partition step). Tunable; start conservative to avoid noise.
- **Stale** — files changed under the map's scope since it was written:
  - Primary signal: `git diff --name-only <commit>..HEAD -- <root>` (run from workspace root). Non-empty ⇒ stale, report the count and a few sample paths. Also surface commit distance via `git rev-list --count <commit>..HEAD`.
  - Fallback (no git, detached commit, or missing `commit:`): flag if `updated:` is older than a threshold, or simply "unverifiable — no commit pin."

Git access is encapsulated in `codemap::git` with one entry that returns `Option<GitFacts>`; everything degrades to the fallback when it returns `None`.

### 3. Derived index (`render`)

`kdb codemap render [path]` assembles the deterministic index to stdout (caller redirects to a file; placement is not kdb's call):

- A domains table — domain · root · owner · updated · **staleness** column (from §2) — each row linking to its `CODEMAP.md`.
- A coverage section — orphan candidates from §2.
- Stable, diff-friendly ordering (by root path).

This index stands alone as the repo's entrypoint — no LLM enrichment. The workflow's job ends at producing the maps; `render` turns them into the index. (No `--check` for render — it's pure stdout derivation.)

### 4. Module layout

```
src/codemap/
  mod.rs          // CodemapDoc, public entrypoints (ls/check/render)
  discover.rs     // walk + collect CODEMAP.md (reuse fmt walker)
  frontmatter.rs  // serde_yaml parse -> CodemapDoc, ParseProblem
  git.rs          // Option<GitFacts>: changed-files + commit-distance, graceful fallback
  check.rs        // dangling / orphan / stale lints
  render.rs       // index assembly (table + coverage)
```

`kdb codemap` dispatch added to clap in `src/main.rs`; thin entrypoints invoked from `src/cmd.rs` (matching the existing `cmd::*` pattern) or directly from a `codemap` arm — follow whichever the relational subcommands established.

## Changes

1. **`src/codemap/` (new)** — modules per §4. Core derivation logic; no I/O side effects beyond reading files and shelling to git.
2. **`src/main.rs`** — register the `codemap` subcommand group (`ls`/`check`/`render`) with clap.
3. **`src/cmd.rs`** — `codemap` entrypoints if matching the existing dispatch convention.
4. **`src/lib.rs`** — `pub mod codemap;`.
5. **`Cargo.toml`** — add `serde_yaml` dependency; version bump 0.35.1 → 0.36.0.
6. **`CHANGELOG.md`** — 0.36.0 entry.
7. **`README.md`** — add `kdb codemap` to the command list.
8. **`.issues/index.md`** — already registers iss-0065 (done).
9. **`tests/`** — fixtures + integration tests (see Verification).

## Files touched

```
┌─────────────────────────────────────────────┬──────────────────────────────┐
│                     File                     │            Action            │
├─────────────────────────────────────────────┼──────────────────────────────┤
│ src/codemap/mod.rs                           │ Create                       │
│ src/codemap/discover.rs                      │ Create                       │
│ src/codemap/frontmatter.rs                   │ Create                       │
│ src/codemap/git.rs                           │ Create                       │
│ src/codemap/check.rs                         │ Create                       │
│ src/codemap/render.rs                        │ Create                       │
│ src/main.rs                                  │ Edit (register subcommand)   │
│ src/cmd.rs                                   │ Edit (entrypoints, if used)  │
│ src/lib.rs                                   │ Edit (pub mod codemap)       │
│ Cargo.toml                                   │ Edit (serde_yaml, 0.36.0)    │
│ CHANGELOG.md                                 │ Edit (0.36.0 entry)          │
│ README.md                                    │ Edit (command list)          │
│ tests/codemap.rs (or tests/cli.rs)           │ Create / Edit (integration)  │
└─────────────────────────────────────────────┴──────────────────────────────┘
```

## Phased rollout

1. **Discovery + frontmatter + `ls`** — `CodemapDoc`, walker, serde_yaml parse, table/JSON output. Dogfood: write 1–2 `CODEMAP.md` in the kdb repo, run `kdb codemap ls`.
2. **`check` (no git)** — dangling, orphan/coverage. Tune the `--min-files` heuristic against kdb's own tree.
3. **Staleness via git** — `codemap::git`, wire into `check` and the `ls` staleness column; verify graceful fallback in a non-git fixture.
4. **`render`** — assemble the index (table + coverage); stable ordering.
5. **Docs + ship** — CHANGELOG, README, version bump; commit 0.36.0.

## Verification

Build:
- `cargo build --release` and `cargo clippy` in `projects/kdb/`.
- `cargo test` — unit tests on: frontmatter parse (valid/malformed/missing `root`), coverage (longest-prefix enclosure, orphan threshold), staleness (fixture git repo: clean vs. files-changed-since-commit, and the no-git fallback path).

Integration (fixture tree under `tests/`):
- A small tree with 2–3 `CODEMAP.md` files, one dangling `root`, and one uncovered subtree above threshold.
- Assert `ls --json` returns the expected docs; `check` flags exactly the dangling + orphan; `check --strict` exits non-zero; `render` emits the expected table rows in stable order.

Dogfood on the kdb repo itself:
- Author `CODEMAP.md` for a couple of real domains (e.g. `src/deps/`, `src/fmt/`).
- `kdb codemap ls` / `check` / `render` — sanity-check output; confirm staleness flips after an unrelated commit under a mapped root.
- `kdb check` still clean (no broken links introduced).
