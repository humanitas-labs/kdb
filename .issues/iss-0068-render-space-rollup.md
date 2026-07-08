---
id: 68
title: "kdb render --space <slug> — materialize a space-level task rollup"
status: superseded
priority: medium
labels:
  - feat
superseded_by: 69
---

# iss-0068 :: `kdb render --space <slug>` — Space-Level Task Rollup

> **Superseded by [iss-0069](iss-0069-space-owned-tasks.md).** The path-collision
> problem this issue works around (a phantom project sharing the space's path) is
> dissolved by making spaces task-bearing containers with their own alias. The
> `render --space` rollup lives on as one change inside iss-0069.

## Problem

`kdb render` materializes a `.tasks/index.md` per **project** (`-P <slug>`) or for
every project (`--all`). It has no **space** mode. So a space that is a real daily
working unit has no single materialized board — only the per-query view
(`kdb tasks list --space <slug>`), which isn't a durable file the plan can link to.

This bites hardest when a space and one of its member projects **share a path**.
Concrete case: the `acme` space (`path: acme`) contains three projects —
`acme`/ICE (path `acme`, same as the space), `globex`/ADR
(`acme/clients/globex`), `initech`/SFD (`acme/clients/initech`).
`kdb render -P acme` writes `acme/.tasks/index.md` with **only ICE tasks**,
even though that file sits at the space root. Globex (13 open) and Initech
(2 in-progress + 2 cycle + 5 backlog) are invisible at the level where the day is
actually planned — `acme/.plan/week.md` schedules Site/Outreach/**Globex**/
**Initech** streams that map one-to-one onto the three projects.

## Proposal

Add `kdb render --space <slug>`, materializing a rollup at `<space.path>/.tasks/index.md`.

Shape — group **by project, then by horizon**, preserving the time-horizon status
spine (in_progress → today → cycle → backlog → parked/done, the latter hidden):

```
# Acme — Space Board

## ICE · Acme Labs
### In Progress … ### Today … ### Cycle …
## ADR · Globex
### In Progress … ### Today … ### Cycle …
## SFD · Initech Partners
### In Progress … ### Today … ### Cycle …
```

Design decisions to settle:

1. **Path collision.** When a member project shares the space's path (ICE ↔
   acme), the space rollup must be **authoritative** at `<space.path>/.tasks/index.md`,
   with that project appearing as one group inside it. Decide how `-P <that-project>`
   render coexists — either it refuses to overwrite the space board, or it writes to
   an alternate target, or space-membership makes the project non-renderable standalone
   at the shared path.
2. **Backlog volume.** Per-project backlog can be long; in a rollup, collapse it to a
   count + `kdb tasks list` hint (like the current hidden-section rendering) rather than
   full tables, so the board stays a live-work view.
3. **Per-project boards still stand.** `clients/globex/.tasks/`, `clients/initech/.tasks/`
   keep their own `-P` boards for depth; the space board is an additive superset.

## Motivation

The space is the cockpit here — the daily plan already spans all three projects. The
task board should render the same surface the plan schedules, so the linking forcing
function (plan item → `T-*.md`) covers the whole space, not just one project.
