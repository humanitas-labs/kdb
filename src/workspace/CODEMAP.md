---
domain: "workspace"
repo: "kdb"
root: "src/workspace"
owner: "kdb"
updated: 2026.06.24
commit: "8c33d68"
confidence: medium
---

# workspace — Code Map

> A map, not a manual. It points to where things live and how a request flows;
> it does not duplicate per-function detail (that's what the code and `kdb outline` are for).

## 1. What this is

Shared workspace infrastructure: it discovers the workspace root (the nearest
`.kdb/` marker walking upward), loads ignore configuration, normalizes
vault-relative paths, and walks the tree to discover files. Every kdb subsystem
(tasks, tree, fmt, index, search, LSP, codemap…) bootstraps through here. In
scope: root discovery, ignore loading/compilation, path normalization, file
discovery. Out of scope: anything that consumes the discovered files (indexing,
symbol extraction, rendering) — those live in their own domains.

## 2. Shape — diagram

```
 caller (cmd.rs / lsp / search / fmt / index / codemap)
   │  WorkspaceContext::discover(start)
   ▼
┌──────────────────────────┐
│ mod.rs                    │
│ WorkspaceContext   :35    │
│  ::discover        :49 ───┼──▶┌──────────────┐ find_root walks up
│  ::from_root       :59    │   │ root.rs      │ for ".kdb" marker
└─────────┬────────────────┘   │ find_root:36 │
          │ from_root assembles └──────────────┘
          │ ignore patterns + globset
          ├──▶┌──────────────────┐  .kdb/ignore lines
          │   │ ignore.rs        │  → "**/{name}" globs
          │   │ load_ignore  :58 │
          │   │ build_globset:95 │
          │   └──────────────────┘
          └──▶┌──────────────────────┐  [index].ignore
              │ config.rs            │  from config.toml
              │ load_index_ignores:24│
              └──────────────────────┘
                      │
   { root, ignore_patterns, ignore_set }
                      ▼
        ┌──────────────────────────┐  parallel ignore-aware walk
        │ discover.rs               │──▶ Vec<PathBuf> (sorted, rel)
        │ discover_files       :25  │
        └─────────┬─────────────────┘
                  │ uses
                  ▼
        ┌──────────────────────────┐
        │ paths.rs                  │  resolve . / .. , reject escapes
        │ normalize_rel_path   :19  │
        └──────────────────────────┘
```

## 3. Entry points

The public API other domains call. Most callers start at `WorkspaceContext::discover`.

| Trigger | Handler (`file:line`) | Purpose |
|---|---|---|
| `WorkspaceContext::discover(start)` | `mod.rs:49` | Find root from a start path, load+compile config, return context. Used by `cmd.rs:116` per command. |
| `WorkspaceContext::from_root(root)` | `mod.rs:59` | Build context when root is already known (e.g. LSP after init). |
| `workspace::root::find_root(start)` | `root.rs:36` | Walk upward to the `.kdb/` marker. Used directly by `lsp/backend.rs:81`. |
| `workspace::root::make_absolute(path)` | `root.rs:71` | Resolve a relative CLI path against cwd. |
| `workspace::discover::discover_files(root, scope, ignore_set)` | `discover.rs:25` | Walk `scope`, return sorted ignore-filtered rel paths. Used by search/fmt/index/codemap/symbols. |
| `workspace::paths::normalize_rel_path(path)` | `paths.rs:19` | Resolve `.`/`..`, reject root-escaping paths. ~34 invocations across 16 files. |

## 4. Lifecycle — the trace

A CLI command resolving its workspace and discovering files:

1. **Command starts** — `cmd.rs:116` — calls `WorkspaceContext::discover(&start)`.
2. **Find root** — `mod.rs:50` → `root::find_root` `root.rs:36` — canonicalizes `start`, walks parents until a dir contains `.kdb/` (`root.rs:55`); bails if none found.
3. **Build context** — `mod.rs:59` `from_root` — canonicalizes the root path.
4. **Load ignore file** — `mod.rs:63` → `ignore::load_ignore_file` `ignore.rs:58` — reads `.kdb/ignore`, or falls back to `DEFAULT_IGNORE` (`ignore.rs:36`) if absent; bare names become `**/{name}` globs (`ignore.rs:85`).
5. **Load config ignores** — `mod.rs:64` → `config::load_index_ignores` `config.rs:24` — reads `[index].ignore` from `.kdb/config.toml` (empty if file/field missing).
6. **Compile globset** — `mod.rs:66` → `ignore::build_ignore_globset` `ignore.rs:95` — merges patterns, always prepends `**/.kdb` (`ignore.rs:99`).
7. **Discover files** — caller hands `ctx.ignore_set` to `discover_files` `discover.rs:25` — parallel `ignore::WalkBuilder` walk honoring `.gitignore`/`.ignore` plus the compiled globset.
8. **Per-entry filter** — `should_visit_entry` `discover.rs:99` prunes ignored dirs; `path_is_ignored` `ignore.rs:119` re-checks files; `rel_path_from_root` `discover.rs:94` strips the root prefix and `normalize_rel_path` `paths.rs:19` cleans it.
9. **Return** — `discover.rs:89` (sort) + `discover.rs:90` (dedup) — paths sorted + deduped, returned to the caller.

## 5. File index

**`src/workspace/`**
| File | Role |
|---|---|
| `mod.rs` | Module root; defines `WorkspaceContext` and its `discover`/`from_root` constructors that wire the submodules together. |
| `root.rs` | Root discovery (`find_root`), marker constants (`.kdb`, `config.toml`), `config_path`, `make_absolute`. |
| `config.rs` | Loads `[index].ignore` patterns from `.kdb/config.toml`. |
| `ignore.rs` | Canonical ignore handling: `.kdb/ignore` loading, default patterns, gitignore-style parsing, globset compilation, match test. Has unit tests. |
| `paths.rs` | `normalize_rel_path` — pure `.`/`..` resolution, rejects root escapes and absolute components. |
| `discover.rs` | Parallel ignore-aware file walker built on the `ignore` crate. |

## 6. Key types & contracts

| Type | `file:line` | What it carries |
|---|---|---|
| `WorkspaceContext` | `mod.rs:35` | The canonical workspace state: `root: PathBuf`, `ignore_patterns: Vec<String>`, `ignore_set: GlobSet`. Threaded to all subsystems. |
| `GlobSet` (from `globset`) | `mod.rs:41` | Compiled ignore matcher; the unit of work passed to `discover_files` and `path_is_ignored`. |
| `ROOT_MARKER` / `CONFIG_FILE` | `root.rs:22` / `root.rs:25` | `".kdb"` / `"config.toml"` — the workspace identity constants. |
| `DEFAULT_IGNORE` | `ignore.rs:36` | Fallback ignore text used when `.kdb/ignore` is absent (also what `kdb init` writes). |

## 7. Extension points

| To do this… | Touch | Notes |
|---|---|---|
| Add a new always-ignored dir default | `ignore.rs:36` (`DEFAULT_IGNORE`) | Only affects new/absent ignore files; existing workspaces keep their `.kdb/ignore`. |
| Add a new workspace config field | `config.rs` | Mirror `load_index_ignores`/`parse_index_ignores`; read it in `from_root` (`mod.rs:59`) and store on `WorkspaceContext`. |
| Change root-marker semantics | `root.rs:22` (`ROOT_MARKER`), `root.rs:36` (`find_root`) | Marker name + the upward walk loop. |
| Tune file discovery behavior | `discover.rs:33` (`WalkBuilder` config), `should_visit_entry:99` | e.g. follow symlinks, change gitignore honoring. |
| Add a path-normalization rule | `paths.rs:19` | Pure function; covered implicitly by discovery + many callers. |

## 8. Invariants & gotchas

- **`.kdb` is always ignored, non-configurable** — `build_ignore_globset` (`ignore.rs:99`) unconditionally prepends `**/.kdb` before user patterns. Removing a user pattern can't expose `.kdb`.
- **Two ignore sources are merged** — `.kdb/ignore` (gitignore-style, depth-wrapped) *and* `[index].ignore` in `config.toml`. `from_root` (`mod.rs:63-65`) concatenates them; both feed one globset.
- **Bare names match at any depth** — `parse_ignore_lines` (`ignore.rs:85`) wraps names without `/`, `*`, `?` as `**/{name}`. A pattern with a slash or glob char is taken literally — surprising if you write `foo/` expecting depth-agnostic matching.
- **Globs use `literal_separator(true)`** — `ignore.rs:107` — so `*` does not cross `/`. Matches gitignore-ish intent but differs from naive globbing.
- **Discovery double-filters** — directories are pruned in `should_visit_entry` (`discover.rs:99`) and files re-checked via `path_is_ignored` (`discover.rs:73`); plus the `ignore` crate's own `.gitignore`/`.ignore` handling is on (`discover.rs:36-41`). A file can be excluded by any of the three layers.
- **`normalize_rel_path` returns `None` on escape** — `paths.rs:19` rejects paths with more `..` than depth or any absolute/root component. Callers must treat `None` as "invalid path," not "empty."
- **Root is always canonicalized** — both `find_root` (`root.rs:42`) and `from_root` (`mod.rs:60`) canonicalize, so symlinked roots resolve to real paths; downstream `strip_prefix` (`discover.rs:95`) depends on this.
- **Missing config/ignore files are not errors** — both `load_index_ignores` (`config.rs:28`) and `load_ignore_file` (`ignore.rs:63`) treat `NotFound` as a default, so fresh or minimal workspaces still work.

## 9. Dependencies & boundaries

- **Calls out to:** `anyhow` (errors), `globset` (`GlobBuilder`/`GlobSet`), `ignore` crate (`WalkBuilder`/`WalkState`), `toml` (config parse), std `fs`/`path`/`env`. No kdb-internal upward deps — this is a leaf domain.
- **Called in by (runtime):** `cmd.rs` (per-command bootstrap, root discovery, path normalization), `lsp/backend.rs`, `search.rs`, `fmt/mod.rs`, `index/mod.rs`, `codemap/{discover,check,frontmatter}.rs`, `symbols/query.rs`, `tree.rs`, `resolve/{mod,go,python,rust,tsjs}.rs`, `render/resolve.rs`, `db/mod.rs`, `deps/{python,utils}.rs`. `normalize_rel_path` is invoked ~34 times across 16 files (`cmd.rs`, `tree.rs`, `index/mod.rs`, `symbols/query.rs`, `lsp/backend.rs`, `codemap/{check,frontmatter}.rs`, `render/resolve.rs`, `deps/{python,utils}.rs`, `resolve/{mod,go,python,rust,tsjs}.rs`, plus `discover.rs` internally). The `root` module's runtime API (`find_root`, `make_absolute`, `config_path`, `ROOT_MARKER`) is used by `cmd.rs`, `lsp/backend.rs`, `symbols/query.rs`, `db/mod.rs`, and internally by `mod.rs`/`config.rs`. (Note: `tasks.rs`, `cycles.rs`, `labels.rs`, `statuses.rs`, `projects.rs`, `materialize.rs`, and `search.rs` also import `ROOT_MARKER`, but only inside `#[cfg(test)]` modules for fixtures — these are not runtime callers.)
- **Owns / does not own:** Owns root discovery, ignore configuration semantics, path normalization, and the file-walk. Does *not* own the `.kdb/` database, indexing, symbol extraction, or what happens to discovered files — it only produces the `WorkspaceContext` and the path list.

## 10. Open questions / staleness

- [ ] `make_absolute` (`root.rs:71`) does not normalize the result (no `normalize_rel_path` applied), so callers may get paths containing `.`/`..`. Verified behavior, but whether all 5 call sites expect that is not checked here.
- [ ] The interaction between the `ignore` crate's native gitignore handling and the kdb globset is not exhaustively tested for precedence/overlap; both are active in `discover_files`.
- [ ] `kdb init`'s actual written `.kdb/ignore` content is asserted to match `DEFAULT_IGNORE` only by the doc comment (`ignore.rs:33`); the init path lives outside this domain and was not cross-checked.
