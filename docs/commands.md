# kdb command surface (v2)

The complete surface. Anything not listed here was dropped from v1: `tree`, `deps`, `graph`, `search`, `index`, `collection`, `codemap`, `fmt`, `update`, and every code-language feature. This file is the source for the kdb block in `kernel/tools/ops.md`, which is what agents read.

## Workspace

```
kdb init [PATH]                 # create .kdb/ with config.toml and ignore
kdb root                        # print the workspace root
```

## Markdown graph

```
kdb check [PATH] [--orphans]            # broken links and embeds, orphan count; exit 1 on errors
kdb outline <PATHS>... [-s <anchor>...] [--json]
kdb refs <file.md[#anchor]> [--count | -l/--files | --json]
kdb render <file.md>                    # resolve ![[ ]] embeds to stdout
kdb lsp [PATH]                          # language server over stdio: diagnostics, definition, completion
```

`check` output is one line per problem, `path:line:col broken link RAW (reason)`, then the orphan count, then `N errors` / `N warnings`.

## Planning layer

```
kdb projects list [-a] [-s <space>] [--json] | show <slug> [--json] | add <slug> --alias <AL> -n <name> --path <rel> [...] | edit <slug> [...]
kdb spaces   list [-a] [--json] | show <slug> | add <slug> [--alias <AL>] -n <name> [--path <rel>] | edit <slug> [...]
kdb cycles   list [--json] | show <key> | add <key> --start <date> --end <date> [...] | edit <key> [...]
kdb labels   list [--json] | show <slug> | add <slug> [...] | edit <slug> [...]
kdb statuses list --tasks|--projects [--json] | show <slug> --tasks|--projects | add ... | edit <slug> [--icon <name>|""] ... | rm <slug>
```

Aliases are 2–6 uppercase characters and unique across projects and spaces. Task ids are `<ALIAS>-<seq>` with `.n` for subtasks, e.g. `KDB-0012`, `KDB-0012.3`.

### Tasks

```
kdb tasks list  [-P project | -S space] [-c C-NN] [-s open|all|<slug,...>] [-p 1-5] [-n N] [--include-children] [--blocked | --ready] [--json]
kdb tasks ready [-P project | -S space] [-c C-NN] [-n N] [--json]      # unblocked, not in progress, not parked; by priority
kdb tasks add "<title>" [-P project | -S space] [-b body] [-p 1-5] [-c C-NN] [--parent <id>] [--before <id>] [--after <id>]...
kdb tasks view <id> [--json]
kdb tasks edit <id> [-t title] [-b body] [-p 1-5] [-c C-NN] [--parent <id>] [--status <slug>]
kdb tasks move <id> --before <id> | --after <id> | --top | --bottom
kdb tasks done | park | reopen <id>                # prints "released: ..." when it unblocks dependents
kdb tasks delete <id> [--hard]                     # soft-delete by default
kdb tasks restore <id>
kdb tasks purge [-P project] [--status <s>] [--deleted] [--dry-run]
kdb tasks label add | rm <id> <label>...
kdb tasks deps add <id> <blocker-id>... [--kind blocks|related]
kdb tasks deps rm <id> <blocker-id>...
kdb tasks deps show <id>
```

Dependencies: `blocks` edges make a task blocked until every blocker's status is closed (`done`; `parked` stays open and keeps blocking). Blockedness is inherited by subtasks from their parent chain. `--after` on `tasks add` adds a `blocks` edge and, when the anchor shares the owner, also positions the task after it. Cycles are rejected. Cross-project edges are allowed.

Every task mutation re-renders the owner's board.

## Boards

```
kdb render -P <project> | -S <space> | --all [-n N]
```

Writes `<owner path>/.tasks/index.md` and one `T-NNNN.md` per open top-level task. Tables carry a status icon column (SVGs in `.kdb/icons/`, named by `task_statuses.icon`) and a `Blocked by` column; blocked task files get `blocked_by:` frontmatter.

## Agents writing concurrently

One db file, WAL mode, 5 s busy timeout. `tasks add` assigns sequence numbers inside the insert transaction and retries once on conflict, so parallel agents can add tasks to the same project without coordination.
