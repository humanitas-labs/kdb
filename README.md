# kdb

**Your context should all live in a single knowledge repository.**

kdb is a CLI and language server for a workspace of markdown files. It keeps the link graph between your notes honest, and it tracks projects, tasks, and cycles in a SQLite file that sits in the same folder. Your tasks render to markdown files beside your code. There is no server with your work on it. Delete the binary tomorrow and your data is exactly where you left it.

> If your data is stored in a database that a company can freely read and access — i.e. not end-to-end encrypted — the company will eventually update their ToS so they can use your data for AI training. The incentives are too strong to resist. — kepano

## Principles

- **Single source of truth.** Notes, tasks, projects, cycles: one repository, one database. No wondering where a decision was written down, no sync lag between a tracker and your editor.
- **Context consolidation is imperative.** Agents need consolidated context to work well. Your notes, tasks, and plan sit on the same surface an agent can read, and agents can write to the task database in parallel.

## Install

```
cargo install --path .
```

> During the v2 rewrite the binary installs as `kdb2` so it can run beside v1. It becomes `kdb` at release.

Then mark a folder as a workspace:

```
$ cd ~/notes
$ kdb init
```

## On disk

A workspace is a folder containing `.kdb/`. Every byte kdb writes is visible to you: the database and status icons in `.kdb/`, rendered task boards in each project's `.tasks/`, and your own notes wherever you put them.

```
workspace/
├── .kdb/
│   ├── config.toml
│   ├── ignore                    # paths to skip, one per line
│   ├── index.db                  # SQLite: projects, spaces, tasks, cycles, labels, dependencies
│   └── icons/                    # status icons used by the rendered boards
├── projects/
│   └── hermaeus/
│       ├── .tasks/
│       │   ├── index.md          # generated board
│       │   ├── T-0002.md
│       │   └── T-0003.md
│       └── notes/
│           └── architecture.md
├── SOP/
│   └── release.md
└── README.md
```

## Your plan

- **Projects** have a slug, a path in the workspace, and a 2–6 letter alias. Tasks get stable ids from the alias, like `HRM-0121`, with `.n` suffixes for subtasks.
- **Spaces** group projects. A space can own tasks of its own and renders a rollup board across its projects.
- **Tasks** carry a status, a priority from 1 to 5, an optional cycle, labels, and an order within their list. One row in SQLite, one `T-NNNN.md` file on disk. Your editor is the UI and git is the sync.
- **Cycles** are dated sprints: planned, active, done, or abandoned.
- **Dependencies** say what a task waits on. A task with an open blocker is blocked, subtasks inherit their parent's blockers, and `tasks ready` lists what can start now.

```
$ kdb projects add hermaeus --alias HRM --path projects/hermaeus
added project hermaeus [HRM] (projects/hermaeus)
$ kdb cycles add C-14 --start 2026-10-05 --end 2026-10-11 --status active
added cycle C-14 (2026-10-05 → 2026-10-11)
$ kdb tasks add -P hermaeus "Deploy Archil mount" -p 1 -c C-14
added task HRM-0001
$ kdb tasks add -P hermaeus "Entity resolver backfill" -p 1 -c C-14 --after HRM-0001
added task HRM-0002
$ kdb tasks add -P hermaeus "Wire source-link hover cards" -p 2 -c C-14 --after HRM-0002
added task HRM-0003
$ kdb tasks edit HRM-0001 --status in_progress
updated task HRM-0001
```

```
$ kdb tasks list -P hermaeus
id        st    p  title
HRM-0001  [~]  1  Deploy Archil mount
HRM-0002  [ ]  1  Entity resolver backfill  (blocked by HRM-0001)
HRM-0003  [ ]  2  Wire source-link hover cards  (blocked by HRM-0002)
$ kdb tasks done HRM-0001
released: HRM-0002
HRM-0001 -> done
$ kdb tasks ready -P hermaeus
id        st    p  title
HRM-0002  [ ]  1  Entity resolver backfill
$ kdb tasks deps show HRM-0003
HRM-0003  blocked  Wire source-link hover cards

blocked by:
- HRM-0002  [ ]  p1  Entity resolver backfill

blocks:
  (none)
```

