---
title: "Prosaic compile-checking + procedure ID rename"
date: 2026-07-07
status: approved
affects: "kdb check (compile-check for Prosaic composition); kernel/SOP/* procedure IDs; workspace-wide SOP references"
---

## Context

We formalized Prosaic composition in `kernel/prosaic.md` §5 (2026.07.07): procedures
are declared as `## <ID> :: <Name>` headings, imported via `use <ID> from <path>`,
and invoked via `run <ID>`. §5.4 defines resolution rules but nothing enforces them —
`run`/`use` references are checked by convention only. This plan (a) builds the
compile-check into `kdb check`, (b) audits the 30 existing procedures for spec
conformance, and (c) renames procedure IDs from domain-letter form (`SOP-O04`) to
pure sequential form (`SOP-004`) so IDs stay stable when procedures move modules.

Tracked as **KDB-0009** (this checker) with companion **KDB-0008** (tree-sitter
grammar for the same `use`/`run`/`match`/`//` constructs — highlighting, not
validation).

### Why the rename

`use X from <path>` makes the **path** carry "where a procedure lives." That makes
the module letter in the ID redundant — and worse, it's the exact thing that goes
stale on a move (`SOP-O04` sitting in `sched/` reads as a lie; "O" = ops). The clean
normalization: the **number is permanent identity** ("which"), the **module is data**
(the file it currently lives in), free to change. Navigation by domain survives via
each module's index table + `SOP/_INDEX.md`.

### Decisions (approved)

1. **ID form:** pure sequential `SOP-001 … SOP-030`, zero-padded 3 digits, `SOP-`
   prefix retained as the type tag.
