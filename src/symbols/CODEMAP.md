---
domain: "symbols"
repo: "kdb"
root: "src/symbols"
owner: "kdb"
updated: 2026.06.24
commit: "8c33d68"
confidence: high
---

# Symbols — Code Map

> A map, not a manual. It points to where things live and how a request flows;
> it does not duplicate per-function detail (that's what the code and `kdb outline` are for).

## 1. What this is

Language-aware code symbol extraction. Given a source file and its `CodeLanguage`,
this domain parses it with tree-sitter and produces a flat `Vec<Symbol>` of
declarations (functions, methods, types, consts, etc.) with line/byte spans,
parent qualification, visibility, and language-native display kinds. It also owns
the CLI-facing formatting/output for `kdb outline` (called `symbols` internally)
and the markdown-heading path that shares the same `SymbolRow`/`SymbolBodyRow`
output shape. Out of scope: the markdown parser itself (`crate::index`), code
indexing/`fmt` headers (`crate::fmt`, `crate::index::code`), and import/reference
resolution (`crate::resolve`) — all of which *consume* this domain.

## 2. Shape — diagram

```
 caller (cmd::symbols, fmt, index::code, resolve)
   │
   ▼  extract_symbols / extract_symbols_from_tree   mod.rs:180,189
┌──────────────┐   ┌──────────────────┐   ┌────────────────────────┐
│ tree.rs      │──▶│ mod.rs dispatch  │──▶│  extract/<lang>.rs     │
│ parse_tree   │   │  match language  │   │  rust/typescript/      │
│  :23         │   │   :197           │   │  python/go  extract()  │
└──────────────┘   └──────────────────┘   └───────────┬────────────┘
   tree-sitter                                         │ push() / push_raw()
   grammars                                            ▼
                                              ┌────────────────────┐
                                              │ Extractor          │
                                              │  dedup via `seen`  │
                                              │  mod.rs:90         │
                                              └─────────┬──────────┘
                                                        │ Vec<Symbol>
            CLI output path (query + display)           ▼
┌───────────────┐   ┌──────────────────┐   ┌────────────────────────┐
│ query.rs      │──▶│ display.rs       │──▶│ print_text /           │
│ collect_rows  │   │ SymbolRow::from  │   │ print_bodies_text      │
│  :39          │   │  format_display  │   │  :290,328              │
└───────────────┘   └──────────────────┘   └────────────────────────┘
   markdown branch → crate::index::parse_markdown (headings)
```

## 3. Entry points

| Trigger | Handler (`file:line`) | Purpose |
|---|---|---|
| `extract_symbols(lang, src)` | `mod.rs:180` | Parse + extract in one call (convenience). |
| `extract_symbols_from_tree(lang, src, tree)` | `mod.rs:189` | Extract from a pre-parsed tree (avoids reparsing). |
| `kdb outline <paths>` (internal `cmd::symbols`) | `query.rs:39` (`collect_rows`) | Per-file symbol/heading rows for CLI listing. |
| `kdb outline -s <sel>` body output | `query.rs:84` (`collect_body_rows`) | Full source bodies for matched selectors. |
| `query::expand_paths(ws, paths)` | `query.rs:301` | Expand files/dirs into `(abs, rel)` pairs. |
| Re-exports for other crates | `mod.rs:36`,`39` | `parse_tree`, `raw_node_text`, `walk_depth_first`, display helpers. |

## 4. Lifecycle — the trace

Tracing `kdb outline src/foo.rs` (no selector), the most common path:

1. **cmd dispatch** — `cmd.rs:300` — `symbols::query::expand_paths` turns the path into `(abs, rel)` pairs.
2. **per-file collect** — `cmd.rs:314` → `query.rs:39` — `collect_rows`; not markdown, so `CodeLanguage::from_path` resolves the language.
3. **read + extract** — `query.rs:69` — `extract_symbols(language, &source)`.
4. **parse** — `mod.rs:181` → `tree.rs:23` — `parse_tree` loads the grammar and parses.
5. **dispatch** — `mod.rs:197` — `match language` routes to `extract::extract_rust` (`extract/rust.rs:36`).
6. **walk + push** — `extract/rust.rs:39` — `walk_depth_first` visits each node; matched kinds call `extractor.push(...)` (`mod.rs:114`), which dedups by `SeenSymbolKey` (`mod.rs:127`).
7. **sort** — `query.rs:70` — symbols sorted by line, then name.
8. **format** — `query.rs:75` → `display.rs:62` — `SymbolRow::from` builds the display string via `format_symbol_display` (`display.rs:138`).
9. **print** — `cmd.rs:329` → `display.rs:290` — `print_text` aligns rows and writes `… L<line>`.

## 5. File index

**`src/symbols/`**
| File | Role |
|---|---|
| `mod.rs` | `SymbolKind`/`Symbol` types, `Extractor` (dedup buffer), top-level `extract_symbols*` dispatch, re-exports. |
| `tree.rs` | tree-sitter parsing + node utilities (parse, text extraction, depth-first walk, type-name normalization, Go receiver / Python decorator helpers). |
| `query.rs` | CLI query layer: collect rows/bodies per file, markdown-vs-code branching, `SymbolSelector` parsing, path expansion. |
| `display.rs` | Output types (`SymbolRow`, `SymbolBodyRow`), kind labels, display formatting, body+docs extraction, text printers. |

**`src/symbols/extract/`**
| File | Role |
|---|---|
| `mod.rs` | Per-language dispatch; re-exports `extract_{rust,typescript,python,go}` and shared helpers. |
| `rust.rs` | Rust extractor — functions, methods, items, statics, macros (incl. macro-body extraction). |
| `typescript.rs` | JS/TS/TSX extractor — classes, members, functions, variables, export/visibility detection. |
| `python.rs` | Python extractor — functions, methods, classes, decorator handling. |
| `go.rs` | Go extractor — functions, methods (receiver typing), types, consts. |

## 6. Key types & contracts

| Type | `file:line` | What it carries |
|---|---|---|
| `Symbol` | `mod.rs:65` | The core record: name, parent, kind, display_kind, line/end_line, start/end byte spans, is_public. |
| `SymbolKind` | `mod.rs:43` | Enum of declaration categories (Function, Method, Struct, …, Variable). |
| `Extractor<'src>` | `mod.rs:90` | Per-extraction buffer holding source bytes, the symbol vec, and a `seen` dedup set. |
| `SeenSymbolKey` | `mod.rs:78` | Dedup key (line/end_line/start_byte/name/parent/kind/display_kind/is_public). |
| `SymbolRow` | `display.rs:34` | Listing row (display string + serializable fields) for `kdb outline`. |
| `SymbolBodyRow` | `display.rs:81` | Body row (full source snippet + spans) for `-s` selector output. |
| `SymbolSelector` | `query.rs:234` | Parsed `Parent::name` / `Parent.name` / `name` selector with `matches`. |
| `CodeLanguage` | `crate::lang` (external) | Drives grammar choice + `from_path` language detection. |

## 7. Extension points

| To do this… | Touch | Notes |
|---|---|---|
| Add a new language | `extract/<lang>.rs` (new), `extract/mod.rs:3-6,27-30`, `tree.rs:36`, `mod.rs:197` | Also add the variant to `crate::lang::CodeLanguage` + the tree-sitter grammar dep in `Cargo.toml`. |
| Add a new `SymbolKind` | `mod.rs:43`, `display.rs:105` (`kind_label`), `display.rs:126` (`is_callable_kind` if callable) | All `match`es over `SymbolKind` are exhaustive — the compiler flags omissions. |
| Change listing output format | `display.rs:138` (`format_symbol_display`), `display.rs:290`/`328` (printers) | JSON shape is governed by `SymbolRow`/`SymbolBodyRow` serde attrs. |
| Adjust body+doc capture | `display.rs:183` (`extract_body_with_docs`), `:236`/`:250` (comment/attr detection) | Doc detection is line-prefix heuristic, language-agnostic. |
| Change selector syntax | `query.rs:241` (`SymbolSelector::parse`), `query.rs:292` (`normalize_selector_name`) | `::` tried before `.`; trailing `()` stripped. |

## 8. Invariants & gotchas

- **Dedup is mandatory** — all symbols go through `Extractor::push`/`push_raw` (`mod.rs:114,154`) which dedups by `SeenSymbolKey`. Bypassing it (constructing `Symbol`s and pushing to the vec directly) would let duplicates through, e.g. when macro re-parsing in `rust.rs` re-discovers the same node.
- **`tree` must match `source`** — `extract_symbols_from_tree` (`mod.rs:189`) trusts that the caller parsed `tree` from the same `source`; byte spans index back into `source`, so a mismatch yields garbage bodies.
- **Lines are 1-based, bytes are 0-based** — `Symbol.line`/`end_line` are `row + 1` (`mod.rs:123`); `start_byte`/`end_byte` are raw tree-sitter offsets. `extract_body_with_docs` mixes both and uses `saturating_sub(1)`.
- **Markdown is a separate branch** — `collect_rows`/`collect_body_rows` (`query.rs:39,84`) detect `.md` first and route through `crate::index::parse_markdown` (headings), *not* tree-sitter. Markdown files never reach `extract_symbols`.
- **Markdown parsed in isolation** — `collect_rows` reads only the target file, never the whole vault (perf fix, iss-0064; comment at `query.rs:34-38`).
- **`-s` requires a single file** — selector body output bails if multiple files match (`cmd.rs:304`); `collect_body_rows` asserts non-empty selectors (`query.rs:90`).
- **Constructor special-case** — `format_symbol_display` (`display.rs:142`) prints bare `constructor()` for TS constructors instead of `constructor constructor()`.

## 9. Dependencies & boundaries

- **Calls out to:** `tree_sitter` + the four grammar crates (`tree_sitter_rust`, `_typescript`, `_python`, `_go`); `crate::lang::CodeLanguage`; `crate::index` (`parse_markdown`, `section_byte_bounds`, `section_line_bounds`, `Heading`) for the markdown branch; `crate::workspace` (`discover`, `paths`, `root`) for path expansion; `anyhow`, `serde`.
- **Called in by:** `crate::cmd::symbols` (CLI); `crate::fmt` (index headers); `crate::index::code` + `crate::index::scanner`; `crate::resolve::{rust,python,tsjs,mod}` (uses `walk_depth_first`, `raw_node_text`, `extract_symbols`). Note: `crate::lsp` does *not* consume this domain — its `document_symbol` provider (`lsp/backend.rs:452` → `lsp/symbols.rs`) builds a markdown-heading tree from `crate::index::parse_markdown` only; the shared name `symbols` is coincidental.
- **Owns / does not own:** Owns symbol extraction, the `Symbol`/`SymbolKind` model, and CLI symbol formatting. Does **not** own markdown parsing, the code index/DB, or reference resolution — it exposes tree-sitter helpers those domains build on.

## 10. Open questions / staleness

- [ ] Per-language extractor internals (`rust.rs`, `typescript.rs`, `python.rs`, `go.rs` bodies) were not read line-by-line beyond their headers/signatures — function lists come from the `kdb fmt` navigation headers, which can drift if not refreshed.
- [ ] `kdb deps src/symbols/query.rs` returned empty in this session; the dependency list in §9 was assembled from `use` statements and grep rather than the `deps` tool.
- [ ] The CLI command is surfaced to users as `kdb outline` but the internal fn is `cmd::symbols`; confirm there is no separate `kdb symbols` alias still wired (CLAUDE.md references `kdb outline`).
