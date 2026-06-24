---
domain: "render"
repo: "kdb"
root: "src/render"
owner: "kdb"
updated: 2026.06.24
commit: "8c33d68"
confidence: high
---

# Render — Code Map

> A map, not a manual. It points to where things live and how a request flows;
> it does not duplicate per-function detail (that's what the code and `kdb outline` are for).

## 1. What this is

Transclusion resolution for markdown: it resolves Obsidian-style `![[file#heading]]`
embeds at render time, recursively splicing the target file (or a single heading
section of it) into the source document. In scope: parsing embed syntax, resolving
embed paths, extracting heading sections, recursive expansion with cycle/depth
guards, and a validate-only entry for `kdb check`. Out of scope: the markdown parser
itself (`crate::index::parse_markdown` / `section_byte_bounds`), regular `[[wikilink]]`
resolution (that lives in the index/link layer), and the `kdb codemap render` command
(unrelated — `src/codemap/render.rs`).

## 2. Shape — diagram

```
 kdb render FILE                              kdb check
 (cmd.rs:512)                                 (index/mod.rs:626)
      │                                            │
      │ render_file                                │ find_embeds + validate_embed_target
      ▼                                            ▼
 ┌──────────────────┐                       ┌──────────────────────┐
 │ mod.rs            │                       │ include::find_embeds  │
 │ render_file  :38  │                       │   (scan lines) :129   │
 │ render_content:47 │                       └──────────┬───────────┘
 └─────────┬─────────┘                                  │ per embed
           │ seed visited set                           ▼
           ▼                                  ┌────────────────────────┐
 ┌─────────────────────────┐                  │ resolve::               │
 │ resolve::render_content  │◀──── recurse ──┐ │ validate_embed_target   │
 │   :145                   │                │ │   :219 (no splice)      │
 └─────────┬───────────────┘                │ └────────────────────────┘
           │ find_embeds (include.rs)        │
           ▼                                 │
 ┌─────────────────────────┐                 │
 │ resolve_directive :189   │─────────────────┘
 │  resolve_include_path:84 │   guards: MAX_DEPTH=10 · visited cycle set
 │  extract_section    :116 │
 └─────────┬───────────────┘
           │ anchor? → parse_markdown + section_byte_bounds (crate::index)
           ▼
   spliced section text
```

## 3. Entry points

| Trigger | Handler (`file:line`) | Purpose |
|---|---|---|
| `kdb render <file>` | `src/cmd.rs:512` → `render::render_file` | Read a file, resolve all embeds, print to stdout |
| `kdb check` (embed validation) | `src/index/mod.rs:626-651` → `find_embeds` + `validate_embed_target` | Report broken embeds without rendering |
| `render::render_content` | `src/render/mod.rs:47` | Resolve embeds in an in-memory string (public API) |
| `render::include::parse_embed_target` | `src/render/include.rs:68` | Public: parse `![[...]]` inner text into a directive |

## 4. Lifecycle — the trace

`kdb render notes/daily.md` with `![[SOP.md#setup]]` inside:

1. **Command dispatch** — `src/cmd.rs:512` — `render_file(workspace.root, rel_path)` is called with the root-relative path.
2. **Read + full section** — `src/render/mod.rs:38-40` — `extract_section(root, rel_path, None)` reads the whole file (no anchor).
3. **Seed cycle guard** — `src/render/mod.rs:47-55` — `render_content` inserts the source file key into a fresh `visited` HashSet, then calls `resolve::render_content` at depth 0.
4. **Depth guard + scan** — `src/render/resolve.rs:152-157` — bail if `depth > MAX_DEPTH` (10); otherwise `find_embeds(&lines)` returns standalone `![[...]]` lines.
5. **Find embeds** — `src/render/include.rs:129-155` — line-by-line scan; toggles a code-fence flag on ` ``` `/`~~~`, matches `EMBED_RE`, parses inner via `parse_embed_target`.
6. **Resolve each directive** — `src/render/resolve.rs:189-214` — `resolve_directive` resolves the path, builds a `visit_key` (`path` or `path#anchor`), inserts it into `visited` (cycle check), extracts the section, recurses at `depth+1`, then removes the key.
7. **Path resolution** — `src/render/resolve.rs:84-112` — `resolve_include_path`: append `.md` if no ext; `kdb://` → root-relative else relative to source's parent; normalize; error if not a file.
8. **Section extraction** — `src/render/resolve.rs:116-139` — read target; with anchor, `parse_markdown` + `section_byte_bounds` (from `crate::index`) slice the heading section.
9. **Splice back** — `src/render/resolve.rs:163-185` — replacements applied bottom-to-top (reversed) so line indices stay stable; `splice` each embed line with its resolved lines; restore trailing newline.
10. **Print** — `src/cmd.rs:515` — final rendered string printed to stdout.

## 5. File index

**`src/render/`**
| File | Role |
|---|---|
| `mod.rs` | Public entry: `render_file` (read+resolve) and `render_content` (in-memory). Seeds the cycle-detection set; declares `include`/`resolve` as `pub` modules and re-exports `IncludeError` (`pub use resolve::IncludeError`). |
| `include.rs` | Embed syntax: `EMBED_RE`, `IncludeDirective`/`Embed` structs, `parse_embed_target`, `find_embeds` (code-fence aware). Has unit tests. |
| `resolve.rs` | Recursion engine: `IncludeError`, `resolve_include_path`, `extract_section`, `render_content` (recursive splice), `resolve_directive`, `validate_embed_target`. |

## 6. Key types & contracts

| Type | `file:line` | What it carries |
|---|---|---|
| `IncludeDirective` | `src/render/include.rs:35` | `file: String`, `anchor: Option<String>`, `root_relative: bool` — the parsed embed target |
| `Embed` | `src/render/include.rs:46` | `directive: IncludeDirective` + `line: usize` (0-based) — a located embed |
| `IncludeError` | `src/render/resolve.rs:34` | Failure modes: `FileNotFound`, `HeadingNotFound`, `CycleDetected`, `MaxDepthExceeded`, `ReadError` |
| `MAX_DEPTH` | `src/render/resolve.rs:30` | `const usize = 10` — recursion ceiling |
| `EMBED_RE` | `src/render/include.rs:30` | `^!\[\[([^\]]+)\]\]$` — whole-line embed matcher |

## 7. Extension points

| To do this… | Touch | Notes |
|---|---|---|
| Change embed syntax (e.g. allow inline) | `include.rs` | Edit `EMBED_RE:30` / `find_embeds:129`; today it only matches whole-line embeds |
| Add a new URL scheme (besides `kdb://`) | `include.rs:75`, `resolve.rs:96` | Parse the prefix in `parse_embed_target`; branch on it in `resolve_include_path` |
| Add a new failure mode | `resolve.rs:34` + `:51` | Add variant to `IncludeError` and its `Display` arm |
| Tune recursion limit | `resolve.rs:30` | `MAX_DEPTH` constant |
| Add an embed consumer | (caller side) | Call `render_file`/`render_content` for output, or `find_embeds`+`validate_embed_target` for validation (see `index/mod.rs:626`) |

## 8. Invariants & gotchas

- **Embeds must be on their own line.** `find_embeds` only matches lines whose trimmed content is exactly `![[...]]` (`EMBED_RE` is anchored `^...$`). Inline embeds are ignored by design (see test `find_embeds_standalone_only`).
- **Code fences are skipped.** `find_embeds:137-144` toggles in/out of ` ``` `/`~~~` blocks so example embeds in docs aren't expanded. The toggle is a simple boolean — nested/mismatched fences can desync it.
- **Replacements are applied bottom-to-top.** `render_content:172` reverses replacements before splicing so earlier line indices remain valid as later lines expand into multiple lines.
- **Cycle detection is keyed by `path` or `path#anchor`.** `resolve_directive:198-206` — the same file embedded twice with *different* anchors is allowed; the key is removed after recursion (`:211`) so it's a true in-progress-chain check, not a visited-once check.
- **`render_content` (mod.rs) seeds `visited` with the source file** (`mod.rs:53-54`) so a doc embedding itself is caught.
- **Wikilink `.md` auto-append.** `resolve_include_path:92-94` appends `.md` only when the path has *no* extension — `![[foo.png]]` stays `foo.png`.
- **`CycleDetected.chain` order is non-deterministic** — built from `visited.iter()` over a HashSet (`resolve.rs:204`), so the reported chain is unordered.
- **Validation only checks the anchor when present.** `validate_embed_target:226` confirms the file via `resolve_include_path` (which already requires `is_file()`), then only re-reads + parses when an anchor exists.

## 9. Dependencies & boundaries

- **Calls out to:** `crate::index::{parse_markdown, section_byte_bounds}` (markdown parsing / heading bounds), `crate::workspace::paths::normalize_rel_path`, `regex`, std `fs`/`path`.
- **Called in by:** `src/cmd.rs` (`render` command, :512), `src/index/mod.rs` (`check` report, :626). `render::include` and `render::resolve` are both declared as `pub` modules in `mod.rs` (`pub mod include;` / `pub mod resolve;`, lines 8-9), so callers can reach them directly; the only `pub use` re-export is `resolve::IncludeError` (line 14).
- **Owns / does not own:** Owns embed parsing + recursive resolution and the `IncludeError` type. Does *not* own the markdown parser, link (`[[...]]`) resolution, the workspace/path layer, or disk-walk — it reads individual target files on demand but builds no index.

## 10. Open questions / staleness

- [ ] Code-fence detection (`find_embeds:137`) is a flat boolean toggle; behavior with indented fences or info-string edge cases is untested beyond the one fence test.
- [ ] `CycleDetected.chain` ordering is non-deterministic (HashSet iteration) — fine for an error message, but a test asserting chain order would be brittle.
- [ ] No verification that `kdb render --project/--all` (the materialize-TODO path in cmd.rs above :505) routes through this domain — that branch returns early before `render_file`; it appears to use a different rendering path.