### Boards

Every task change re-renders its project's board, and `kdb render` does it on demand. Each table row starts with a status icon and names any open blockers.

```
$ kdb render -P hermaeus
wrote /Users/you/notes/projects/hermaeus/.tasks/index.md
```

```markdown
## Cycle (2)

_Scheduled this cycle, not today_

| | Task | Title | Blocked by | Priority |
|---|---|---|---|---|
| ![](../../../.kdb/icons/queued.svg) | [HRM-0002](T-0002.md) | Entity resolver backfill |  | 1 |
| ![](../../../.kdb/icons/queued.svg) | [HRM-0003](T-0003.md) | Wire source-link hover cards | HRM-0002 | 2 |
```

Statuses are rows in the database, not a fixed list. Add your own with `kdb statuses add`, choose which count as closed or hidden, and pick an icon with `--icon`.

### Agents writing at once

The database is one SQLite file in WAL mode with a busy timeout, and task numbers are assigned inside the insert transaction. Several agents can add and update tasks in the same project at the same time without coordinating.

## Your knowledge base

- **One graph.** Every markdown file is parsed. Headings are nodes, and standard links, wikilinks, and `![[embeds]]` are edges. Nothing is indexed ahead of time; the graph is built from your files each run.
- **Broken links, found.** `kdb check` reports broken links and embeds with file, line, and column, counts orphan files, and exits non-zero on errors. Put it in CI.
- **Transclusion.** Compose a document from canonical sections with `![[file#heading]]`. `kdb render <file>` resolves embeds recursively and prints the result.
- **In your editor.** `kdb lsp` puts the same broken-link errors in your editor as you type, with go-to-definition on links and completion for paths and headings.

```
$ kdb check
SOP/release.md:9:5 broken link ../projects/hermaeus/notes/architecture.md#deploy (target heading not found: projects/hermaeus/notes/architecture.md#deploy)
projects/hermaeus/notes/index.md:5:3 broken link roadmap.md (target file not found: projects/hermaeus/notes/roadmap.md)
2 orphan files (run `kdb check --orphans` to list)
2 errors
2 warnings
```

```
$ kdb outline projects/hermaeus/notes/architecture.md
# Architecture  L1
## Overview     L3
## Components   L7
### Ingest      L9
### Store       L13
## Data flow    L17
$ kdb refs projects/hermaeus/notes/architecture.md#components
projects/hermaeus/notes/index.md:3:3  architecture.md#components
```

```
$ cat SOP/release.md
# Release SOP

## Preflight

![[../projects/hermaeus/notes/architecture.md#components]]
$ kdb render SOP/release.md
# Release SOP

## Preflight

## Components

### Ingest

Pulls documents from the mount.

### Store

SQLite plus blob storage.
```

Link resolution: a markdown link or wikilink resolves relative to the file it is in, a `kdb://` link resolves from the workspace root, and a target with no extension gets `.md`.

### Zed

Point Zed's language server at kdb in `settings.json`:

```json
"lsp": {
  "kdb": {
    "binary": {
      "path": "/Users/you/.cargo/bin/kdb",
      "arguments": ["lsp", "/Users/you/notes"]
    }
  }
}
```

Any editor that speaks LSP over stdio works the same way: run `kdb lsp <workspace>`.

## Commands

```
kdb init | root
kdb check [PATH] [--orphans]
kdb outline <PATHS>... [-s <anchor>...] [--json]
kdb refs <file.md[#anchor]> [--count | --files | --json]
kdb render <file.md>
kdb render -P <project> | -S <space> | --all [-n N]
kdb lsp [PATH]
kdb projects | spaces | cycles | labels | statuses   list | show | add | edit ...
kdb tasks list | ready | add | view | edit | move | done | park | reopen | delete | restore | purge | label | deps ...
```

Every command takes `--help`. The full surface with flags is in [docs/commands.md](docs/commands.md).

## License

MIT. Copyright (c) 2026 Humanitas Labs.
