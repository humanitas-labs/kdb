---
domain: "fmt"
repo: "kdb"
root: "src/fmt"
owner: "kdb"
updated: 2026.06.24
commit: "8c33d68"
confidence: high
---

# fmt — Code Map

> A map, not a manual. It points to where things live and how a request flows;
> it does not duplicate per-function detail (that's what the code and `kdb outline` are for).

## 1. What this is

The `kdb fmt` engine: it generates and maintains the navigation index headers that
`kdb` writes into source files. For **code files** (Rust/TS/JS/TSX/Python/Go) it
inserts a managed comment block listing each symbol and its line; for **markdown
files** it writes `path:` + `outline:` keys into YAML frontmatter. In scope:
discovery of files in scope, preamble detection (where the block goes), block
generation, idempotent strip-and-rewrite, and warning reporting. Out of scope:
symbol/heading extraction (delegated to `crate::symbols` / `crate::index`), file
discovery walk (`crate::workspace::discover`), and CLI argument wiring (`crate::cmd`).

## 2. Shape — diagram

```
 caller                                     this domain (src/fmt)
 ┌───────────────┐
 │ cmd::format   │  format_path / format_workspace
 │ lsp::format_  │  format_source (single string)
 │   source      │
 └──────┬────────┘
        │
        ▼
 ┌──────────────────────┐   discover_{code,markdown}_files_in_scope
 │ discover files in     │──▶ crate::workspace::discover / ignore
 │ scope (mod.rs:776/762)│
 └──────┬───────────────┘
        │  per file, branch on extension
        ├───────────────────────────────┐
        ▼ code                           ▼ markdown
 ┌────────────────────┐          ┌──────────────────────┐
 │ rewrite_code_index │          │ rewrite_markdown_nav │
 │   mod.rs:209       │          │   mod.rs:548         │
 └─────┬──────────────┘          └─────┬────────────────┘
       │                               │
       │ preamble_end_index            │ markdown_preamble_end
       ▼ (preamble.rs:44)              ▼ (preamble.rs:405)
 ┌────────────────────┐          ┌──────────────────────┐
 │ find_managed_block │          │ strip nav keys +      │
 │ + render_block     │          │ render_nav_frontmatter│
 │ (mod.rs:339/457)   │          │ (mod.rs:708/662)     │
 └─────┬──────────────┘          └─────┬────────────────┘
       │ extract_symbols               │ parse_markdown
       ▼ (crate::symbols)              ▼ (crate::index)
 ┌──────────────────────────────────────────────────────┐
 │ RewriteResult { content, removed_blocks, … }          │
 │ → fs::write if changed; collect FormatWarning         │
 └──────────────────────────────────────────────────────┘
```

## 3. Entry points

| Trigger | Handler (`file:line`) | Purpose |
|---|---|---|
| `kdb fmt [PATH]` (via `cmd::format` → `fmt::format_path`) | `src/fmt/mod.rs:108` | Rewrite a single file or subtree scope. |
| Whole-workspace rewrite | `src/fmt/mod.rs:84` | `format_workspace` — same as `format_path` but scoped to root. |
| LSP format-on-save (`lsp::formatting` → `fmt::format_source`) | `src/fmt/mod.rs:144` | Rewrite one in-memory source string; always forces markdown frontmatter. |

## 4. Lifecycle — the trace

A `kdb fmt src/foo.rs` invocation on a Rust file:

1. **CLI dispatch** — `src/cmd.rs:536` — `fmt::format_path` is called with root, target, ignore patterns, `force`.
2. **Canonicalize + scope check** — `src/fmt/mod.rs:114` — root and target are canonicalized; target must be inside root (`bail!` otherwise).
3. **Discover files** — `src/fmt/mod.rs:130` — `discover_code_files_in_scope` filters discovered paths to those with a known `CodeLanguage`.
4. **Per-file rewrite** — `src/fmt/mod.rs:181` — `format_files` reads each file and calls `rewrite_code_index`.
5. **Strip old blocks** — `src/fmt/mod.rs:237` — loop: `find_managed_block` locates an existing managed comment block within the preamble search window; it (plus leading separator comment lines) is drained. Repeats until none remain, counting removed blocks / non-canonical rows.
6. **Find insertion point** — `src/fmt/mod.rs:261` — `preamble_end_index` (→ `preamble.rs:44`, `rust_preamble_end`) returns the line after attrs/`use`/`mod`/comments.
7. **Extract + sort symbols** — `src/fmt/mod.rs:268` — `extract_symbols` (from `crate::symbols`), sorted by line then name; line numbers are then shifted to account for the block about to be inserted.
8. **Render block** — `src/fmt/mod.rs:304` — `render_block` builds aligned comment rows (`prefix header`, separator rules, `name … Lnnn`).
9. **Write if changed** — `src/fmt/mod.rs:197` — only writes when output differs from source; bumps `updated_files`; appends any `FormatWarning` from `removal_warning_message`.
10. **Report** — `src/cmd.rs:545` — prints `updated N of M files` and any warnings.

(Markdown files follow the parallel path via `format_markdown_files` → `rewrite_markdown_nav`.)

## 5. File index

**`src/fmt/`**
| File | Role |
|---|---|
| `mod.rs` | Orchestration: discovery, code-index rewrite, markdown-nav rewrite, block rendering, warnings, public API (`format_workspace`/`format_path`/`format_source`). |
| `preamble.rs` | Per-language preamble detection — where the managed block is inserted; comment prefixes; markdown frontmatter boundary. |

## 6. Key types & contracts

| Type | `file:line` | What it carries |
|---|---|---|
| `FormatReport` | `src/fmt/mod.rs:58` | `scanned_files`, `updated_files`, `warnings` — the run summary returned to callers. |
| `FormatWarning` | `src/fmt/mod.rs:66` | `rel_path` + `message` for non-fatal issues (e.g. removed non-standard rows, skipped foreign frontmatter). |
| `RewriteResult` | `src/fmt/mod.rs:72` | `content` (rewritten file) + `removed_blocks` / `removed_noncanonical_rows` counters; shared by code and markdown paths. |
| `MarkdownNavResult` | `src/fmt/mod.rs:535` | `Rewritten(RewriteResult)` vs `ForeignFrontmatter` — drives the skip-unless-`force` behaviour for markdown. |
| `CodeLanguage` | `crate::lang` (used `mod.rs:152`, `preamble.rs:3`) | Extension → language; gates which files are code-formatted and selects comment prefix / preamble logic. |

## 7. Extension points

| To do this… | Touch | Notes |
|---|---|---|
| Support a new code language | `src/fmt/preamble.rs:31` (`comment_prefix`), `:44` (`preamble_end_index`) + a new `*_preamble_end` fn | Also requires the variant in `crate::lang::CodeLanguage` and symbol extraction in `crate::symbols`. |
| Change the rendered block layout | `src/fmt/mod.rs:457` (`render_block`), `:662` (`render_nav_frontmatter`) | `LINE_GAP` (`mod.rs:54`) controls column spacing. |
| Adjust what counts as a "managed" block to strip | `src/fmt/mod.rs:370` (`is_header_candidate`), `:403` (`is_index_body_line`), `:415` (`is_canonical_index_body_line`) | `LEGACY_INDEX_HEADER` (`mod.rs:53`) keeps old `## Index` blocks recognizable. |
| Change which markdown frontmatter keys are managed | `src/fmt/mod.rs:708` (`strip_nav_keys`), `:731` (`has_foreign_key`) | Both hardcode `path:` / `outline:`; keep them in sync. |
| Add a new entry point | `src/fmt/mod.rs:84`/`:108`/`:144` | Reuse `rewrite_code_index` / `rewrite_markdown_nav`. |

## 8. Invariants & gotchas

- **Idempotency via strip-then-regenerate** — every rewrite first removes all existing managed blocks (`mod.rs:237` loop) before inserting a fresh one, so running `fmt` repeatedly is a no-op after the first pass. Writes only happen when `formatted != source` (`mod.rs:197`).
- **Line numbers in the block account for the block itself** — symbols/headings below the insertion point are shifted by the inserted line count (`mod.rs:281`, markdown `mod.rs:635`). The empty-symbols case inserts 3 lines, otherwise `symbols.len() + 5` (`mod.rs:276`). Get this wrong and every `Lnnn` is off.
- **Preamble search window is clamped** — block detection scans `preamble_end_index(...).max(256).min(len)` lines (`mod.rs:238`, `mod.rs:572`); a stray comment block beyond that window won't be found/stripped.
- **Foreign frontmatter is protected** — markdown files whose frontmatter has any key other than `path:`/`outline:` are skipped with a warning unless `--force` (`mod.rs:592`). `format_source` (LSP) always forces (`mod.rs:146`).
- **Newline + trailing-newline preservation** — both rewriters detect `\r\n` vs `\n` and whether the file ended with a newline, and restore it (`mod.rs:214`, `:308`, `:549`, `:650`). Don't bypass this when adding output.
- **`removal_warning_message` only warns on cleanup, not normal regeneration** — a single in-place block refresh produces no warning; warnings fire for non-canonical rows or >1 removed block (`mod.rs:319`).

## 9. Dependencies & boundaries

- **Calls out to:** `crate::index` (`parse_markdown`, `Heading`), `crate::lang::CodeLanguage`, `crate::symbols` (`extract_symbols`, `format_symbol_display`, `Symbol`), `crate::workspace::discover::discover_files`, `crate::workspace::ignore::build_ignore_globset`, plus `anyhow`, `globset`, `std::fs`.
- **Called in by:** `crate::cmd::format` (`src/cmd.rs:536`) for the CLI; `crate::lsp::formatting` (`src/lsp/formatting.rs:37`) for format-on-save.
- **Owns / does not own:** Owns the format/layout of the managed index block and markdown nav frontmatter, and the strip/insert rewrite logic. Does **not** own symbol extraction, markdown parsing, file-system discovery/ignore rules, or CLI/LSP plumbing — those are upstream modules it composes.

## 10. Open questions / staleness

- [ ] All `file:line` anchors verified against the files at commit `8c33d68`; the in-file index headers in `mod.rs` (L20-51) and `preamble.rs` (L6-28) are themselves fmt-generated and match.
- [ ] The exact inserted-line-count constants (3 / `len+5`) are read from `mod.rs:276-280`; not separately cross-checked against `render_block` output counting, though the two are clearly paired.
- [ ] `confidence: high` — the domain is two small, self-contained files with no async/IO branching beyond `fs::read/write`; little was inferred.
