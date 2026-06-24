---
domain: "db"
repo: "kdb"
root: "src/db"
owner: "kdb"
updated: 2026.06.24
commit: "8c33d68"
confidence: medium
---

# db — Code Map

> A map, not a manual. It points to where things live and how a request flows;
> it does not duplicate per-function detail (that's what the code and `kdb outline` are for).

## 1. What this is

The SQLite-backed relational layer for the kdb workspace: it owns connection
setup and the embedded, ordered migration runner that materializes the schema
(projects, cycles, tasks, labels, statuses, full-text search). Scope is *schema
and connection plumbing only* — opening the DB at `.kdb/index.db`, enabling
pragmas, and applying migrations. The query/mutation logic for each entity lives
in sibling modules (`tasks.rs`, `projects.rs`, `cycles.rs`, …), not here.

## 2. Shape — diagram

```
 caller (cmd.rs, tasks.rs, …)
   │ db::open(root)
   ▼
┌──────────────────────────┐
│ db::open                 │  src/db/mod.rs:59
│  • Connection::open      │   .kdb/index.db
│  • PRAGMA foreign_keys=ON│
│  • PRAGMA journal_mode=WAL│
└────────────┬─────────────┘
             │ migrate(&conn)
             ▼
┌──────────────────────────┐      ┌───────────────────────────┐
│ db::migrate              │─────▶│ MIGRATIONS (embedded)     │
│  read PRAGMA user_version│      │ src/db/mod.rs:29          │
│  for each idx > version: │      │  0001_relational.sql      │
│   execute_batch(sql)     │◀─────│  0002_tasks_order.sql     │
│   bump user_version      │      │  0003_customizable_statuses
│ src/db/mod.rs:72         │      │  0004_status_hidden.sql   │
└────────────┬─────────────┘      │  0005_search.sql          │
             │                    │  0006_search_kind.sql     │
             ▼                    └───────────────────────────┘
      rusqlite::Connection  ──▶ returned to caller
```

## 3. Entry points

The module exposes three public items; all real work funnels through `open`.

| Trigger | Handler (`file:line`) | Purpose |
|---|---|---|
| `db::open(root)` | `src/db/mod.rs:59` | Open/create `.kdb/index.db`, set pragmas, run pending migrations, return `Connection`. |
| `db::db_path(root)` | `src/db/mod.rs:54` | Compute canonical db path (`<root>/.kdb/index.db`) without opening. |
| `db::DB_FILE` | `src/db/mod.rs:26` | Const filename (`index.db`) inside `.kdb/`. |

## 4. Lifecycle — the trace

A command (e.g. `kdb tasks list`) gets a connection like this:

1. **caller invokes open** — `src/cmd.rs:751` (`tasks_list`) — `let conn = db::open(&ctx.workspace.root)?;`
2. **resolve path** — `src/db/mod.rs:60` → `db_path(root)` joins `ROOT_MARKER` (`.kdb`, `src/workspace/root.rs:22`) + `DB_FILE`.
3. **open connection** — `src/db/mod.rs:62` — `Connection::open(&path)`; creates the file if absent.
4. **enable FK enforcement** — `src/db/mod.rs:63` — `PRAGMA foreign_keys = ON` (off by default in SQLite).
5. **set WAL** — `src/db/mod.rs:65` — `PRAGMA journal_mode = WAL`.
6. **migrate** — `src/db/mod.rs:67` → `migrate(&conn)`.
7. **read version** — `src/db/mod.rs:74` — `PRAGMA user_version` gives the highest applied migration index.
8. **apply pending** — `src/db/mod.rs:77` — iterate `MIGRATIONS`; `version = idx+1`; skip if `version <= current`, else `execute_batch(sql)` then bump `user_version`.
9. **return** — `src/db/mod.rs:68` — caller receives a ready `Connection` and runs entity SQL itself.

## 5. File index

**`src/db/`**
| File | Role |
|---|---|
| `mod.rs` | Connection open, pragmas, embedded-migration runner, `user_version` tracking, idempotency test. |

**`src/db/migrations/`** (embedded via `include_str!`, applied in array order)
| File | Role |
|---|---|
| `0001_relational.sql` | Base schema: `projects`, `cycles`, `tasks`, `labels`, `task_labels`, `meta`; hardcoded CHECK status constraints; seeds `meta.top_n=10`. |
| `0002_tasks_order.sql` | Adds lexical `"order"` column to `tasks`, backfills from `seq`, indexes `(project_id, parent_id, "order")`. |
| `0003_customizable_statuses.sql` | Introduces `project_statuses`/`task_statuses` lookup tables; rebuilds `projects`+`tasks` to use FK status (drops CHECK); adds `child_seq`, `deleted_at` to tasks. |
| `0004_status_hidden.sql` | Adds `is_hidden` to both status tables; marks `done` hidden. |
| `0005_search.sql` | FTS5 `search_fts` virtual table + `search_meta` (incremental sync state) + `collections`. |
| `0006_search_kind.sql` | Recreates `search_fts` with a `kind` column (`docs`/`code`); clears `search_meta` to force reindex. |

## 6. Key types & contracts

| Type | `file:line` | What it carries |
|---|---|---|
| `MIGRATIONS: &[(&str, &str)]` | `src/db/mod.rs:29` | Ordered `(name, sql)` pairs; array index + 1 == schema version. Append-only. |
| `rusqlite::Connection` | `src/db/mod.rs:59` (return) | The handle every sibling module uses; pragmas already set. |
| `DB_FILE: &str` | `src/db/mod.rs:26` | `"index.db"` — the file under `.kdb/`. |
| `PRAGMA user_version` | `src/db/mod.rs:74,84` | Integer cursor of applied migrations; the migration runner's only state. |

## 7. Extension points

| To do this… | Touch | Notes |
|---|---|---|
| Add a schema change | new `src/db/migrations/NNNN_*.sql` + register in `MIGRATIONS` (`src/db/mod.rs:29`) | Append a new tuple at the END only; index implies version. Never edit an applied migration. |
| Change DB location/name | `DB_FILE` (`src/db/mod.rs:26`), `db_path` (`src/db/mod.rs:54`) | `.kdb` dir name comes from `ROOT_MARKER` (`src/workspace/root.rs:22`). |
| Change connection-level behavior | `open` (`src/db/mod.rs:59`) | Pragmas (FK, WAL) set here before `migrate`. |

## 8. Invariants & gotchas

- **MIGRATIONS is append-only and order-sensitive** — version is derived from array index (`idx + 1`, `src/db/mod.rs:78`). Reordering, inserting, or rewriting an entry silently corrupts every existing DB's `user_version` mapping. Comment at `src/db/mod.rs:28` states this.
- **`user_version` is the sole migration ledger** — there is no `_migrations` table. A DB is "current" iff `user_version == MIGRATIONS.len()` (asserted in the test, `src/db/mod.rs:105`).
- **Migrations that rebuild tables disable FKs internally** — `0003` wraps its table-swap in `PRAGMA foreign_keys = OFF/ON` (`migrations/0003_customizable_statuses.sql:5,104`) even though `open` turns FKs on; SQLite FK pragma is connection-scoped, not transactional.
- **`open` is idempotent** — calling it on an already-migrated DB applies nothing (the `version <= current` skip, `src/db/mod.rs:79`). Verified by `open_creates_schema_and_is_idempotent`.
- **FTS5 columns can't be ALTERed** — `0006` drops & recreates `search_fts` and wipes `search_meta` to force a full reindex (`migrations/0006_search_kind.sql:8,18`). Any future FTS column change must do the same.
- **Status legality is enforced at the DB, not in Rust** — after `0003`, `tasks.status`/`projects.status` are FKs into the lookup tables (`ON UPDATE CASCADE`). Inserting an unknown status fails at the SQLite layer.

## 9. Dependencies & boundaries

- **Calls out to:** `rusqlite` (bundled SQLite, FTS5), `anyhow`; `crate::workspace::root::ROOT_MARKER` for the `.kdb` dir name.
- **Called in by:** every entity/command module that needs a connection — `src/cmd.rs`, `src/tasks.rs`, `src/projects.rs`, `src/cycles.rs`, `src/labels.rs`, `src/statuses.rs`, `src/search.rs`, `src/materialize.rs` (all via `db::open`).
- **Owns / does not own:** Owns connection setup, pragmas, and the schema definition (all tables/indexes/seed rows live in the migration SQL here). Does **not** own any queries, inserts, updates, or business logic — those live in the sibling modules, which receive the bare `Connection`.

## 10. Open questions / staleness

- [ ] No transactional wrapper around the whole migration run — each `execute_batch` + `user_version` bump is separate. A migration that fails mid-batch could leave the DB partially migrated with a stale `user_version`; relies on SQLite's per-statement-batch atomicity. Worth confirming if this ever bites in practice.
- [ ] `meta.top_n` is seeded (`migrations/0001_relational.sql:70`) and read by the test, but its runtime consumer was not traced here — likely in `materialize.rs`/render logic, outside this domain.