2. **Local tiers namespaced:** global kernel procedures draw from `SOP-0NN`; a
   space/project-tier procedure (e.g. Quartile's `studio/clients/quartile/shared/kernel/SOP/`)
   uses an **uppercase alias** prefix — `SOP-QTP-0NN` — matching the kdb project-alias
   convention (`kdb projects … --alias`). The checker resolves by path regardless; the
   prefix is for human uniqueness.
3. **Numbering order:** deterministic seed — walk SOP files in sorted path order,
   within each file by ascending current ID. Frozen as the crosswalk below.
4. **Transition:** the checker accepts **both** `SOP-XNN` and `SOP-NNN` so the
   checker (workstream A) can land before the rename (workstream C); tighten to
   `SOP-NNN` only after C.

### Frozen crosswalk (old → new)

| Old | New | Name | File |
|---|---|---|---|
| SOP-C01 | SOP-001 | Task Workflow | kernel/SOP/code.md |
| SOP-C02 | SOP-002 | Commit | kernel/SOP/code.md |
| SOP-C03 | SOP-003 | Refactor Cleanup | kernel/SOP/code.md |
| SOP-C04 | SOP-004 | Bugfix | kernel/SOP/code.md |
| SOP-C05 | SOP-005 | Multitasking | kernel/SOP/code.md |
| SOP-C06 | SOP-006 | ADR | kernel/SOP/code.md |
| SOP-C07 | SOP-007 | Bug Report | kernel/SOP/code.md |
| SOP-C08 | SOP-008 | Flow Test | kernel/SOP/code.md |
| SOP-C09 | SOP-009 | Code Map | kernel/SOP/code.md |
| SOP-C10 | SOP-010 | Architecture Change | kernel/SOP/code.md |
| SOP-CM01 | SOP-011 | Workstream Progress Report | kernel/SOP/comms.md |
| SOP-O06 | SOP-012 | kdb Hygiene | kernel/SOP/common/hygiene.md |
| SOP-F01 | SOP-013 | Monthly Close | kernel/SOP/finance.md |
| SOP-F02 | SOP-014 | Model Versioning | kernel/SOP/finance.md |
| SOP-F03 | SOP-015 | Document Filing | kernel/SOP/finance.md |
| SOP-I01 | SOP-016 | Signal Processing | kernel/SOP/intel.md |
| SOP-I02 | SOP-017 | Intelligence Summary | kernel/SOP/intel.md |
| SOP-I03 | SOP-018 | Periodic Intelligence Report | kernel/SOP/intel.md |
| SOP-M01 | SOP-019 | Document Ingestion | kernel/SOP/marina.md |
| SOP-M02 | SOP-020 | Entity Creation | kernel/SOP/marina.md |
| SOP-M03 | SOP-021 | Data Hygiene | kernel/SOP/marina.md |
| SOP-M04 | SOP-022 | Entity Mention Registration | kernel/SOP/marina.md |
| SOP-M05 | SOP-023 | Message Logging | kernel/SOP/marina.md |
| SOP-O05 | SOP-024 | New Project | kernel/SOP/ops.md |
| SOP-O01 | SOP-025 | Cycle Review | kernel/SOP/sched/cycle.md |
| SOP-O02 | SOP-026 | Cycle Planning | kernel/SOP/sched/cycle.md |
| SOP-O07 | SOP-027 | Opportunity Scan | kernel/SOP/sched/cycle.md |
| SOP-O03 | SOP-028 | Daily Shutdown | kernel/SOP/sched/daily.md |
| SOP-O04 | SOP-029 | Morning Build | kernel/SOP/sched/daily.md |
| SOP-S01 | SOP-030 | Reflection Interview | kernel/SOP/self.md |

Note the O-series scatters (O06→012, O05→024, O01→025…) — intended: the domain
grouping was the thing being decoupled.

---

## Workstream B — Audit (do first, settles the procedure set before numbering)

For each of the 30 procedures, check:
- heading form `## <ID> :: <Name>` present; when-to-run paragraph present; `prosaic` fence present.
- every **cross-file** reference is a real `use <ID> from <path>` import; same-file refs stay bare (spec §5.2).
- no dangling IDs (every `run`/`use` resolves per §5.4).
- `use template:` and prose mentions are not miswritten as calls.

Output: an audit table (procedure · file · issue · fix), reviewed before any fix is
applied. Expected findings: a few prose references that should become real `run`/`use`,
possibly a `run` site missing its `use` line. Fix in place; this is also the moment to
confirm the procedure set is final (no merges/splits) before numbers are assigned.

---

## Workstream A — `kdb check` compile-check (core deliverable)

Lives in `src/index/`, alongside the existing link check (`Index::check()` at
`src/index/mod.rs:625`). Reuses the existing markdown parse (headings already give us
the symbol table) + the tree-walk that already visits `fenced_code_block` nodes
(`src/index/markdown.rs:341`).

### Changes

1. **Extract prosaic blocks.** Extend the markdown walk to capture blocks whose info
   string is `prosaic`, with their line offsets. Store as `Vec<ProsaicBlock>` on
   `FileEntry` (each block: source lines + start line).
   - File: `src/index/markdown.rs` (Edit — capture prosaic fences), `src/index/mod.rs`
     (Edit — add field to `FileEntry`, wire into `ParsedDocument`/`FileEntry`).

2. **Parse statements.** After stripping `/* … */` (may span lines) and `// …`
   comments from a block, line-scan for:
   - `use <ID> from <path> [as <alias>]` → import (ID, path, alias, line).
   - `run <ID> …` → invocation (ID, line).
   - Skip the `use template: …` form (ordinary verb, not an import).
   - ID pattern: `SOP-(?:[A-Z]+-)?\d+` (accepts `SOP-004`, `SOP-O04`, `SOP-QTP-004`
     during transition — the optional uppercase segment covers both the legacy domain
     letter and an alias prefix).
   - New file: `src/index/prosaic.rs` (block comment-stripping + statement extraction;
     pure functions, unit-testable in isolation).

3. **Symbol table.** Map every `## <ID> :: <Name>` heading → (file, anchor) across all
   indexed files. Derive from existing `headings`; no new parse.
   - File: `src/index/mod.rs` (Edit — build the procedure map in/near `check()`).

4. **Resolution pass (§5.4).** In `Index::check()`:
   - each `use X from P` ⇒ `P` resolves (root-relative) **and** `P` exports `X`
     (heading present). Else → error.
   - each `run X` ⇒ `X` defined in the same file **or** imported by a `use` in the
     **same block**. Else → error.
   - prose/comment mentions outside fences are not checked (§5.4.3).
   - Emit as a new `ProcedureError { source_file, line, column, raw, reason }`, added to
     `CheckReport`, printed in the existing `file:line:col reason` format so it folds
     into the same non-zero exit.
   - File: `src/index/mod.rs` (Edit — extend `CheckReport`, `print()`, `check()`).

5. **Tests.** Fixture markdown covering: valid import+run; missing path; missing symbol;
   unimported `run`; same-file `run` (ok); `use template:` not misparsed; commented-out
   `run` ignored; cross-tier `use` (global `SOP-0NN` ↔ `SOP-QTP-0NN`) resolves by path.
   - File: `src/index/prosaic.rs` unit tests + an integration fixture under `tests/`.

### Trade-offs

- **Regex line-scan, not the tree-sitter-prosaic grammar.** The grammar lags (KDB-0008)
  and the checker only needs `use`/`run`/heading extraction, which regex handles
  robustly. A later grammar-based rewrite is expected, not a surprise — noted here so it
  reads as a deliberate staging choice.
- **Comment stripping is load-bearing:** a commented `// run SOP-027 later` must not
  count. Strip before matching.

---

## Workstream C — Rename migration (after B + A; verified by A)

1. Generate the migration from the frozen crosswalk. For each old→new: rewrite
   definition headings, all `run`/`use`/prose refs, **link anchors**
   (`#sop-o04-…` → `#sop-029-…`), and index tables.
2. Scope: 25 digimata `.md` files + `~/.claude/CLAUDE.md` + `~/.claude/plans/distributed-snacking-newt.md`,
   plus doctrine, intel templates, and the Quartile tier
   (`studio/clients/quartile/shared/kernel/SOP/` — namespaced `SOP-QTP-0NN`).
3. Update task bodies that cite IDs (KDB-0008/0009) and spec §8.
4. Order longest-ID-first when substituting to avoid partial-token collisions
   (`SOP-O01` before `SOP-O` fragments; not an issue with anchored regex, but the
   script must use word-boundary/anchored matches).

---

## Files touched

```
┌─────────────────────────────────────────────┬──────────────────────────────────┐
│                    File                     │              Action              │
├─────────────────────────────────────────────┼──────────────────────────────────┤
│ src/index/prosaic.rs                        │ Create (parse + resolve + tests) │
│ src/index/markdown.rs                       │ Edit (capture prosaic fences)    │
│ src/index/mod.rs                            │ Edit (symbol table, check(),     │
│                                             │       CheckReport, print)        │
│ src/index/mod.rs (FileEntry)                │ Edit (prosaic_blocks field)      │
│ tests/ (prosaic fixtures)                   │ Create                           │
│ kernel/SOP/*.md (30 procs, 25 files)        │ Edit (rename — workstream C)     │
│ kernel/prosaic.md §8                         │ Edit (conformance status)        │
│ ~/.claude/CLAUDE.md, plans/*.md             │ Edit (rename refs)               │
│ studio/clients/quartile/.../SOP/*          │ Edit (namespaced rename)         │
└─────────────────────────────────────────────┴──────────────────────────────────┘
```

## Verification

- `cargo test` in `projects/kdb` — new prosaic unit + integration tests green.
- `cargo build --release` + reinstall; `kdb check` from digimata root:
  - passes on the (post-audit, post-rename) tree with zero procedure errors;
  - deliberately break one `use` path / one `run` ID in a scratch file → check reports it
    at the right `file:line:col`; revert.
- `rg 'SOP-[A-Z]' --glob '*.md'` across digimata + `~/.claude` returns nothing
  unintentional after workstream C.
- All heading anchors resolve (existing link check stays green).
- Update spec §8; reconcile KDB-0008 (grammar) and close KDB-0009 when A+C land.

## Sequencing

1. Confirm crosswalk (done — frozen above).
2. **B** Audit → review table → fix in place.
3. **A** Build checker (accepts both ID forms) → tests green.
4. **C** Rename migration → run checker to verify.
5. Spec §8 + task reconciliation + grammar follow-up (KDB-0008).

B and A are independent and can proceed in parallel; C waits on both.
