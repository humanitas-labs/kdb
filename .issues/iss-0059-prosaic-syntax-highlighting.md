---
id: 59
title: "prosaic language syntax highlighting"
status: done
priority: medium
labels:
  - enhancement
  - languages
---

# iss-0059 :: Prosaic Syntax Highlighting

## Problem

Prosaic is a pseudocode language used for writing SOPs and operational procedures. Currently rendered as plain text in code blocks. Needs syntax highlighting via tree-sitter so it lights up in Zed/VSCode.

## Token Types

| Token | Examples | Color intent |
|---|---|---|
| Comment | `/* ... */` | gray/dim |
| Control flow | `for each`, `if`, `in`, `break` | purple |
| Action verb | `copy`, `write`, `create`, `mark`, `check`, `print`, `list`, `update`, `sort`, `tag`, `assign`, `prune`, `classify`, `pick`, `take`, `allocate`, `link`, `group`, `inventory`, `note`, `review` | blue |
| Block label | `scan:`, `evaluate:`, `budget:`, `output:` | orange |
| File path | `.tasks/TODO.md`, `.plan/cycle/{id}/plan.md` | green |
| Template var | `{cycle_id}`, `{YYYY.MM.DD}` | cyan |
| Operator | `→`, `=`, `×` | red |
| Annotation | `(facts, not feelings)` | gray italic |

## Scope

1. Write tree-sitter grammar for prosaic (`tree-sitter-prosaic`)
2. Wire into kdb's Zed extension so `prosaic` code blocks in markdown get highlighted
3. Add to kdb's language support list

## Notes

- Indentation-based nesting (Python-style)
- Small grammar — ~6-7 token types
- Used in `SOP.md` and potentially other operational docs
