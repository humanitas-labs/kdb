---
id: 63
title: "relational layer — projects, cycles, tasks, labels"
status: in_progress
priority: high
labels:
  - feat
---

# iss-0063 :: Relational Layer — Projects, Cycles, Tasks, Labels

## Problem

kdb today indexes files and code symbols (unstructured side of the knowledge base) but has no structured/relational layer. Operational state — projects, cycles, tasks — lives in free-text markdown files with manual IDs and hand-maintained indexes.

Concrete pain in digimata root:

- `.tasks/` folders accumulate `T-NNNN.md` files forever (hermaeus has 40+). No natural pruning.
- IDs are hand-incremented. Collisions and drift between the per-task files and `TODO.md`.
- No structured querying — can't ask "all P1 active tasks in C-14" without grep gymnastics.
- Cycle metadata (key, dates, top priority, status) is implicit in folder names under `.plan/cycle/`.

kdb's stated vision (`.internal/vision.md`) is "relational data and unstructured data that live in the same queryable layer". The relational tables are the missing half.

## Solution

Extend the existing `.kdb/index.db` with a relational layer. Four new tables (`projects`, `cycles`, `tasks`, `labels` + a `task_labels` join), four new subcommand namespaces (`kdb projects`, `kdb cycles`, `kdb tasks`, `kdb labels`), and a render pipeline that regenerates the existing per-project `.tasks/` views as materialized outputs.

### 1. Schema

```sql
CREATE TABLE projects (
  id          INTEGER PRIMARY KEY AUTOINCREMENT,
  slug        TEXT    NOT NULL UNIQUE,
  name        TEXT    NOT NULL,
  path        TEXT    NOT NULL UNIQUE,  -- relative to kdb root
  status      TEXT    NOT NULL DEFAULT 'active'
              CHECK (status IN ('active','paused','archived')),
  description TEXT,
  created_at  TEXT    NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
  updated_at  TEXT    NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now'))
);

CREATE TABLE cycles (
  id          INTEGER PRIMARY KEY AUTOINCREMENT,
  key         TEXT    NOT NULL UNIQUE,   -- "C-14" or "2026-C-14" — user's choice
  start_date  TEXT    NOT NULL,           -- Monday
  end_date    TEXT    NOT NULL,           -- following Sunday
  description TEXT,                       -- top priority / main effort
  status      TEXT    NOT NULL DEFAULT 'planned'
              CHECK (status IN ('planned','active','done','abandoned')),
  path        TEXT,                       -- rel path to cycle artifacts (e.g. .plan/cycle/C-14/)
  created_at  TEXT    NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now'))
);

CREATE TABLE tasks (
  id          INTEGER PRIMARY KEY AUTOINCREMENT,  -- internal PK
  project_id  INTEGER NOT NULL REFERENCES projects(id),
  seq         INTEGER NOT NULL,                   -- per-project counter
  title       TEXT    NOT NULL,
  body        TEXT,
  status      TEXT    NOT NULL DEFAULT 'open'
              CHECK (status IN ('open','in_progress','done','parked')),
  priority    INTEGER NOT NULL DEFAULT 3 CHECK (priority BETWEEN 1 AND 5),
  cycle_id    INTEGER REFERENCES cycles(id),
  parent_id   INTEGER REFERENCES tasks(id),
  created_at  TEXT    NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
  updated_at  TEXT    NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
  closed_at   TEXT,
  UNIQUE (project_id, seq)
);
CREATE INDEX idx_tasks_status_pri ON tasks(project_id, status, priority, updated_at);

CREATE TABLE labels (
  id    INTEGER PRIMARY KEY AUTOINCREMENT,
  slug  TEXT NOT NULL UNIQUE,
  name  TEXT NOT NULL,
  color TEXT
);

CREATE TABLE task_labels (
  task_id  INTEGER NOT NULL REFERENCES tasks(id) ON DELETE CASCADE,
  label_id INTEGER NOT NULL REFERENCES labels(id) ON DELETE CASCADE,
  PRIMARY KEY (task_id, label_id)
);

CREATE TABLE meta (
  key   TEXT PRIMARY KEY,
  value TEXT NOT NULL
);
-- schema_version, top_n (default 10), render_mode
```

**Display IDs** (never stored; derived from rows):
- Project: `slug` (`hermaeus`).
- Cycle: `key` (`2026-C-14`) or display form `C-14`.
- Task: `{project.slug}-{task.seq}` (e.g. `hermaeus-42`).

### 2. CLI surface

