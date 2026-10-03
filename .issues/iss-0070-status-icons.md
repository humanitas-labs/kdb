---
id: 70
title: "Status icons in rendered task tables"
status: proposed
priority: medium
labels:
  - feat
---

# iss-0070 :: Status Icons in Rendered Task Tables

## Problem

`kdb render --project` emits plain tables: id, title, priority. Status is only visible from which section a row sits in. A glyph per status, in the row, reads faster and matches how the Threads app and the plan dependency graphs already draw tasks (moon phases).

## Design

Icons are SVG files in [docs/assets/](../docs/assets/), copied into `.kdb/icons/` on `kdb init` (and on first render if missing). The rendered table gets a leading icon column referencing them by relative path, e.g. `![](../.kdb/icons/in_progress.svg)`. Reference layout: [docs/assets/reference.png](../docs/assets/reference.png).

| Status | Icon | File |
|---|---|---|
| backlog | dotted circle | `backlog.svg` |
| cycle, today (queued) | new moon, outline | `queued.svg` |
| in_progress | waning crescent | `in_progress.svg` |
| in review | waning gibbous | `in_review.svg` |
| done | full circle, green | `done.svg` |

Mapping lives in the data, not the code: add an `icon TEXT` column to `task_statuses` holding the file name. A status with no icon renders an empty cell. `parked` has no icon yet.

## Open questions

1. The fills are tuned for dark backgrounds (`#fff`, `#b5d1ff`). On a light theme the queued icon vanishes. Either ship a second set, or pick mid-tone fills that read on both.
2. Paper and Zed preview both render `![]()` images from relative paths; confirm the SVGs scale to row height and sit on the baseline. If not, fall back to a Unicode glyph column (`◌ ○ ◐ ◑ ●`).
3. Carry this into the kdb rewrite rather than the current tree; it is a render-layer change and should land with the new materializer.
