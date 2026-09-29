---
path: projects/kdb/docs/languages/prosaic.md
---

# Prosaic

Prosaic is a pseudocode language for writing operational procedures and SOPs. It uses indentation-based nesting and a small set of highlighted token types.

**The normative language spec lives in the digimata kernel: `kernel/prosaic.md`.** The reference implementation lives in the Prosaic project, not this repo:

- Tree-sitter grammar: `labs/projects/prosaic/tree-sitter-prosaic/`
- Zed highlighting: `labs/projects/prosaic/editors/zed/` (a separate `prosaic` dev extension)

kdb keeps procedure-reference resolution for `kdb check` (`src/index/prosaic.rs`).

Use `prosaic` as the language identifier in fenced code blocks. See the spec's §7 (Conformance) for what the grammar currently supports versus the full spec.