```text
kdb projects list [--status s]
kdb projects add <slug> --path <rel> [--name ...] [--desc ...]
kdb projects edit <slug> [--name ...] [--path ...] [--status ...]
kdb projects show <slug>

kdb cycles list [--status s] [--active]
kdb cycles add  <key> --start <YYYY-MM-DD> --end <YYYY-MM-DD> [--desc ...] [--path ...]
kdb cycles edit <key> [--desc ...] [--status ...] [--path ...] [--start ...] [--end ...]
kdb cycles show <key>
kdb cycles close <key>
kdb cycles open  <key>

kdb tasks [list]                               # cwd-scoped if inside a project; else all
         [--project slug] [--all-projects]
         [--status s] [--cycle key] [--label l]
         [--priority N] [-n N]
kdb tasks add "title" [--project slug] [-p N] [--cycle key] [--label a,b] [-b body]
kdb tasks edit <id>   [--title ...] [-b ...] [-p ...] [--status ...] [--cycle ...] [--label ...]
kdb tasks show <id>
kdb tasks done   <id>
kdb tasks park   <id>
kdb tasks reopen <id>
kdb tasks import [--project slug] [--delete-sources]

kdb labels list
kdb labels add <slug> [--name ...] [--color ...]
kdb labels rm <slug>

kdb render [--project slug | --cycle key | --all]     # regenerate materialized views
```

Conventions:
- `kdb tasks` with no subcommand aliases to `list`.
- `list` default: active (`open`+`in_progress`), ordered `priority ASC, updated_at DESC`, top-N.
- Task ID parsing: `hermaeus-42` → project slug + seq lookup.
- `add`/`edit` with no `-b` opens `$EDITOR`.
- Every mutating command auto-renders the affected project(s) unless `--no-render`.

### 3. Render pipeline — materialized views

Given the DB is source of truth, everything on disk for tasks/cycles is regenerated.

**Per project** (`projects/<slug>/.tasks/`):

- `TODO.md` — rendered digest. Sections grouped by cycle (current cycle first, then other active cycles, then uncycled, then parked). Within each cycle: Stack (`in_progress`) then Heap (`open`). Each line: `- [ ] {slug}-{seq} — {title}` with priority marker if P1/P2.
- `T-{slug}-{seq}.md` — one file per task in the top-N (by `updated_at DESC` among active). Frontmatter + body.

**Per cycle** (`.plan/cycle/<key>/`) — thin pointer, not full ownership:

- Do *not* generate `.plan/cycle/` files yet. `cycles.path` points at the existing artifact folder (e.g. `.plan/cycle/C-14/`). Ownership of `.plan/cycle/` contents is out of scope for v0.1.

**GC**: `kdb render` deletes any `T-*.md` in a project's `.tasks/` that isn't in the top-N result. Never touches DB.

**Top-N** default = 10, stored in `meta.top_n`, overridable via `--n`.

### 4. cwd scoping

`kdb` already walks up from cwd to find `.kdb/` root. Extend: after finding root, compute `cwd` relative to root. Match against `projects.path` (longest prefix wins). Any `kdb tasks`/`kdb render` without `--project` or `--all-projects` scopes to that project. Outside any registered project → error with "specify --project or --all-projects".

### 5. Import migration

`kdb tasks import [--project slug] [--delete-sources]`:

1. If `--project` omitted, derive from cwd (same scoping rule as above).
2. Parse each `.tasks/T-*.md` under the project's path:
   - Frontmatter → fields (id, title, status, priority, cycle, labels).
   - Body → `body`.
   - Preserve numeric seq from the filename (`T-0042.md` → seq=42).
3. Parse `.tasks/TODO.md` for free-text bullets under Stack/Heap with no T-id:
   - Insert as new tasks with auto-assigned seq (continuing from max).
   - **Flag ambiguous parses** (missing metadata) and print them; do not silently insert — require `--force` to accept.
4. `UPDATE sqlite_sequence` (if used) and also track max(seq) per project so future `add` continues cleanly.
5. If `--delete-sources`, unlink the parsed `.md` files.
6. Auto-run `kdb render` at the end.

A separate `kdb cycles import` parses `.plan/cycle/<key>/` folder names to seed the cycles table (setting `path` to the folder). Out of scope for first pass — can be done via `kdb cycles add` per cycle initially.

### 6. Gitignore

Per-project `.gitignore` additions (applied during `kdb projects add` if not present):

```
.tasks/T-*.md
```

`TODO.md` and `.kdb/index.db` stay committed.

## Implementation

### Context

kdb currently has no persistent DB — indexing is in-memory via tree-sitter + walkdir. This issue introduces `rusqlite` (bundled) and a `db/` module, plus four subcommand namespaces.

Existing kdb source layout: `src/{cmd.rs, deps/, fmt/, index/, lang.rs, lib.rs, lsp/, main.rs, project/, render/, resolve/, symbols/, tree.rs, update.rs}`. No sqlite dep yet.

### Changes

