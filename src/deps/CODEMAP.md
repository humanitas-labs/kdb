---
domain: "deps"
repo: "kdb"
root: "src/deps"
owner: "kdb maintainers"
updated: 2026.06.24
commit: "8c33d68"
confidence: high
---

# deps — Code Map

> A map, not a manual. It points to where things live and how a request flows;
> it does not duplicate per-function detail (that's what the code and `kdb outline` are for).

## 1. What this is

`src/deps` is a **per-file, single-shot outbound code-dependency extractor**: given one source file, it parses its imports/`use`/`mod`/`import` statements and resolves each to the workspace-relative path of the file it points at. It dispatches by language (Rust, TS/JS/TSX, Python, Go) and returns a deduplicated, sorted `Vec<Dependency>`.

Scope boundary: this module only handles **code** dependency resolution for a *single* file in isolation, reusing the tree-sitter import parsers in `crate::resolve`. It does **not** handle markdown links (that's `crate::index::deps`), does not build a whole-workspace import graph (that's `crate::resolve::build_workspace_import_index`), and does not attach symbol-level anchors (every `Dependency.anchor` is `None`).

> Note: at this commit `src/deps::collect_outbound` has **no callers in the repo** — the live `kdb deps` command path resolves through `crate::index::deps` instead. See §9/§10.

## 2. Shape — diagram

```
 caller (root, source_file)
   │
   ▼
┌──────────────────────────────┐
│ mod.rs::collect_outbound L29 │  read file + detect language
│  CodeLanguage::from_path     │
└───────────────┬──────────────┘
                │ dispatch by language
   ┌────────────┼────────────┬──────────────┐
   ▼            ▼            ▼              ▼
┌────────┐  ┌──────────┐  ┌─────────┐  ┌────────┐
│rust.rs │  │typescript│  │python.rs│  │ go.rs  │
│collect │  │.rs       │  │collect  │  │collect │
│  L24   │  │collect22 │  │  L19    │  │  L18   │
└───┬────┘  └────┬─────┘  └────┬────┘  └───┬────┘
    │            │             │           │
    │ parse via crate::resolve │ line-scan │ line-scan
    │ (rust)     │ (tsjs)      │ (regex-ish)│ (go.mod)
    ▼            ▼             ▼           ▼
        ┌────────────────────────────┐
        │ utils.rs  resolve_file L40  │  candidate path → exists?
        │  resolve_with_exts    L14   │
        │  list_go_package_files L49  │
        └──────────────┬─────────────┘
                       ▼
            BTreeSet<Dependency>  (dedup + sort)
                       │
                       ▼
              Vec<Dependency>  (file, anchor=None)
```

## 3. Entry points

The only public entry is the dispatcher; the per-language `collect` functions are `pub(super)` and reached only through it.

| Trigger | Handler (`file:line`) | Purpose |
|---|---|---|
| `deps::collect_outbound(root, source_file)` | `src/deps/mod.rs:29` | Detect language, read file, dispatch, return sorted deps |
| (dispatch) Rust file | `src/deps/rust.rs:24` | Resolve `mod`/`use` to files |
| (dispatch) JS/TS/TSX file | `src/deps/typescript.rs:22` | Resolve relative import specifiers |
| (dispatch) Python file | `src/deps/python.rs:19` | Resolve `import`/`from … import` |
| (dispatch) Go file | `src/deps/go.rs:18` | Resolve module-qualified + relative imports |

## 4. Lifecycle — the trace

Representative path: a **Rust** source file.

1. **Entry** — `src/deps/mod.rs:29` — `collect_outbound(root, source_file)` is called.
2. **Language detect** — `src/deps/mod.rs:30` — `CodeLanguage::from_path` (`src/lang.rs:26`) maps extension → `CodeLanguage`; `None` → error "deps is not supported".
3. **Read source** — `src/deps/mod.rs:37-39` — joins `root` + `source_file`, reads to string.
4. **Dispatch** — `src/deps/mod.rs:43` — `CodeLanguage::Rust` → `rust::collect`.
5. **Parse imports** — `src/deps/rust.rs:44` — delegates to `crate::resolve::collect_mod_and_use` (`src/resolve/rust.rs:207`, tree-sitter) → `(mods, uses)`.
6. **Resolve `mod` decls** — `src/deps/rust.rs:46-51` → `resolve_mod_decl` (`src/deps/rust.rs:76`) builds `<dir>/<name>.rs` or `<dir>/<name>/mod.rs`, checks existence via `utils::resolve_file` (`src/deps/utils.rs:40`).
7. **Resolve `use` paths** — `src/deps/rust.rs:53-61` → `parse_use_prefix` (`:64`) strips `{`, `,`, ` as `, trailing `::`; `resolve_use` (`:102`) walks `crate`/`self`/`super` against `source_segments` (`:171`), then `rust_module_path` (`:146`) tries progressively shorter prefixes against `src/<...>.rs` / `src/<...>/mod.rs` candidates (`:163`).
8. **Collect** — each resolved path inserted as `Dependency { file, anchor: None }` into a `BTreeSet` (dedup + sort).
9. **Return** — `src/deps/mod.rs:51` — `deps.into_iter().collect()` → `Vec<Dependency>`.

## 5. File index

**`src/deps/`**
| File | Role |
|---|---|
| `mod.rs` | Public dispatcher `collect_outbound`; language switch; declares the four language submodules + `utils` |
| `rust.rs` | Rust extractor: resolves `mod` decls and `use` paths (`crate`/`self`/`super`) to `src/…` files via tree-sitter import list |
| `typescript.rs` | JS/TS/TSX extractor: resolves relative (`.`/`/`) import specifiers with extension/`index.*` fallbacks |
| `python.rs` | Python extractor: hand-parses `import x` and `from y import z`; handles relative-dot modules and `__init__.py` packages |
| `go.rs` | Go extractor: scans `import`/`import (…)` blocks, resolves module-prefixed + relative imports to all `.go` files in the target package dir |
| `utils.rs` | Shared path resolvers: `resolve_file`, `resolve_with_exts` (extension/index fallback), `list_go_package_files` |

## 6. Key types & contracts

| Type | `file:line` | What it carries |
|---|---|---|
| `Dependency` | `src/index/deps.rs:19` | `{ file: PathBuf, anchor: Option<String> }` — resolved workspace-relative target; `anchor` always `None` from this module |
| `CodeLanguage` | `src/lang.rs:15` | Enum (Rust / JavaScript / TypeScript / Tsx / Python / Go); `from_path` (`:26`) drives dispatch |
| per-language collector signature | e.g. `src/deps/rust.rs:24` | `(root: &Path, source_file: &Path, source: &str, deps: &mut BTreeSet<Dependency>)` — uniform contract across all four |
| `RustDependencyCollector` / `TsDependencyCollector` | `src/deps/rust.rs:33`, `src/deps/typescript.rs:31` | Holds `root` + `source_file` for the resolution helpers (python/go are free functions) |

## 7. Extension points

| To do this… | Touch | Notes |
|---|---|---|
| Add a new language | new `src/deps/<lang>.rs` + `mod` decl in `mod.rs:11-15` + arm in `mod.rs:42-49` | Must add the variant to `CodeLanguage` (`src/lang.rs`) and an import parser in `crate::resolve` |
| Change path-resolution fallbacks (extensions / index files) | `src/deps/utils.rs:14` (`resolve_with_exts`) | Shared by TS; Rust/Python/Go build candidates themselves |
| Tweak TS extension preference order | `src/deps/typescript.rs:80-81` (`JS_FIRST` / `TS_FIRST`) | Order = first existing file wins |
| Tweak Rust `use` prefix parsing | `src/deps/rust.rs:64` (`parse_use_prefix`) | Underlying tokenization is in `crate::resolve::collect_mod_and_use` |
| Attach symbol anchors | all four collectors + `Dependency` construction | Currently hardcoded `anchor: None` everywhere |

## 8. Invariants & gotchas

- **`anchor` is always `None`.** Every `Dependency` produced here is file-level only; no symbol granularity. Consumers must not assume anchors.
- **Existence-gated.** A candidate is only emitted if the resolved file actually exists on disk (`resolve_file` checks `is_file`). Imports of files outside the workspace, or of third-party packages, silently produce nothing.
- **Workspace-relative outputs only.** `normalize_rel_path` (`src/workspace/paths.rs`) is the gatekeeper — paths that escape the root (`..` past root) are dropped.
- **Rust = tree-sitter; Python & Go = string scanning.** Python (`python.rs`) and Go (`go.rs`) do *not* use `crate::resolve`'s tree-sitter parsers — they hand-scan lines (strip `#` / `//` comments, match `import `/`from `/`import (`). Multi-line Python imports without parens, or unusual formatting, can be missed.
- **Go expands to the whole package.** A single Go import resolves to **every** `.go` file in the target directory (`list_go_package_files`), not one file — Go packages are directory-scoped.
- **Go relative + module imports need `go.mod`.** `go_module_name` reads `root/go.mod`; without it, only `./`-relative imports resolve.
- **Dedup + deterministic order** comes from collecting into a `BTreeSet<Dependency>` before returning — relies on `Dependency`'s derived `Ord`.

## 9. Dependencies & boundaries

- **Calls out to:** `crate::resolve` (`collect_mod_and_use`, `collect_specifiers` — tree-sitter import parsing), `crate::lang::CodeLanguage`, `crate::workspace::paths::normalize_rel_path`, `crate::index::deps::Dependency`, `std::fs`.
- **Called in by:** declared `pub mod deps` in `src/lib.rs:29`, but **no in-repo caller invokes `collect_outbound`** at this commit (confirmed by repo-wide grep). The shipped `kdb deps` command (`src/cmd.rs:423`) uses `crate::index::deps` (markdown) / `collect_code_outbound` (`src/index/deps.rs:51`, reads the prebuilt `CodeIndex.code_imports`) instead.
- **Owns / does not own:** owns single-file code import→path resolution and its language dispatch. Does **not** own markdown deps, the workspace-wide import index (`build_workspace_import_index`), symbol resolution, or any persisted index/state.

## 10. Open questions / staleness

- [ ] **Is this module dead/legacy?** `collect_outbound` has no callers; the live command goes through `index::deps`. Likely a superseded single-file path or an API kept for external/library use. Worth confirming whether to wire it into `kdb deps` or remove it.
- [ ] `Dependency` is defined in `src/index/deps.rs` (the markdown-deps module), not in this domain — a structural coupling that may surprise readers. Not verified whether a move was intended.
- [ ] Python/Go extractors diverge from the tree-sitter approach used by Rust/TS; parser-fidelity differences (e.g. conditional imports, multiline statements) are not characterized by tests I located here.
- [ ] No tests live under `src/deps/`; correctness of resolution edge cases (Rust `super::super`, TS `index.*`, Go vendored paths) is unverified from this map's vantage.
