---
domain: "index"
repo: "kdb"
root: "src/index"
owner: "kdb maintainers"
updated: 2026.06.24
commit: "8c33d68"
confidence: medium
---

# index — Code Map

> A map, not a manual. It points to where things live and how a request flows;
> it does not duplicate per-function detail (that's what the code and `kdb outline` are for).

## 1. What this is

The in-memory index at the core of kdb: it parses markdown files into headings + links, walks the whole vault to build inbound/outbound link graphs, and validates references (`kdb check`). It also indexes code: resolved per-file imports (`kdb deps`) and symbol declarations + inbound references (`kdb refs -s`). In scope: markdown parsing/linking, the vault index, code import/symbol indexing, and reference/dependency collection. Out of scope: the tree-sitter symbol *extraction* itself (lives in `crate::symbols`), import *resolution* (`crate::resolve`), embed rendering (`crate::render`), and CLI argument wiring (`crate::cmd`).

## 2. Shape — diagram

```
 cmd.rs                          src/index
   │ build_index / build_workspace_index
   ▼
┌──────────────────┐   markdown only   ┌───────────────────────────────┐
│ VaultIndex       │◀──────────────────│ markdown.rs                    │
│ ::build_*        │   parse_markdown  │  parse_markdown (tree-sitter   │
│ mod.rs:508       │                   │  + regex wikilinks)            │
└───────┬──────────┘                   └───────────────────────────────┘
        │ populate_inbound (mod.rs:692)
        ▼
┌──────────────────┐
│ file_inbound /   │── check() ──▶ CheckReport (mod.rs:625)
│ heading_inbound  │── refs.rs:82 collect_inbound (md links)
└──────────────────┘── deps.rs:25 collect_outbound (md links)

┌──────────────────┐   vault + code    ┌───────────────────────────────┐
│ WorkspaceIndex   │──────────────────▶│ CodeIndex (mod.rs:418)         │
│ mod.rs:464       │                   │  code_imports  ─ deps.rs:51    │
└──────────────────┘                   │  symbols: SymbolIndex          │
                                       └──────────────┬────────────────┘
                                                      ▼
                          ┌───────────────────────────────────────────┐
                          │ code.rs Indexer::build (code.rs:198)        │
                          │  load → extract → scope → resolution loop   │
                          │  → seed/link usage refs                     │
                          │  uses scope.rs (ModuleScope) + scanner.rs   │
                          │  (UsageScanner, tree-sitter AST walk)       │
                          └───────────────────────────────────────────┘
                                refs.rs:349 collect_symbol_refs reads .refs
```

## 3. Entry points

These are library functions invoked by `crate::cmd` (the CLI layer), not HTTP routes.

| Trigger | Handler (`file:line`) | Purpose |
|---|---|---|
| `kdb check` | `VaultIndex::check` `src/index/mod.rs:625` | Validate links/embeds + orphans → `CheckReport` |
| `kdb refs <file>[#h]` | `refs::collect_inbound` `src/index/refs.rs:82` | Inbound markdown link refs to a file/heading |
| `kdb refs -s <file> <sym>` | `refs::collect_symbol_refs` `src/index/refs.rs:349` | Inbound code references to a symbol |
| `kdb deps <file>` (md) | `deps::collect_outbound` `src/index/deps.rs:25` | Outbound markdown link deps |
| `kdb deps <file>` (code) | `deps::collect_code_outbound` `src/index/deps.rs:51` | Outbound code import deps |
| (vault build) | `VaultIndex::build_with_ignores` `src/index/mod.rs:521` | Discover + parse all `.md`, build inbound maps |
| (workspace build) | `WorkspaceIndex::build_with_ignores` `src/index/mod.rs:471` | Vault + code index together |
| (targeted refs) | `WorkspaceIndex::build_for_target` `src/index/mod.rs:483` | Symbol index scoped to importers of one file |
| (LSP incremental) | `VaultIndex::upsert_file` / `reload_file` `src/index/mod.rs:565` / `:588` | Re-parse one file from buffer/disk |

## 4. Lifecycle — the trace

`kdb check` on a vault:

1. **Build** — `src/index/mod.rs:521` — `build_with_ignores` canonicalizes root, compiles ignore globs.
2. **Discover** — `src/index/mod.rs:800` — `discover_markdown_files` walks the tree, keeps `.md`.
3. **Parse (parallel)** — `src/index/mod.rs:528` — `par_iter` reads each file and calls `parse_markdown` (`src/index/markdown.rs:51`), producing `FileEntry { headings, links }`.
4. **Inbound graph** — `src/index/mod.rs:692` — `populate_inbound` resolves each link's target file (`resolve_target_path` `src/index/mod.rs:831`), filling `file_inbound` and `heading_inbound`.
5. **Check links** — `src/index/mod.rs:631` — for each link, `resolve_link` (`mod.rs:745`) confirms the target file and (if anchored) the heading exist; failures become `BrokenLink`.
6. **Check embeds** — `src/index/mod.rs:645` — re-reads source, finds `![[…]]` via `crate::render::include::find_embeds`, validates via `crate::render::resolve::validate_embed_target`.
7. **Orphans** — `src/index/mod.rs:670` — any file with zero inbound links *from other files* is flagged.
8. **Report** — `src/index/mod.rs:327` — `CheckReport::print` formats results to stdout.

## 5. File index

**`src/index/`**
| File | Role |
|---|---|
| `mod.rs` | Data types (`VaultIndex`, `CodeIndex`, `WorkspaceIndex`, `FileEntry`, `Link`, `Heading`, `LinkRef`, `CheckReport`, …), vault build, `check`, inbound population, link resolution. |
| `markdown.rs` | Single-file markdown parsing: headings, markdown links, wikilinks (tree-sitter-md + regex), anchor slugging, `kdb://` and target parsing, section bounds. |
| `deps.rs` | Outbound dependency collection (markdown links + resolved code imports) and text printing. |
| `refs.rs` | Inbound reference collection — markdown links and code symbols — plus target parsing, selectors, and context-line text rendering. |
| `code.rs` | `SymbolIndex` + `Indexer`: load code files, extract symbols, build module scopes, run reexport/glob resolution loop, seed + link usage references. |
| `scanner.rs` | `UsageScanner` — tree-sitter AST walk that finds identifier usages matching imported names (Rust/Go/TS qualified + member access). |
| `scope.rs` | `ModuleScope` and helper structs (`ExportedNames`, `GlobSource`, `ReexportTarget`, `FollowedReexport`) describing per-file visible names built from imports. |

## 6. Key types & contracts

| Type | `file:line` | What it carries |
|---|---|---|
| `VaultIndex` | `src/index/mod.rs:109` | Root + `files` map + `file_inbound`/`heading_inbound` graphs. |
| `CodeIndex` | `src/index/mod.rs:127` | Workspace packages, lang caches, `code_imports`, `symbols: SymbolIndex`. |
| `WorkspaceIndex` | `src/index/mod.rs:176` | `vault` + `code` combined. |
| `FileEntry` | `src/index/mod.rs:185` | One markdown file: rel/abs path, `headings`, `links`. |
| `Link` / `LinkTarget` | `src/index/mod.rs:235` / `:223` | Parsed link: kind, raw, target (file/anchor/root_relative), pos. |
| `LinkRef` | `src/index/mod.rs:259` | A reference site (source file/line/col/raw) for inbound graphs. |
| `SymbolKey` / `SymbolRef` | `src/index/mod.rs:142` / `:157` | Definition identity (file+name+parent+kind+line) and one ref row. |
| `CheckReport` | `src/index/mod.rs:311` | `broken_links`, `broken_embeds`, `orphans`. |
| `SymbolIndex` | `src/index/code.rs:66` | `symbols` per file + `refs: HashMap<SymbolKey, Vec<SymbolRef>>`. |
| `ModuleScope` | `src/index/scope.rs:27` | Per-file bindings, aliases, namespace/glob sources for usage scanning. |

## 7. Extension points

| To do this… | Touch | Notes |
|---|---|---|
| Support a new link syntax | `markdown.rs` | Add a collector in `parse_markdown` (`markdown.rs:51`) and a target parser like `parse_markdown_target` / `parse_wikilink_target`. |
| Support a new code language for refs | `scanner.rs`, `scope.rs`, `crate::symbols`, `crate::resolve` | Add qualified-usage handling in `UsageScanner`; resolution feeds from `crate::resolve` import index. |
| Add a new check rule | `mod.rs` | Extend `VaultIndex::check` (`mod.rs:625`) and `CheckReport` (`mod.rs:311`) + `print` (`mod.rs:327`). |
| Change link-target resolution | `mod.rs:831` | `resolve_target_path` is the single chokepoint for md/wikilink/`kdb://`. |
| New `kdb deps`/`refs` output format | `deps.rs`, `refs.rs` | Collection vs. rendering are separate fns; add a renderer next to `print_text`. |

## 8. Invariants & gotchas

- **Paths are root-relative `PathBuf` keys.** `VaultIndex.files`, inbound maps, and `code_imports` all key on paths relative to a *canonicalized* root. Build canonicalizes (`mod.rs:472`, `:522`); CLI targets must go through `resolve_file_target` (`mod.rs:863`) to match.
- **Inbound is fully recomputed.** `populate_inbound` (`mod.rs:692`) rebuilds *both* graphs from scratch on every `upsert_file`/`reload_file`/`remove_file` — O(all links), not incremental. Cheap for a vault, but don't assume diffing.
- **Orphan = no inbound from *other* files.** Self-links don't count (`mod.rs:676`); a file linking only to itself is still an orphan.
- **Anchors are slugged before comparison.** Link anchors and heading anchors are both passed through `slug_anchor` (`mod.rs:718`, `refs.rs:77`); compare slugs, never raw text. Duplicate heading slugs get numeric suffixes (`assign_heading_anchors`).
- **Wikilinks auto-append `.md`; markdown links do not.** `resolve_target_path` (`mod.rs:839`) only adds `.md` for `LinkKind::Wikilink` with no extension. `parse_markdown_target` rejects non-`.md` files outright (`markdown.rs:106`).
- **Wikilinks parsed by regex, excluding code spans.** `collect_wikilinks` uses regex over raw text but skips ranges from `collect_wikilink_excluded_ranges` (`markdown.rs:62`) so `` `[[x]]` `` in inline code isn't a link.
- **Targeted symbol build ≠ full build.** `build_for_target` / `build_targeted` only extract + scan files that import the target (`code.rs:225`); `.refs` will be incomplete for unrelated symbols — use `build_with_symbol_refs` for the full index.
- **`UsageScanner.collect` requires the tree to come from the same source** passed to `new()` (`scanner.rs:71`) — byte offsets are reused for columns/snippets.

## 9. Dependencies & boundaries

- **Calls out to:** `crate::workspace` (discovery, ignore globs, `normalize_rel_path`), `crate::resolve` (import index, reexport bindings, workspace caches), `crate::symbols` (tree-sitter parse + symbol extraction), `crate::render` (embed finding/validation in `check`), `crate::lang` (`CodeLanguage`); plus `tree-sitter`, `tree-sitter-md`, `regex`, `rayon`, `globset`, `serde`.
- **Called in by:** `crate::cmd` (check/refs/deps commands, `build_index`/`build_workspace_index`), `crate::lsp` (`backend` does incremental updates via `upsert_file`/`reload_file`; `completion`/`diagnostics`/`hover`/`definition` query the `files`/`headings` maps — none of the lsp modules touch the inbound graphs), and `crate::deps` modules reference index types.
- **Owns / does not own:** Owns the in-memory vault + code index data structures and the link/ref/dep graphs derived from them. Does *not* own on-disk state (read-only over the workspace), symbol extraction internals, import resolution rules, or embed rendering.

## 10. Open questions / staleness

- [ ] `code.rs` (1199 lines, 30+ `Indexer` methods) was read only at the build-orchestration level (`code.rs:198`); the reexport/glob resolution loop and Go same-package linking are summarized from method names + outline, not line-by-line.
- [ ] The `refs.rs` text-rendering machinery (`SymbolRefTextRenderer`, `ContextWindow`, `SourceContext`, `FsLineSource`) is treated as a rendering detail; its context-window math was not deeply verified.
- [ ] Exact CLI flag → function mapping is inferred from `src/cmd.rs` grep (lines 230/363/397/441/443); `cmd.rs` is outside this domain, so trigger labels in §3 are approximate.
- [ ] `markdown.rs` lower half (`section_*_bounds`, heading underline/setext handling, `normalize_*`) skimmed via outline only.
