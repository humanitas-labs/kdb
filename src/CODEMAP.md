---
domain: "core"
repo: "dremnik/kdb"
root: "src"
owner: "kdb"
updated: 2026.06.24
commit: "8c33d68"
confidence: high
---

# core — Code Map

> A map, not a manual. It points to where things live and how a command flows;
> it does not duplicate per-function detail (that's what the code and `kdb outline` are for).

## 1. What this is

The crate-level overview: the `kdb` binary's CLI surface (clap parser + `main()` dispatch),
the thin command-implementation layer (`cmd.rs`), and the **relational entity layer** — the
loose `src/*.rs` modules that read/write the SQLite ontology (projects, tasks, cycles,
statuses, labels) plus a handful of cross-cutting utilities (color, lang, tree, search,
update, materialize). The heavy machinery — markdown/code indexing, symbol extraction,
dependency analysis, the LSP, transclusion rendering, fmt headers, codemap assembly, and the
DB schema/migration — lives in `src/<subdir>/`, each with its **own** colocated `CODEMAP.md`.
This map summarizes those subdirs in one line each and points you there.

## 2. Shape — module graph

```
                          ┌──────────────┐
   kdb <args>  ───────────▶│  main.rs     │  clap parse + match
                          │  (Cli/Command│  main.rs:691
                          │   → dispatch)│
                          └──────┬───────┘
                                 │ calls kdb::cmd::* / kdb::lsp::serve
                                 ▼
                          ┌──────────────┐
                          │   cmd.rs     │  CmdContext + one fn per subcommand
                          │  cmd.rs:100  │
                          └──┬────────┬──┘
            relational reads/│        │ filesystem / code intelligence
            writes (db::open)│        │
        ┌───────────────────┘        └───────────────────────┐
        ▼                                                     ▼
┌─────────────────────────────────┐          ┌──────────────────────────────────┐
│ relational entity modules        │          │ subtree domains (own CODEMAP.md)  │
│  projects.rs  tasks.rs           │          │  index/  symbols/  resolve/       │
│  cycles.rs    statuses.rs        │          │  deps/   render/   fmt/           │
│  labels.rs    tasks_import.rs    │          │  lsp/    codemap/  workspace/     │
│  materialize.rs                  │          │  db/  (SQLite schema + open)      │
└──────────────┬──────────────────┘          └──────────────────────────────────┘
               │ db::open(root) → rusqlite::Connection
               ▼
        ┌──────────────┐    util: color.rs · lang.rs · tree.rs
        │  db/ (.kdb)  │         search.rs · update.rs
        └──────────────┘
```

## 3. Entry points

The single binary entry is `main()`; every subcommand routes to a `kdb::cmd::*` function
(or `kdb::lsp::serve` for the LSP).

| Trigger | Handler (`file:line`) | Purpose |
|---|---|---|
| `main()` dispatch | `src/main.rs:691` | parse `Cli`, match `Command`, call into `cmd`/`lsp` |
| `kdb init` / `root` / `check` | `src/cmd.rs:161` / `:216` / `:226` | workspace lifecycle + link/orphan check |
| `kdb tree` / `outline` / `refs` / `deps` / `graph` | `src/cmd.rs:243` / `:291` / `:352` / `:423` / `:458` | filesystem + code-intelligence reads |
| `kdb render` / `fmt` / `update` | `src/cmd.rs:473` / `:523` / `:564` | transclusion / index headers / self-update |
| `kdb lsp` | `src/main.rs:741` → `kdb::lsp::serve` | LSP over stdio (see `lsp/CODEMAP.md`) |
| `kdb projects …` | `src/cmd.rs:570`–`:639` | projects CRUD |
| `kdb tasks …` | `src/cmd.rs:741`+ | tasks list/add/edit/view/move/label/lifecycle |
| `kdb cycles / labels / statuses …` | `src/cmd.rs:1057` / `:1140` / `:1239` | cycles, labels, status lookup-table CRUD |
| `kdb search` / `kdb index` | `src/cmd.rs:1412` / `:1525` | FTS5 search + incremental index sync |

## 4. Lifecycle — the trace

Representative relational path: `kdb tasks list`.

1. **Parse** — `src/main.rs:691` — `Cli::parse()` produces `Command::Tasks { action }`; an
   absent subcommand defaults to `TasksCmd::List { status: "open", … }` (`src/main.rs:762`).
2. **Dispatch** — `src/main.rs` match arm calls `kdb::cmd::tasks_list(...)` (`src/cmd.rs:741`).
3. **Resolve workspace** — `cmd` builds a `CmdContext` (`src/cmd.rs:100`) via
   `CmdContext::from_path` → `WorkspaceContext::discover` (walks up for `.kdb/`).
4. **Open DB** — `db::open(&ctx.workspace.root)` (`src/db/mod.rs:59`) opens `.kdb/kdb.db` and
   runs migrations; see `db/CODEMAP.md`.
5. **Resolve filters** — `resolve_project` / `parse_statuses` (`src/cmd.rs:658` / `:703`) turn
   CLI args (active project default, `open` status alias) into concrete ids.
6. **Query** — `tasks::list(...)` in `src/tasks.rs` runs the SELECT (using `CHAIN_CTE` for the
   parent chain / external-id rendering) and returns rows.
7. **Render** — `tasks::render_list` formats the flat table; `color.rs` colorizes status cells
   when stdout is a TTY (`src/color.rs:22`).

## 5. File index

Loose top-level `src/*.rs` files only. (Subdirectories each have their own `CODEMAP.md`.)

| File | Role |
|---|---|
| `main.rs` | clap `Cli`/`Command` definitions + `async main()` dispatch to `cmd`/`lsp` |
| `lib.rs` | crate root; declares every `pub mod` (the module manifest) |
| `cmd.rs` | one public fn per subcommand; `CmdContext` (workspace + index builders) |
| `projects.rs` | `projects` table: slug/alias/path/status CRUD + render |
| `tasks.rs` | `tasks` table: per-project `seq`, external-id formatting, statuses, ordering |
| `tasks_import.rs` | one-shot importer for legacy `T-NNNN.md` task files → DB rows |
| `cycles.rs` | `cycles` table: `C-NN` key, start/end, status CRUD + render |
| `statuses.rs` | `project_statuses` / `task_statuses` lookup tables (shared `Kind` shape) |
| `labels.rs` | `labels` table + `task_labels` join CRUD + render |
| `materialize.rs` | DB → markdown: per-project `.tasks/index.md` + `T-{seq}.md` files |
| `search.rs` | FTS5 full-text search index: incremental `sync`, `rebuild`, query (`FType`) |
| `tree.rs` | filtered directory-tree builder/renderer for `kdb tree` |
| `color.rs` | `#RRGGBB` → 24-bit ANSI; TTY detection for colorized output |
| `lang.rs` | `CodeLanguage` enum + path-based language detection (shared by code subdirs) |
| `update.rs` | self-update: check GitHub releases, download, verify, replace binary |

**Subtree domains (see colocated `CODEMAP.md`):**

| Subdir | One-line role |
|---|---|
| `db/` | SQLite connection (`db::open`) + schema/migrations for the relational layer → `src/db/CODEMAP.md` |
| `index/` | Markdown parser, vault indexer, link resolver, md refs/deps → `src/index/CODEMAP.md` |
| `symbols/` | Multi-language code symbol extraction → `src/symbols/CODEMAP.md` |
| `resolve/` | Workspace-aware code import resolution → `src/resolve/CODEMAP.md` |
| `deps/` | Code dependency extraction for `kdb deps` → `src/deps/CODEMAP.md` |
| `render/` | `![[file#heading]]` transclusion resolution → `src/render/CODEMAP.md` |
| `fmt/` | Code file index-header generation/maintenance → `src/fmt/CODEMAP.md` |
| `lsp/` | Language Server Protocol implementation → `src/lsp/CODEMAP.md` |
| `codemap/` | CODEMAP index assembly → `src/codemap/CODEMAP.md` |
| `workspace/` | Shared workspace root/config/discovery/paths/ignore → `src/workspace/CODEMAP.md` |

## 6. Key types & contracts

| Type | `file:line` | What it carries |
|---|---|---|
| `Cli` / `Command` | `src/main.rs:30` / `:36` | the full clap command tree; the source of truth for the CLI surface |
| `CmdContext` | `src/cmd.rs:100` | resolved `start` path + `WorkspaceContext` (root, ignore); builds `VaultIndex`/`WorkspaceIndex` |
| `Project` | `src/projects.rs:39` | a project row (slug, alias, path, status) |
| `Status` + `Kind` | `src/statuses.rs:87` / `:48` | a status row and the project-vs-task discriminator (table + flag column) |
| `Cycle` | `src/cycles.rs:36` | a cycle row (`C-NN` key, start/end, status) |
| `Label` | `src/labels.rs:38` | a label row (slug + optional name/color) |
| `CodeLanguage` | `src/lang.rs:15` | language enum + `from_path` detection shared across code subdirs |
| `FType` | `src/search.rs` | search row class (`docs` / `code` / `all`) gating default search scope |

## 7. Extension points

| To do this… | Touch | Notes |
|---|---|---|
| Add a new subcommand | `src/main.rs` (variant in `Command` ~L36), `src/cmd.rs` (impl fn), `src/main.rs:691` (dispatch arm) | follow an existing arm; build `CmdContext::from_path` first |
| Add a new relational entity | new `src/<entity>.rs`, register in `src/lib.rs:24`, add `db/` migration, wire `cmd.rs` + `main.rs` | mirror `labels.rs` for the simplest CRUD shape |
| Add a task/project status field | `src/statuses.rs` (struct + `Kind`), `db/` migration | both lookup tables share the `Status` shape |
| Change task id / ordering format | `src/tasks.rs` (`format_external_id`, `default_order_key`, `CHAIN_CTE`) | per-project `seq`; subtasks use dotted `child_seq` |
| Add a new code language | `src/lang.rs:15` (`CodeLanguage` + `from_path`), then the relevant subdir (`symbols/`, `deps/`, `resolve/`, `fmt/`) | `lang.rs` is the shared registry |
| Change materialized TODO output | `src/materialize.rs` | regenerated from DB; hand-edited `TODO.md` is left alone |

## 8. Invariants & gotchas

- **Workspace discovery walks up for `.kdb/`.** Every command resolves a root via
  `CmdContext::from_path` → `WorkspaceContext::discover` (`src/cmd.rs:111`). Outside a
  workspace, relational/search commands fail; this is intentional.
- **`db::open` runs migrations on every open** (`src/db/mod.rs:59`). The DB is created/upgraded
  lazily — there's no separate migrate step.
- **Task `seq` is per-project, top-level only.** Subtasks (`parent_id IS NOT NULL`) do not
  consume `seq`; they carry a sibling-local `child_seq` and render dotted (`src/tasks.rs` header).
- **Soft-delete is the default.** `tasks delete` sets `deleted_at` and cascades to the subtree;
  `--hard` / `purge` are the only destructive paths.
- **The clap tree in `main.rs` is the CLI contract.** Help text, aliases (`outline`↔`symbols`,
  `view`↔`show`), and default subcommands all live here — keep `cmd.rs` fn signatures in sync.
- **Materialized files are generated, not authored.** Stale `T-*.md` files are removed on
  re-materialize; never hand-edit them (`src/materialize.rs` header).
- **Color output is TTY-gated.** `color.rs` returns plain text when stdout is not a terminal,
  so piped output stays clean (`src/color.rs:22`).

## 9. Dependencies & boundaries

- **Calls out to:** `clap`/`tokio` (CLI + async `main`), `rusqlite` (bundled SQLite, FTS5),
  and the in-crate subtree domains (`index`, `symbols`, `resolve`, `deps`, `render`, `fmt`,
  `lsp`, `codemap`, `workspace`, `db`).
- **Called in by:** the `kdb` binary only; `lib.rs` also re-exports everything for tests and
  the LSP. `main.rs` references `kdb::search::FType` directly (`src/main.rs:2`).
- **Owns / does not own:** owns the CLI surface, command dispatch, the relational entity
  modules' read/write logic, and the FTS5 search index lifecycle. Does **not** own the SQLite
  schema/migrations (that's `db/`), markdown/code parsing (`index/`, `symbols/`), or LSP
  protocol handling (`lsp/`).

## 10. Open questions / staleness

- [ ] §3/§4 line anchors for the long `cmd.rs` task subcommands (`tasks_view` `:913`,
  `cycles_list` `:1057`, etc.) were taken from `kdb outline`; the leading CRUD fns (`:161`–`:741`)
  were read directly. The mid-file task fns were not all opened line-by-line.
- [ ] `FType` exact definition line in `search.rs` not pinned (referenced via `kdb::search::FType`
  in `main.rs:2` and used in `cmd.rs:1412`); the struct/enum body lives below the file header.
- [ ] `lib.rs` doc comment (`src/lib.rs:8`) still lists the older module set in prose; the actual
  `pub mod` list (`:24`–`:46`) is authoritative and includes `search`, `codemap`, `tasks_import`.
