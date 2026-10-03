---
id: 67
title: "tasks move --top panics when first sibling holds the all-min order key"
status: proposed
priority: medium
labels:
  - bug
---

# iss-0067 :: `tasks move --top` Panics on All-Min Order Key

## Problem

`kdb tasks move <id> --top` panics when the current first sibling already holds
the minimum order key:

```
$ kdb tasks move SFD-0023 --top
thread 'main' (1266390) panicked at src/tasks.rs:182:5:
cannot produce key before all-min key: "000000000000"
```

The fractional-indexing keygen at `src/tasks.rs:182` cannot generate a key
*before* `"000000000000"` — there is no room below the minimum — and panics
instead of handling the case.

Observed 2026.07.02 in the digimata workspace: first sibling of the `scotty`
project's task list held order `000000000000`, so any `--top` move panicked.

## Workaround

`kdb tasks move <id> --before <current-first-id>` succeeds — `--before`
generates a key *between* min and the first key's successor rather than
strictly below the first key (produced `0000000000009` in the observed case).

## Fix sketch

When the target position's lower bound is the all-min key, fall back to the
"between min and first" midpoint (what `--before first` effectively does)
instead of trying to go strictly below it. Alternatively: rebalance the
sibling keys when the keyspace floor is hit. Either way, `--top` should never
panic on a reachable state — the all-min key is produced by kdb's own
inserts, so this is self-inflicted input.

## Acceptance

- `kdb tasks move <id> --top` succeeds when the first sibling's order key is
  `"000000000000"` (regression test).
- No panic path remains in the `--top` / `--bottom` keygen; boundary cases
  return a valid key or rebalance.
