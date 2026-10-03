---
id: 69
title: "Spaces as task-bearing containers — space-native tasks with their own alias"
status: done
priority: medium
labels:
  - feat
supersedes: 68
---

> **Done.** Shipped on master. Spaces carry a required uppercase alias; tasks
> belong to exactly one owner (project XOR space) enforced by a DB CHECK, with
> per-owner seq via partial unique indexes and cross-container alias uniqueness
> via triggers. `kdb tasks add -S`, `kdb render --space`, and `tasks list -S`
> (space-native + member rollup) are wired end-to-end. The schema landed as
> migration `0008` applied to the live DB, then folded into `0003`/`0007` for a
> clean fresh-install sequence (no build-then-rebuild). Owner-reassignment
> (project→space) was deferred per open question 2.

# iss-0069 :: Spaces as Task-Bearing Containers

## Problem

A task must belong to a project (`tasks.project_id NOT NULL`). The load-bearing
reason isn't hierarchy — it's the **task id**: ids are `{ALIAS}-{seq}`, and the
alias comes from the project (`projects.alias`). Spaces deliberately carry no
alias and own no tasks — they're "organizational only."

That forces ceremony wherever a space is itself the working unit. Concrete case:
the `iceberg` space's *own* work (the Site and Outreach streams in
`studio/.plan/week.md`) has no home unless we invent a phantom `iceberg`/ICE
project that exists only to hold tasks the space conceptually owns. That phantom
project then collides with the space over the shared `studio/` path — the entire
premise of the now-superseded iss-0068.

The clean model: a space is a real working unit that can own tasks directly. A
task belongs to exactly one container — a project **or** a space. Clients that are
genuinely their own efforts (adrata/ADR, quartile/SFD) stay member projects; the
lab's own work becomes space-native under `iceberg`/ICE-the-space.

## Proposal

Give spaces an `alias` and let tasks attach to a space instead of a project. A
task has exactly one owner: `(project_id, space_id)` is an XOR. External ids come
from whichever owner holds the task — `ICE-0042` when the owner is the iceberg
space, `ADR-0013` when it's the adrata project. Nothing changes for project-owned
tasks.

This subsumes iss-0068's rollup: the space owns `<space.path>/.tasks/` outright
because it owns tasks, so there is no competing project board to overwrite and no
path-collision guard to build. `kdb render --space` still materializes the
aggregate board (space-native tasks + member-project rollup); it just no longer
has to arbitrate a shared path.

## Changes

### 1. Schema — migration `0008_space_tasks.sql`

- `ALTER TABLE spaces ADD COLUMN alias TEXT` with the same shape/constraint as
  `projects.alias` (`UPPER`, len 2–6, `GLOB '[A-Z][A-Z0-9]*'`, **unique across
  projects *and* spaces** so ids never clash — enforce with a trigger or a shared
  lookup, since a plain per-table `UNIQUE` won't span both).
- Rebuild `tasks`: make `project_id` **nullable**, add `space_id INTEGER
  REFERENCES spaces(id)`, and add the owner XOR check:
  `CHECK ((project_id IS NULL) <> (space_id IS NULL))`.
- Replace `UNIQUE (project_id, seq)` with two **partial** unique indexes (SQLite
  treats NULLs as distinct, so one combined constraint won't hold):
  - `CREATE UNIQUE INDEX ... ON tasks(project_id, seq) WHERE project_id IS NOT NULL`
  - `CREATE UNIQUE INDEX ... ON tasks(space_id, seq)   WHERE space_id   IS NOT NULL`
- Keep the existing `parent_id`/`seq`/`child_seq` invariant unchanged. Subtasks
  inherit their owner from the parent (owner columns propagate; a child of a
  space-native task is space-native).
- Backfill: existing rows keep `project_id`, `space_id = NULL`. No data moves.

### 2. `src/spaces.rs`

- Add `alias` to `Space`, `AddArgs`, `EditArgs`, `SELECT_COLS`, and both render
  functions. `spaces add` requires `--alias`; `spaces edit --alias` mutates it.
- Alias validation shared with projects (extract the check if it isn't already).

### 3. `src/tasks.rs` — the owner abstraction

This is the bulk. Every place that assumes `project_id` non-null must handle the
space case:

- `TaskView` / `Task`: carry the owner as an enum-ish pair (project or space) plus
  the resolved alias. `external_id()` formats from whichever alias is present —
  `format_external_id` already takes the alias as a param, so the change is at the
  join/resolution layer, not the formatter.
- `AddArgs`: accept `space_id` as an alternative to `project_id`; enforce XOR.
  `seq` allocation reads the per-owner max (`MAX(seq) WHERE space_id = ?`).
- `ListFilters`: `-S/--space` already rolls up member projects; extend it to
  **union** space-native tasks. Add a way to select a single owner (space-native
  only) for the render path.
- Every `WHERE project_id = ?` query, the `CHAIN_CTE` alias join, reparenting,
  and move/order logic: audit for the null-owner branch.

### 4. `src/materialize.rs` — space board

- `materialize_space(conn, root, slug)`: render `<space.path>/.tasks/index.md`
  with the space's own tasks first (status spine, per-task `T-*.md` files in that
  dir), then a `## {ALIAS} · {name}` group per member project linking into each
  member's own `.tasks/` via relative path. Reuse the extracted
  `render_status_sections` helper (heading level + link prefix) from iss-0068's
  plan.
- Member per-task files stay owned by each member's own `materialize` run; the
  space board links to them, doesn't duplicate them.

### 5. `src/cmd.rs` + `src/main.rs`

- `render`: add `-S/--space <slug>` → `materialize_space`; mutually exclusive with
  `file`/`--project`/`--all`.
- `tasks add`: allow `-S/--space <slug>` as the owner (XOR with `-P`).
- `spaces add/edit`: thread `--alias`.

### 6. Docs

- Update the CLAUDE.md kdb section: spaces now carry an alias and can own tasks;
  `render --space`, `tasks add -S`. Update the ontology mapping row for Space.

## Migration for the iceberg case (post-ship)

```
kdb spaces edit iceberg --alias ICE          # give the space its alias
# move the lab's own (Site/Outreach) tasks from the phantom ICE project to the space,
# then delete the phantom project; adrata/quartile stay as member projects.
kdb render --space iceberg
```

## Open questions

1. **Alias uniqueness across projects+spaces.** A trigger enforcing "no alias in
   `spaces` collides with `projects` and vice versa" is the honest fix. Confirm we
   want hard enforcement vs. advisory (hard — id collisions are silent corruption).
2. **Reassigning owner.** Do we support moving a task from a project to a space
   (and renumbering `seq` into the new owner's sequence), mirroring the existing
   reparent-renumbers-child_seq logic? Likely yes, but can land in a follow-up.

## Motivation

The space is the cockpit — the day is planned at the space level, spanning the
lab's own work and its client projects. The data model should let the space own
that work directly instead of routing it through a project that exists only to
satisfy the "tasks need an alias" constraint. Give the alias to the space and the
constraint is satisfied at the right level.