1. **Deps** — add `rusqlite` (bundled), `chrono`, `serde_yaml` (for frontmatter parsing during import) to `Cargo.toml`.
2. **`src/db/mod.rs`** — connection management, migrations runner, `apply_migrations()`. Store DB file at `.kdb/index.db`.
3. **`src/db/migrations/0001_relational.sql`** — the schema above. Wired into `apply_migrations`.
4. **`src/models/`** — `project.rs`, `cycle.rs`, `task.rs`, `label.rs`. Row structs + serde + markdown rendering helpers.
5. **`src/cmd/projects/`** — `list.rs`, `add.rs`, `edit.rs`, `show.rs`. Thin CRUD over `projects`.
6. **`src/cmd/cycles/`** — same pattern + `close.rs`, `open.rs`.
7. **`src/cmd/tasks/`** — same pattern + `done.rs`, `park.rs`, `reopen.rs`, `import.rs`.
8. **`src/cmd/labels/`** — `list.rs`, `add.rs`, `rm.rs`.
9. **`src/cmd/render.rs`** — orchestrates per-project and per-cycle rendering. Uses `models/task.rs::render_todo_md` + `render_task_md`. Handles GC.
10. **`src/project/scope.rs`** — extend existing `project/` module with cwd → project-row resolution.
11. **`src/main.rs`** — register new subcommand namespaces under clap.
12. **Auto-render hook** — helper `fn after_mutation(project_id) -> Result<()>` called from every mutating `tasks`/`cycles`/`projects` command. Can be suppressed via `--no-render`.
13. **Import command** — `kdb tasks import` as specced in §5 above.
14. **Version bump** — kdb minor (0.25 → 0.26).

### Files touched

```
┌─────────────────────────────────────────────────┬──────────────────────────────┐
│                      File                       │            Action            │
├─────────────────────────────────────────────────┼──────────────────────────────┤
│ projects/kdb/Cargo.toml                         │ Edit (add deps, bump ver)    │
│ projects/kdb/src/db/mod.rs                      │ Create                       │
│ projects/kdb/src/db/migrations/0001_*.sql       │ Create                       │
│ projects/kdb/src/models/{project,cycle,task,    │ Create                       │
│                          label}.rs              │                              │
│ projects/kdb/src/cmd/projects/*.rs              │ Create                       │
│ projects/kdb/src/cmd/cycles/*.rs                │ Create                       │
│ projects/kdb/src/cmd/tasks/*.rs                 │ Create                       │
│ projects/kdb/src/cmd/labels/*.rs                │ Create                       │
│ projects/kdb/src/cmd/render.rs                  │ Edit (or create if new)      │
│ projects/kdb/src/project/scope.rs               │ Create                       │
│ projects/kdb/src/main.rs                        │ Edit (register subcommands)  │
│ projects/kdb/CHANGELOG.md                       │ Edit (add 0.26 entry)        │
│ projects/kdb/.issues/index.md                   │ Edit (register iss-0063)     │
│ projects/kdb/.issues/iss-0063-*.md              │ Create (this file)           │
│ ontology.md (digimata root)                     │ Edit (task/cycle defs)       │
│ projects/.claude/CLAUDE.md                      │ Edit (Tasks section)         │
│ SOP/tasks.md                                    │ Create (workflow SOP)        │
│ index.md (digimata root)                        │ Edit (layout note)           │
│ .gitignore (digimata root) + per-project        │ Edit (add .tasks/T-*.md)     │
│ .kdb/index.db (digimata root)                   │ Created by `kdb init`        │
│ projects/*/.tasks/T-*.md                        │ Deleted (import --delete-    │
│                                                 │ sources)                     │
└─────────────────────────────────────────────────┴──────────────────────────────┘
```

### Phased rollout

1. **Schema + CRUD** — db module, migrations, `kdb projects` subcommands. Dogfood: register a couple projects.
2. **Tasks CRUD** — `kdb tasks add/edit/show/done/park/reopen/list`. No render yet.
3. **Render pipeline** — `kdb render`, GC, auto-render hook. Golden-file tests.
4. **Cycles CRUD + labels** — finish the table set.
5. **Import** — `kdb tasks import` + dogfood on `projects/hermaeus/.tasks/`.
6. **Root migration** — run `kdb projects add` for every dir in `projects/`, `kdb tasks import --delete-sources` per project, verify.
7. **Docs** — ontology/CLAUDE.md/SOP/index updates.
8. **Commit + ship** — 0.26.0.

### Verification

Build:
- `cargo build --release` in `projects/kdb/`
- `cargo test` — unit tests on db, models, render (golden-file for TODO.md); integration test for import using a fixture `.tasks/` tree.

Dogfood:
- `kdb init` at digimata root (creates `.kdb/index.db`).
- `kdb projects add hermaeus --path projects/hermaeus` (and repeat for each project).
- `cd projects/hermaeus && kdb tasks import` — verify count matches `ls .tasks/T-*.md | wc -l`, ids preserved, bodies intact.
- Spot-check 3 imported tasks against source files.
- `kdb tasks` from inside `projects/hermaeus/` shows scoped top-N.
- `kdb tasks done hermaeus-42` — verify status flip + re-render.
- `kdb render --all` — verify all projects get fresh `TODO.md` + top-N `T-*.md`.
- `kdb check` at digimata root — no broken links after edits.

End state:
- One `.kdb/index.db` at digimata root holds all projects/cycles/tasks/labels.
- Every project's `.tasks/` folder contains only `TODO.md` (committed) + top-N `T-*.md` (gitignored), rendered from DB.
- `kdb tasks add "title"` from any project cwd inserts a row, assigns `{slug}-{next_seq}`, re-renders.
- No manual ID incrementing anywhere.
