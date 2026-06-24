---
domain: "codemap"
repo: "kdb"
root: "src/codemap"
owner: "andrew"
updated: 2026.06.24
commit: "8c33d68"
confidence: high
---

# codemap — Code Map

> A map, not a manual. It points to where things live and how a request flows;
> it does not duplicate per-function detail (that's what the code and `kdb outline` are for).

## 1. What this is

`kdb codemap` is the deterministic index/freshness layer over colocated `CODEMAP.md`
domain maps (like this one). The maps are the source of truth, authored separately
(by an LLM workflow); this module is the **read-only** side: discover the maps, parse
their frontmatter, lint coverage + staleness against git, and render a derived index.
In scope: discovery, frontmatter parsing, coverage/orphan analysis, git staleness, the
`ls`/`check`/`render` commands. Out of scope: authoring map prose, persisting any state,
any LLM call — it is pure derivation (`mod.rs:6`).

## 2. Shape — diagram

```
 CLI (src/main.rs)                       codemap module (src/codemap/)
   │ kdb codemap {ls,check,render}
   ▼
┌──────────────────┐   resolve_scope     ┌──────────────────────┐
│ dispatch         │────────────────────▶│ mod.rs               │
│ main.rs:917      │   (enclosing repo)   │  resolve_scope :59   │
└──────────────────┘                      └──────────┬───────────┘
                                                     │ scope (abs repo root)
                                                     ▼
                                          ┌──────────────────────┐    ┌──────────────┐
                                          │ discover.rs          │───▶│ frontmatter  │
                                          │  discover :49        │    │  parse :38   │
                                          │ (ignore-aware walk)  │    └──────────────┘
                                          └──────────┬───────────┘    → CodemapDoc
                                       Discovery {docs, problems}
                        ┌──────────────────┬─────────┴──────────┐
                        ▼                  ▼                     ▼
                  ┌───────────┐      ┌───────────┐        ┌───────────┐
                  │ ls        │      │ check     │        │ render    │
                  │ mod.rs:90 │      │ check.rs  │        │ render.rs │
                  │ → table   │      │  :65      │        │  :17      │
                  └───────────┘      └─────┬─────┘        └─────┬─────┘
                                           │ orphan_candidates  │ assemble
                                           ▼                    ▼ index md
                                     ┌───────────┐        (shared: git.rs
                                     │ git.rs    │◀───────  staleness :39)
                                     │ staleness │
                                     └───────────┘
```

## 3. Entry points

| Trigger | Handler (`file:line`) | Purpose |
|---|---|---|
| `kdb codemap ls [path] [--json]` | `mod.rs:90` (`ls`) | discover maps, print an aligned table or JSON |
| `kdb codemap check [path] [--stale] [--orphans] [--strict] [--min-files N] [--json]` | `check.rs:65` (`check`) | lint dangling/orphan/stale/parse findings; `--strict` exits non-zero |
| `kdb codemap render [path]` | `render.rs:17` (`render`) | print the deterministic index markdown to stdout |
| clap subcommand enum + dispatch | `main.rs:225` (`CodemapCmd`), `main.rs:917` | maps CLI flags to the three public fns |

## 4. Lifecycle — the trace

The `check` path (the richest; `ls`/`render` are subsets):

1. **Dispatch** — `main.rs:926` — clap parses `CodemapCmd::Check` and calls `check::check(path, Lints{stale,orphans}, strict, min_files, json)`.
2. **Resolve scope** — `check.rs:72` → `mod.rs:59` (`resolve_scope`) — builds a `CmdContext`; with no explicit path, `enclosing_repo` (`mod.rs:75`) walks up for the nearest `.git`, bounded at the workspace root.
3. **Discover** — `check.rs:74` → `discover.rs:49` (`discover`) — ignore-aware walk collects every `CODEMAP.md`, reads each, calls `frontmatter::parse`; results split into `docs` and `problems`, sorted by root.
4. **Parse problems** — `check.rs:78` — every unparseable map becomes a `Finding::Problem` (always reported).
5. **Dangling roots** — `check.rs:86` → `dangling_reason` (`check.rs:148`) — a `root` that is outside the repo or no longer exists becomes `Finding::Dangling`.
6. **Orphans** — `check.rs:96` → `discover_code_files` (`discover.rs:26`) + `orphan_candidates` (`check.rs:166`) — counts uncovered code files per ancestor dir, keeps maximal subtrees at/above `min_files`.
7. **Staleness** — `check.rs:103` → `git::staleness` (`git.rs:39`) — per non-dangling map, `git diff --name-only <commit>..HEAD -- .` from the subtree; `Stale`/`Fresh`/`Unverifiable`.
8. **Render + exit** — `check.rs:130` → `render_findings` (`check.rs:211`); if `strict` and any non-`Unverifiable` finding exists, `std::process::exit(1)` (`check.rs:142`).

## 5. File index

**`src/codemap/`**
| File | Role |
|---|---|
| `mod.rs` | module root: `CodemapDoc`/`ParseProblem` types, `resolve_scope`, `enclosing_repo`, `ls` command + table |
| `discover.rs` | ignore-aware discovery of `CODEMAP.md` files and supported code files; `Discovery` result |
| `frontmatter.rs` | parse a map's YAML frontmatter into `CodemapDoc`; `root` defaults to the map's dir; `confidence` ignored |
| `check.rs` | `check` command, `Finding` enum, `Lints`, `orphan_candidates` coverage analysis, finding renderer |
| `git.rs` | `Staleness` enum + git-derived staleness (`git diff` from the subtree), graceful `Unverifiable` fallback |
| `render.rs` | `render` command: assemble the index markdown (Domains table + Coverage + Problems) |

## 6. Key types & contracts

| Type | `file:line` | What it carries |
|---|---|---|
| `CodemapDoc` | `mod.rs:25` | a parsed map: repo-relative `file`, `domain`, `repo`, `root`, `owner`, `updated`, `commit` (all paths repo-relative) |
| `ParseProblem` | `mod.rs:45` | an unparseable map: `file` + human `message` (surfaced by `check`, never fatal in `ls`) |
| `Discovery` | `discover.rs:40` | scan result: `docs: Vec<CodemapDoc>` + `problems: Vec<ParseProblem>` |
| `Finding` (enum) | `check.rs:45` | lint result variants: `Dangling`/`Orphan`/`Stale`/`Problem`/`Unverifiable` (serde-tagged `kind`) |
| `Lints` | `check.rs:25` | which lint families to run; empty (no flags) ⇒ all (`is_all` at `check.rs:37`) |
| `Staleness` (enum) | `git.rs:21` | `Fresh` / `Stale{changed, commit_distance, sample}` / `Unverifiable{reason}` |
| `RawFrontmatter` | `frontmatter.rs:25` | all-optional serde shape so missing required fields become `ParseProblem`s, not serde errors |

## 7. Extension points

| To do this… | Touch | Notes |
|---|---|---|
| Add a new lint | `check.rs` | add a `Finding` variant (`check.rs:45`), emit it in `check` (`check.rs:65`), render it in `render_findings` (`check.rs:211`); add a `Lints` flag + gate if opt-in |
| Add a frontmatter field | `frontmatter.rs`, `mod.rs` | add to `RawFrontmatter` (`frontmatter.rs:25`) + `CodemapDoc` (`mod.rs:25`), map it in `parse` (`frontmatter.rs:62`) |
| Change staleness logic | `git.rs:39` | the only place git is consulted; keep the `Unverifiable` fallback for non-git trees |
| Add a `codemap` subcommand | `main.rs:225` (`CodemapCmd`), `main.rs:917` dispatch, new fn in this module | follow `ls`/`check`/`render` shape (each takes `path`, calls `resolve_scope` then `discover`) |
| Tune coverage sensitivity | `check.rs:21` (`DEFAULT_MIN_FILES`) | also surfaced as `--min-files`; used by both `check` and `render` |
| Change the rendered index shape | `render.rs:31` (`assemble`) | pure string builder; output is stdout-only (caller decides placement) |

## 8. Invariants & gotchas

- **Maps are source of truth; this module never writes them.** No LLM, no persisted state — pure derivation (`mod.rs:6`). `render` prints to stdout; placement of the index file is the caller's call (`render.rs:4`).
- **Paths are repo-relative, not workspace-relative.** Discovery uses the resolved scope (repo root) as the base so a map records `root: src/deps` and stays portable with the codebase (`discover.rs:5`). Test fixtures and `CodemapDoc` assume this.
- **Default scope is the enclosing git repo, not the workspace.** With no explicit path, `enclosing_repo` walks up for `.git`, bounded at the workspace root, because the kdb workspace may span many repos (`mod.rs:54`, `mod.rs:75`).
- **Git runs from the subtree dir, not the workspace root.** `git -C <subtree>` lets git find the repo that owns the code even when the meta-workspace root isn't a git repo (`git.rs:5`). Everything degrades to `Unverifiable` if git or the commit pin is missing.
- **`Unverifiable` is advisory.** `--strict` only fails on actionable findings; `Unverifiable` (no commit pin / no git) is excluded from the exit-code count (`check.rs:136`).
- **`confidence` frontmatter is deliberately not modeled.** It's an authored self-assessment; serde drops unknown fields silently (`frontmatter.rs:12`). A verifier finalizes it out-of-band.
- **Orphan reporting is maximal-subtree.** A dir straddling a covered root is skipped so the real gap (`src/api`, not `src` or the repo root) is reported once (`check.rs:190`).
- **Stale is skipped for dangling roots.** A dangling root invalidates staleness, so `check` filters those before calling `git::staleness` (`check.rs:106`).

## 9. Dependencies & boundaries

- **Calls out to:** `crate::cmd::CmdContext` (`cmd.rs:100`, scope resolution), `crate::workspace::{discover::discover_files, WorkspaceContext, paths::normalize_rel_path}` (ignore-aware walk + path normalization), `crate::lang::CodeLanguage` (supported-file filter), `serde`/`serde_yaml`/`serde_json`, `anyhow`, and the `git` binary via `std::process::Command` (`git.rs:94`).
- **Called in by:** `src/main.rs` only — the `Command::Codemap` dispatch (`main.rs:917`). Public surface: `codemap::ls`, `codemap::check::check`, `codemap::render::render`, plus `check::DEFAULT_MIN_FILES`/`check::Lints` referenced by clap (`main.rs:247`).
- **Owns / does not own:** Owns the `CODEMAP.md` frontmatter contract, coverage/orphan algorithm, and the git-staleness heuristic. Does **not** own map authoring, any database/table, or the ignore ruleset (that's `workspace::ignore`). No side effects beyond reading files and spawning `git`.

## 10. Open questions / staleness

- [ ] `split_frontmatter` (`frontmatter.rs:77`) handles `\n`, `\r\n`, and EOF-only frontmatter; very unusual mixed line endings are untested beyond the listed markers.
- [ ] `commit_distance` uses `git rev-list --count <commit>..HEAD` over the whole repo, not scoped to the subtree (`git.rs:80`) — so the "N commits behind" count is repo-wide, while `changed` is subtree-scoped. Possibly intentional, but worth noting as a mild inconsistency.
- [ ] The frontmatter contract is documented as living in `~/.claude/templates/codemap.md` (`frontmatter.rs:3`), a path outside this repo — the two can drift independently.
