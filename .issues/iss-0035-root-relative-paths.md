---
id: 35
title: Root-Relative Path Support
status: proposed
priority: medium
labels:
  - feat
path: kdb/.issues/iss-0035-root-relative-paths.md
outline: |
  • ISS-0035 :: Root-Relative Path Support      L17
    ◦ Intent                                    L19
    ◦ Examples                                  L23
    ◦ Behavior                                  L33
    ◦ Open Questions                            L40
---

# ISS-0035 :: Root-Relative Path Support

## Intent

Support absolute paths that resolve from the kdb project root, so links from deeply nested files don't require fragile `../../../` chains.

## Examples

```markdown
<!-- currently: fragile relative path from .self/signals/events/ -->
[C-09 plan](../../../.plan/cycle/C-09/plan.md)

<!-- proposed: root-relative -->
[C-09 plan](/.plan/cycle/C-09/plan.md)
```

## Behavior

- Paths starting with `/` resolve from the kdb project root (the directory containing `.kdb/` or `kdb.toml`)
- `kdb check` should validate root-relative paths the same as relative paths
- LSP autocomplete should support root-relative paths
- Hover/go-to-definition should resolve correctly

## Open Questions

- Syntax: `/path` or `//path`? Single `/` is more natural but could conflict with filesystem absolute paths in edge cases.
- Should this apply to all link types (markdown links, wiki links, code file links)?
