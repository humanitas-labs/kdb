---
domain: "lsp"
repo: "digimata/kdb"
root: "src/lsp"
owner: "kdb"
updated: 2026.06.24
commit: "8c33d68"
confidence: high
---

# LSP — Code Map

> A map, not a manual. It points to where things live and how a request flows;
> it does not duplicate per-function detail (that's what the code and `kdb outline` are for).

## 1. What this is

A [tower-lsp](https://docs.rs/tower-lsp) language server that gives editors IDE features over a kdb markdown vault: heading outlines, go-to-definition and hover for links, link-broken diagnostics, link autocomplete, and code-index formatting. It is the IDE front-end to `crate::index::VaultIndex`; it owns LSP transport, an in-memory open-document cache, and a cached vault index — but not parsing, link resolution, or fmt logic, which it delegates to `crate::index` and `crate::fmt`.

## 2. Shape — diagram

```
 editor (stdin/stdout JSON-RPC)
   │
   ▼
┌──────────────────────────────────────────────┐      ┌────────────────────────┐
│ backend.rs  Backend : LanguageServer          │      │ shared state           │
│  serve() L75 → tower_lsp Server               │◀────▶│ documents: open text   │
│  initialize/initialized/did_* L359-446        │      │ index: VaultIndex cache│
│  dispatches each request to a feature module  │      └────────────────────────┘
└───┬───────┬────────┬────────┬────────┬────────┘
    │       │        │        │        │
    ▼       ▼        ▼        ▼        ▼
 symbols  definition hover  completion diagnostics   formatting
  L36      L58       L51     L39        L32 (events)  L21
    │       │        │        │        │              │
    └───────┴────────┴────────┴────────┴──────────────┘
                         │ all read through
                         ▼
            crate::index  (parse_markdown, resolve_target_path,
            VaultIndex, slug_anchor)   +   crate::fmt::format_source
```

## 3. Entry points

`serve()` is the only public export (`mod.rs:20`). It is invoked from the CLI dispatcher at `src/main.rs:741` (`Command::Lsp`). All other entry points are LSP protocol methods on `Backend`'s `LanguageServer` impl:

| Trigger | Handler (`file:line`) | Purpose |
|---|---|---|
| process start (`kdb lsp`) | `backend.rs:75` | discover vault root, enter tower-lsp loop |
| `initialize` | `backend.rs:359` | advertise capabilities, detect watched-files support |
| `initialized` | `backend.rs:399` | register `**/*.md` watcher, build index |
| `textDocument/didOpen` | `backend.rs:414` | cache text, sync into index, publish diagnostics |
| `textDocument/didChange` | `backend.rs:425` | re-cache, re-sync, re-diagnose |
| `textDocument/didClose` | `backend.rs:435` | drop cached text, reload from disk, clear diagnostics |
| `workspace/didChangeWatchedFiles` | `backend.rs:442` | apply external file changes to index |
| `textDocument/documentSymbol` | `symbols.rs:36` | heading outline tree |
| `textDocument/definition` | `definition.rs:58` | jump to link target / index-row line |
| `textDocument/completion` | `completion.rs:39` | file + heading link completions |
| `textDocument/hover` | `hover.rs:51` | section preview for links / index rows |
| `textDocument/formatting` | `formatting.rs:21` | regenerate code-index header block |

## 4. Lifecycle — the trace

Go-to-definition on a markdown link (the representative read path):

1. **Request arrives** — `backend.rs:455` `goto_definition` forwards to `definition::goto_definition`.
2. **Index-row shortcut** — `definition.rs:65-75` first tries `index_line_jump`: if the cursor sits on an outline/nav row ending in `LNN`, jump to that line in the same file (no index needed).
3. **Resolve URI to paths** — `definition.rs:77` `markdown_rel_path` maps the URI to (abs, vault-relative) paths, rejecting non-markdown / out-of-root URIs.
4. **Get buffer text** — `definition.rs:81` `document_text` prefers in-memory open text over disk (`backend.rs:210`).
5. **Find link under cursor** — `definition.rs:85` `link_under_position` scans markdown + wikilink regexes for the span containing the cursor byte offset (`position_to_byte_offset`, `backend.rs:528`, does UTF-16→byte mapping).
6. **Resolve target path** — `definition.rs:89` `crate::index::resolve_target_path` turns the link target into a vault-relative path.
7. **Look up in index** — `definition.rs:97-117` reads the cached `VaultIndex` via `with_index`; if the link has an anchor, finds the matching heading (`slug_anchor`) and computes its 0-based line/column.
8. **Return Location** — `definition.rs:126-136` converts the target file's abs path to a `Url` and returns a scalar `Location`.

## 5. File index

**`src/lsp/`**
| File | Role |
|---|---|
| `mod.rs` | Module wiring; re-exports `serve` (`mod.rs:20`). |
| `backend.rs` | `Backend` state + `LanguageServer` impl; index caching, open-doc cache, file watcher, path/URI helpers, `position_to_byte_offset`. |
| `symbols.rs` | `documentSymbol` — flat headings → nested `DocumentSymbol` tree via a level stack. |
| `definition.rs` | `definition` — link-under-cursor resolution + index-row `Lnn` line jumps; frontmatter detection; unit tests. |
| `completion.rs` | `completion` — detect link context (`[[`, `](`, `#`, `kdb://`) and emit file / heading completions. |
| `hover.rs` | `hover` — section-preview markup for links and outline index rows; rewrites preview links to absolute file URLs. |
| `diagnostics.rs` | Broken-link diagnostics published on open/change; cleared on close. |
| `formatting.rs` | `formatting` — calls `crate::fmt::format_source`, returns a whole-document `TextEdit`. |
| `semantic_tokens.rs` | Stub: `is_index_header_line` only; `#[allow(dead_code)]`, awaiting iss-0017. |

## 6. Key types & contracts

| Type | `file:line` | What it carries |
|---|---|---|
| `Backend` | `backend.rs:97` | client handle, vault `root`, `documents` (open text), cached `index`, watched-files flag |
| `VaultIndex` | `crate::index` (used `backend.rs:108`) | the resolved vault: `files` map (rel-path → entry with `abs_path`, `headings`) |
| `CompletionContext` | `completion.rs:94` | enum `File` / `Heading` describing what the cursor calls for (kind, prefix, `root_relative`, edit range) |
| `LinkKind` / `LinkTarget` | `crate::index` (e.g. `definition.rs:38`) | markdown vs wikilink; resolved `{file, anchor, root_relative}` |
| `SymbolNode` | `symbols.rs:28` | intermediate node (level + symbol + child indices) for building the heading tree |
| `INDEX_LINE_RE` | `definition.rs:34` | shared regex matching trailing `Lnn` on nav/index rows (reused by hover) |

## 7. Extension points

| To do this… | Touch | Notes |
|---|---|---|
| Add a new LSP request | the matching feature module + dispatch stub in `backend.rs:448-475` | also advertise it in `ServerCapabilities` at `backend.rs:369-394` |
| Support a new link syntax | `completion.rs:116` (`completion_context`), `definition.rs:143` (`link_under_position`), `hover.rs` regexes | resolution itself lives in `crate::index` (`parse_*_target`, `resolve_target_path`) |
| Change broken-link rules | `diagnostics.rs:117` `link_error_reason` | severity/source set in `collect_link_diagnostics` `diagnostics.rs:95` |
| Add a completion trigger char | `backend.rs:383-389` `trigger_characters` | parsing branch in `completion_context` |
| Format a new file type | `crate::fmt::format_source`; gate in `formatting.rs:26-31` | LSP side only decides eligibility (`code_rel_path` / `markdown_rel_path`) |
| Flesh out semantic tokens | `semantic_tokens.rs` + new capability in `backend.rs:369` | currently a stub (iss-0017) |

## 8. Invariants & gotchas

- **In-memory text wins over disk.** `document_text` (`backend.rs:210`) returns cached open-buffer text before falling back to `fs::read_to_string`. Every feature must read through it, or it will see stale saved content for dirty buffers.
- **Index is lazily built and cached once.** `ensure_index_loaded` (`backend.rs:137`) builds via `spawn_blocking` and only writes if still `None` (double-checked). Keep mutations going through `VaultIndex::upsert_file` / `reload_file` / `remove_file` (defined in `crate::index`, `src/index/mod.rs:565`/`588`/`617`; called from `backend.rs:251`/`266`/`338`/`345`/`347`); never rebuild on every request.
- **didClose re-syncs from disk.** Closing a file calls `sync_document_from_disk` (`backend.rs:437`) so the index reflects the saved (not the discarded in-memory) version.
- **LSP positions are UTF-16; Rust strings are UTF-8.** Always cross the boundary via `position_to_byte_offset` (`backend.rs:528`). Heading/link line/column from `crate::index` are 1-based and must be `saturating_sub(1)` to LSP's 0-based (see `definition.rs:108`, `symbols.rs:74`).
- **Index-row jump/hover runs for any file type, links only for markdown.** `goto_definition`/`hover` try `markdown_rel_path().or_else(code_rel_path)` for the `Lnn` shortcut (`definition.rs:66`, `hover.rs:56`), but link resolution bails unless the file is markdown.
- **Watched-files registration is conditional.** `register_markdown_watcher` (`backend.rs:270`) is a no-op unless the client advertised `dynamic_registration` during `initialize` (`backend.rs:360-367`).
- **Diagnostics treat self-links as valid.** `link_error_reason` (`diagnostics.rs:127`) short-circuits `target_file == source_file`; anchors are then checked against the freshly `parse_markdown`'d buffer, not the index.

## 9. Dependencies & boundaries

- **Calls out to:** `tower-lsp` (transport, `lsp_types`), `tokio` (RwLock, spawn_blocking), `regex`, `crate::index` (`VaultIndex`, `parse_markdown`, `resolve_target_path`, `slug_anchor`, `section_byte_bounds`, link parsing), `crate::fmt::format_source`, `crate::lang::CodeLanguage`, `crate::workspace` (`root::find_root`, `config::load_index_ignores`, `paths::normalize_rel_path`).
- **Called in by:** `src/main.rs:741` (`Command::Lsp { path }`) — the only caller; re-exported through `crate::lsp` (`src/lib.rs:34`).
- **Owns / does not own:** Owns LSP transport, the open-document text cache, the cached `VaultIndex` instance, and diagnostic publishing. Does **not** own markdown/link parsing or resolution semantics (in `crate::index`), code-index formatting (in `crate::fmt`), or vault discovery (in `crate::workspace`).

## 10. Open questions / staleness

- [ ] `mod.rs` doc comment (lines 8-9) says diagnostics and hover are "not yet implemented" — both are implemented (`diagnostics.rs`, `hover.rs`). The doc comment is stale, not the code.
- [ ] `semantic_tokens.rs` is a `#[allow(dead_code)]` stub pending iss-0017; `is_index_header_line` has no live caller in this subtree.
- [ ] Line numbers for `crate::index` symbols (e.g. `resolve_target_path`, `VaultIndex`) are not anchored here — they live outside this domain and may drift; verify against the index crate if editing resolution.
- [ ] `formatting` advertises both code and markdown eligibility (`formatting.rs:26-28`), but whether `fmt::format_source` actually transforms markdown was not traced into the fmt crate.
