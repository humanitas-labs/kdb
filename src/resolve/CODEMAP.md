---
domain: "resolve"
repo: "kdb"
root: "src/resolve"
owner: "kdb"
updated: 2026.06.24
commit: "8c33d68"
confidence: medium
---

# resolve — Code Map

> A map, not a manual. It points to where things live and how a request flows;
> it does not duplicate per-function detail (that's what the code and `kdb outline` are for).

## 1. What this is

Workspace-aware import resolution for code files. Given a project root and a source file, this domain parses each file's import/use statements and resolves every specifier to a concrete in-repo file path (plus its kind, the names it binds, and its line number). It understands four languages — TypeScript/JavaScript, Rust, Go, Python — and their workspace layouts (pnpm/npm workspaces + tsconfig paths, Cargo workspaces, `go.work`, Python project roots). Out of scope: symbol extraction (`crate::symbols`), the usage/reference graph and ranking (`crate::index`), and rendering output (`crate::render`).

## 2. Shape — diagram

```
 caller (crate::index::CodeIndex::build)
   │ build_workspace_import_index(root, ignore)            mod.rs:194
   ▼
┌────────────────────┐   ┌──────────────────────────────┐
│ WorkspaceCaches    │   │ discover_code_files()        │
│ ::build()          │   │ (walkdir + ignore globset)   │
│ mod.rs:83          │   │ mod.rs:294                   │
└─────────┬──────────┘   └───────────────┬──────────────┘
          │ per-language caches          │ rel paths
          └──────────────┬───────────────┘
                         ▼ rayon par_iter, per file
              ┌─────────────────────────────────────┐
              │ resolve_imports_for_language()       │
              │ mod.rs:240  (static dispatch on lang)│
              └───┬────────┬────────┬────────┬───────┘
                  ▼        ▼        ▼        ▼
            ┌────────┐┌────────┐┌──────┐┌────────┐
            │ tsjs   ││ rust   ││ go   ││ python │  LanguageResolver
            │Resolver││Resolver││Resol ││Resolver│  (mod.rs:156)
            └────────┘└────────┘└──────┘└────────┘
                  │ each emits Vec<ResolvedImport>
                  ▼
          BTreeMap<PathBuf, Vec<ResolvedImport>>  → WorkspaceImportResult
```

Shared helpers (`resolve_file`, `resolve_with_exts`, `normalize_identifier`, `exported_symbol_names`) live in `mod.rs` and are used by every language resolver.

## 3. Entry points

The domain has no CLI surface of its own; callers are other crate modules.

| Trigger | Handler (`file:line`) | Purpose |
|---|---|---|
| `build_workspace_import_index(root, ignore)` | `mod.rs:194` | Scan every code file under root, resolve all imports. Main entry; called by `CodeIndex::build*`. |
| `resolve_imports_for_language(root, file, src, lang, caches)` | `mod.rs:240` | Resolve one file's imports; static dispatch to per-language resolver. |
| `WorkspaceCaches::build(root, ignore)` | `mod.rs:83` | Pre-build all four language workspace caches. |
| `extract_reexport_bindings(file, src, lang, tree)` | `mod.rs:166` | One-hop re-export bindings; called by `crate::index::code`. |

`exported_symbol_names` (`mod.rs:271`) is `pub(super)` — an internal helper used by wildcard import expansion within the resolve module, not a cross-module entry point.

## 4. Lifecycle — the trace

Resolving a single Rust `use crate::foo::Bar;` during a full index build:

1. **`build_workspace_import_index`** — `mod.rs:194` — builds `WorkspaceCaches`, discovers code files, then `par_iter` over them.
2. **`discover_code_files`** — `mod.rs:294` — walkdir under root, skips ignored dirs/files, keeps paths whose extension maps to a `CodeLanguage`.
3. **`resolve_imports_for_language`** — `mod.rs:240` — sees `CodeLanguage::Rust`, constructs `RustResolver::new(root, &caches.rust_workspace)`.
4. **`RustResolver::resolve_source`** — `rust.rs:453` — builds a `CrateContext` for the file, then walks `mod`/`use` items via `collect_source_imports`.
5. **`CrateContext::from_workspace`** — `rust.rs:545` — finds the enclosing crate (`find_crate_root`) and pulls its `src_root` / dependency maps from the cache.
6. **`CrateContext::resolve_use`** — `rust.rs:604` — splits on `::`, handles `crate`/`self`/`super`/external-crate prefixes, walks to a candidate path.
7. **`resolve_file`** — `mod.rs:423` — confirms the path exists on disk and case-corrects it via `canonicalize_existing_rel_path` (`mod.rs:432`).
8. **emit `ResolvedImport`** — `rust.rs:509` — raw specifier, resolved path, `ImportKind`, bound names (`imported_names`), line. For wildcard `use ... *`, names come from `exported_symbol_names` (`mod.rs:271`).
9. **collect** — `mod.rs:225` — imports sorted by (line, raw, path), inserted into `file_imports` BTreeMap, returned in `WorkspaceImportResult`.

## 5. File index

**`src/resolve/`**
| File | Role |
|---|---|
| `mod.rs` | Public API, shared types (`ImportKind`, `ImportNames`, `ResolvedImport`, `ReexportBinding`, `LanguageResolver`), the scan driver, and path/identifier helpers. |
| `tsjs.rs` | TypeScript/JavaScript resolver — uses `unrs_resolver` with per-package tsconfig, pnpm/npm workspace discovery; specifiers parsed via tree-sitter (`tree-sitter-javascript`/`tree-sitter-typescript`). |
| `rust.rs` | Rust resolver — Cargo manifest + workspace parsing, `mod`/`use` resolution via tree-sitter, crate-context path walking. |
| `go.rs` | Go resolver — `go.work` / `go.mod` parsing, line-based import extraction, module-path matching to local dirs. |
| `python.rs` | Python resolver — project-root discovery (pyproject/setup.py/poetry/hatch), `import`/`from` resolution, `__all__` re-export extraction. |

## 6. Key types & contracts

| Type | `file:line` | What it carries |
|---|---|---|
| `ResolvedImport` | `mod.rs:131` | The output unit: `raw` specifier, `resolved_path: Option<PathBuf>`, `kind`, `names`, `line`. |
| `ImportKind` | `mod.rs:96` | `Relative` / `Workspace` / `TsconfigPath` / `External`. |
| `ImportNames` | `mod.rs:109` | `locals` (in-scope names), `aliases` (alias→def for renames), `is_namespace` (brings all exports). |
| `ReexportBinding` | `mod.rs:144` | One re-exported symbol: raw specifier, exported name, definition name, line. |
| `LanguageResolver` | `mod.rs:156` | Trait contract: `resolve(&self, file, src) -> Vec<ResolvedImport>`; static dispatch, no `Box<dyn>`. |
| `WorkspaceCaches` | `mod.rs:74` | Bundles the four per-language workspace caches built once per scan. |
| `WorkspaceImportResult` | `mod.rs:186` | Scan output: workspace packages, go/rust workspace caches, `file_imports` map. |
| `RustWorkspaceCache` / `RustWorkspaceCrate` | `rust.rs:362` / `rust.rs:352` | Per-crate src roots, entry files, dependency maps, keyed by crate root. |
| `GoWorkspaceCache` | `go.rs:141` | `modules_by_path`: module name → local dir. |
| `PythonWorkspaceCache` | `python.rs:75` | Discovered Python project package roots. |

## 7. Extension points

| To do this… | Touch | Notes |
|---|---|---|
| Add a new language | new `xx.rs` + `mod.rs` | Implement `LanguageResolver`; add `mod`/`pub(crate) use` in `mod.rs:52-67`; add arm in `resolve_imports_for_language` (`mod.rs:240`) and (if it has a workspace) in `WorkspaceCaches::build` (`mod.rs:83`). Must also be a `CodeLanguage` (`crate::lang`). |
| Add re-export support for a language | the lang's `collect_reexports` + `mod.rs:166` | Wire the new arm into `extract_reexport_bindings`; Go currently returns empty (`mod.rs:178`). |
| Change how files are discovered | `discover_code_files` `mod.rs:294` | Ignore handling delegates to `crate::workspace::ignore`. |
| Add a new `ImportKind` | `mod.rs:96` + each resolver's classify fn | e.g. `classify_use_kind` (rust.rs:984), `classify_local_kind` (tsjs.rs:228). |
| Adjust file-extension fallback resolution | `resolve_with_exts` `mod.rs:395` | Index-file + extension probing shared by tsjs/python. |

## 8. Invariants & gotchas

- **`resolved_path` is root-relative, case-corrected, and verified to exist** — `resolve_file` (`mod.rs:423`) returns `None` if the file is absent; `canonicalize_existing_rel_path` (`mod.rs:432`) reconciles case so paths join across the index. Don't fabricate paths that bypass this.
- **Static dispatch only** — `LanguageResolver` is a trait but dispatch is a `match` in `resolve_imports_for_language`; there is no trait-object registry. Adding a language means editing the match arm, not just implementing the trait.
- **Caches are built once per scan** — `WorkspaceCaches::build` runs before the parallel file loop; resolvers borrow the cache immutably across rayon threads. Per-file resolvers (`RustResolver`, etc.) are cheap to construct and created per file.
- **Output ordering is deterministic** — imports are sorted by `(line, raw, resolved_path)` (`mod.rs:215`) and stored in a `BTreeMap`, so downstream index output is stable.
- **Go has no re-export extraction** — `extract_reexport_bindings` returns empty for Go (`mod.rs:178`).
- **Wildcard imports trigger a second file read** — Rust `use ...::*` and similar expand names via `exported_symbol_names` (`mod.rs:271`), which re-reads and re-parses the target file through `crate::symbols`.
- **tsjs picks the longest-prefix package resolver** — `resolver_for` (`tsjs.rs:191`) selects the package whose root is the longest path prefix of the source file, falling back to a root-level resolver; tsconfig `paths` resolution depends on this being right.

## 9. Dependencies & boundaries

- **Calls out to:** `crate::lang` (`CodeLanguage`), `crate::symbols` (`extract_symbols`, `walk_depth_first`), `crate::workspace::{ignore, paths}` (ignore globset, rel-path normalization); external crates `unrs_resolver` (tsjs path resolution), `tree-sitter` + `tree-sitter-javascript`/`tree-sitter-typescript` (tsjs specifier parsing) + `tree-sitter-rust` (rust/reexports) + `tree-sitter-go`/`tree-sitter-python` (go/python), `toml` (Cargo), `walkdir`, `globset`, `rayon`, `anyhow`.
- **Called in by:** `crate::index` (`index/mod.rs:421,439,452` via `CodeIndex::build*`; `index/code.rs:429` via `extract_reexport_bindings`; `index/scope.rs:4` uses `ResolvedImport`). Re-exported at `crate::resolve` (`lib.rs:38`).
- **Owns / does not own:** Owns import parsing + path resolution and the per-language workspace caches. Does NOT own symbol extraction, the usage/reference graph, ranking, or any persisted index — those belong to `crate::symbols` and `crate::index`.

## 10. Open questions / staleness

- [ ] Python and Go resolver internals were read only at their entry points (`PythonResolver::resolve` python.rs:120, `GoResolver::resolve_source` go.rs:189); the deeper module-path resolution (e.g. `absolute_module_paths`, `workspace_module_match`) was not traced line-by-line.
- [ ] tsjs `resolve()` (tsjs.rs:153) was read; the `WorkspacePatternSet` / pnpm/package.json glob discovery (tsjs.rs:762+) was not verified in detail.
- [ ] `confidence: medium` — mod.rs and the Rust hot path are verified against source; the other three resolvers are mapped at the API/entry level, so their internal claims (in §5/§6) are inferred from outlines and signatures rather than full reads.
