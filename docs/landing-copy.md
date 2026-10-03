# kdb — landing page copy

Pulled from the v1 landing app (`apps/landing/src/app/page.tsx` on master, removed from v2). This is the product description for v2: it never mentioned code-language support, only the markdown graph and the task layer.

## Hero

**Your context should all live in a single knowledge repository.**

> If your data is stored in a database that a company can freely read and access — i.e. not end-to-end encrypted — the company will eventually update their ToS so they can use your data for AI training. The incentives are too strong to resist. — kepano

kdb is a CLI. Your database is a SQLite file on disk. Your tasks are markdown files in your repo. There is no server with your work on it. There is no ToS to update. Delete the binary tomorrow — your data is exactly where you left it.

## Principles

These are the things kdb is opinionated about.

- **Single source of truth.** Notes, tasks, projects, cycles — one repository, one database. No tool-switching. No wondering where a decision was written down. No sync lag between Jira, Notion, and your editor.
- **Context consolidation is imperative.** Agents need consolidated context to operate effectively. Your notes, tasks, and plan have to sit on the same surface an agent can read.

## On disk

A kdb workspace is just a folder on disk. Every byte kdb writes into it is visible to you: a SQLite index in `.kdb/`, cycle files in `.cycles/`, materialized tasks in each project's `.tasks/`, and your own notes wherever you put them.

```
workspace/
├── .kdb/
│   └── index.db                  # SQLite: projects, tasks, cycles, labels
├── .cycles/
│   ├── index.md                  # rollup of every cycle
│   ├── C-14.md                   # active
│   └── C-13.md
├── projects/
│   ├── kdb/
│   │   ├── .tasks/
│   │   │   ├── index.md
│   │   │   ├── T-0120.md
│   │   │   └── T-0121.md
│   │   ├── notes/
│   │   │   └── architecture.md
│   │   └── README.md
│   ├── project-b/
│   │   └── .tasks/
│   │       └── T-0045.md
│   └── project-c/
│       └── docs/
│           └── roadmap.md
├── SOP/
│   └── release.md
└── README.md
```

## Your plan

- **Projects.** Register every project with a slug and a 3-letter alias. Tasks inherit stable IDs like HRM-0120 — same shape across every repo you work in.
- **Tasks.** Priorities, statuses, cycles, labels. One row in SQLite, one T-NNNN.md file on disk. Your editor is the UI. Git is the sync.
- **Cycles.** Week-long sprints with start and end dates. Planned, active, done, abandoned — nothing more. Scope the work, ship it, close the loop.

```
$ kdb projects add --slug hermaeus --alias HRM --path projects/hermaeus
registered: hermaeus (HRM)
$ kdb tasks add --project hermaeus "Wire source-link hover cards" -p 2 -c C-14
HRM-0121 added
$ kdb tasks list --status in_progress,open -n 3
HRM-0119  in_progress  p1  C-14  Deploy Archil mount
HRM-0120  open         p1  C-14  Entity resolver backfill
HRM-0121  open         p2  C-14  Wire source-link hover cards
```

```
$ kdb render --project hermaeus --limit 10
projects/hermaeus/.tasks/index.md
projects/hermaeus/.tasks/T-0119.md
projects/hermaeus/.tasks/T-0120.md
projects/hermaeus/.tasks/T-0121.md
$ kdb cycles list
C-14  active   2026-04-20  2026-04-27
C-13  done     2026-04-13  2026-04-20
C-12  done     2026-04-06  2026-04-13
```

## Your knowledge base

- **One graph.** Headings are nodes. Links are edges. Every markdown file in your repo is parsed, every link resolved — no indexing server, no cloud, no seat fee.
- **Broken links, found.** `kdb check` reports broken links, broken embeds, and orphan files across the project. Wire it into CI and stop shipping rot into your own docs.
- **Transclusion that works.** Compose documents from canonical sources with `![[file#heading]]`. `kdb render` resolves embeds recursively and prints to stdout.

```
$ kdb outline notes/architecture.md
notes/architecture.md
  # Architecture          L1
  ## Overview              L5
  ## Components            L22
    ### Ingest             L24
    ### Store              L40
  ## Data flow             L68
```

```
$ kdb refs notes/architecture.md#components
notes/index.md:12         [architecture › components](architecture.md#components)
notes/onboarding.md:34    [[architecture#components]]
SOP/release.md:8          ![[architecture#components]]
```

```
$ kdb check
broken links:
  notes/old-plan.md:15 → roadmap.md (not found)
  SOP/release.md:42   → architecture.md#deploy (no such heading)
broken embeds:
  notes/index.md:3    ![[glossary#obsolete-term]]
orphan files:
  notes/draft-2024.md (no inbound links)
```

```
$ kdb render SOP/release.md
# Release SOP
## Preflight
- Verify architecture notes
  (inlined from architecture.md#components)
- ...
## Deploy
  (inlined from deploy.md#steps)
- ...
```
