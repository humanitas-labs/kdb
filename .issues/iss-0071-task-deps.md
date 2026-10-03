---
id: 71
title: "Task dependencies — blocks edges, derived blocked state, and `tasks ready`"
status: proposed
priority: high
labels:
  - feat
---

# iss-0071 :: Task Dependencies

## Problem

kdb tasks are flat rows with a status and a priority. Ordering between tasks lives only in plan documents (for example the "Hard prerequisites" and "Blocked" columns in `labs/projects/threads/.plan/desktop-tasks.md`). Agents running in parallel cannot ask the store what is unblocked, so a human re-reads the graph at every wave, and the hand-written "Blocked" cells go stale the moment a task closes.

## Design

### Schema

One new table. Edges are directed: `task_id` cannot start until `depends_on` is resolved.

```sql
CREATE TABLE task_deps (
  task_id     INTEGER NOT NULL REFERENCES tasks(id) ON DELETE CASCADE,
  depends_on  INTEGER NOT NULL REFERENCES tasks(id) ON DELETE CASCADE,
  kind        TEXT NOT NULL DEFAULT 'blocks' CHECK (kind IN ('blocks','related')),
  created_at  TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
  PRIMARY KEY (task_id, depends_on),
  CHECK (task_id <> depends_on)
);
CREATE INDEX idx_task_deps_on ON task_deps(depends_on);
```

Rules:

- Cross-project and cross-space edges are allowed. A Threads task may depend on a kdb task.
- Cycle detection runs on insert, in the application, by walking `blocks` edges from `depends_on` back to `task_id`. Reject with the cycle path in the error.
- `related` edges are informational and never affect readiness.

### Blocked is derived, never stored

There is no `blocked` status. A task is blocked when it is open and any `blocks` dependency is unresolved. Resolved means the blocker's status has `is_closed = 1`.

This requires one data fix: `parked` is currently `is_closed = 1`, which would let a parked blocker release its dependents. Parked becomes `is_closed = 0, is_hidden = 1`. Hidden already keeps it out of open lists; closed should mean finished. `closed_at` stops being stamped on park.

### Gates

Milestones (MS1 to MS4 in the Threads plan) are tasks like any other, carrying the label `gate`, with no deliverable of their own. Downstream tasks depend on the gate; the gate is closed by hand when its acceptance checks pass. No new concept, and `ready` handles them for free.

### CLI

```
kdb tasks deps add <id> --on <id>...         # add blocks edges (--related for the other kind)
kdb tasks deps rm  <id> --on <id>...
kdb tasks deps show <id>                      # blocked by / blocks, both directions, with status
kdb tasks add <title> --after <id>...         # set deps at creation
kdb tasks ready [-P|-S] [-c <cycle>] [-n N]   # open, not in_progress, no unresolved blockers
                                              # ordered by priority then order; --json
kdb tasks list [--blocked | --ready]          # filters; default output marks blocked rows
kdb tasks view <id>                           # gains "Blocked by" and "Blocks" sections
kdb tasks done <id>                           # prints any tasks this release
```

`ready` is the agent entry point. A `par()` worker runs `kdb tasks ready -P threads --json`, picks the top row, and starts it.

### Render

Task tables in `.tasks/index.md` gain a `Blocked by` column listing unresolved blocker ids. The icon column from [iss-0070](iss-0070-status-icons.md) can reuse the dotted circle, dimmed, for blocked rows if a distinct glyph is wanted. Per-task files list both directions.

### Out of scope, follow-on issues

- Claiming: `tasks start <id> --by <agent>` with an `assignee` column, refusing a second claim. Needed for `par()` to be fully self-scheduling; separate issue.
- Event log: `task_events` for reconstructing a wave afterward. Separate issue.
- Importing prerequisites from existing plan docs: a one-off loop over `tasks deps add` once this lands, not a feature.

## Where it lands

In the rewrite, not the current tree. `src/tasks.rs` is 2,000 lines and is being replaced; the new task module starts from this schema. The Threads plan continues on the hand-maintained columns until then.

## Open questions

1. Should `ready` exclude tasks whose parent is blocked? Proposed: yes, blockedness inherits down the `parent_id` chain.
2. Should `done` on a task with open dependents warn, or is that normal? Proposed: normal, no warning.
