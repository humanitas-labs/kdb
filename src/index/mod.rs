//! Markdown parser, vault indexer, and link resolver.
//!
//! This is the core of kdb. It parses markdown files to extract headings and
//! links, builds an in-memory index of the entire vault, and validates that
//! all references resolve to real targets.
//!
//! # Architecture
//!
//! 1. **Parsing** — [`parse_markdown`] extracts headings and links from a single file.
//! 2. **Indexing** — [`VaultIndex::build`] walks the vault directory and parses every
//!    `.md` file, building maps of files, headings, and inbound links.
//! 3. **Checking** — [`VaultIndex::check`] validates all links and reports broken
//!    references and orphan files.
//! 4. **Resolution** — [`resolve_target_path`] resolves a link target relative to
//!    its source file, handling both markdown and wikilink syntax.

mod code;
pub mod deps;
mod markdown;
pub mod prosaic;
pub mod refs;
mod scanner;
mod scope;

use anyhow::{Context, Result};
use globset::GlobSet;
use rayon::prelude::*;
use serde::Serialize;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};

use crate::workspace::discover::discover_files;
use crate::workspace::ignore::{build_ignore_globset, path_is_ignored};
use crate::workspace::paths::normalize_rel_path;
use crate::resolve::{
    GoWorkspaceCache, ResolvedImport, RustWorkspaceCache, WorkspacePackages,
    build_workspace_import_index,
};
use crate::symbols::SymbolKind;

pub use code::SymbolIndex;
pub use markdown::{
    parse_markdown, parse_markdown_target, parse_wikilink_target, section_byte_bounds,
    section_line_bounds, slug_anchor,
};

// ---------------------------------------------------------------------------
// Data types
// ---------------------------------------------------------------------------

// NOTE: index block managed by kdb fmt — do not update manually

// ------------------------------------------
// projects/kdb/src/index/mod.rs
//
// mod code                               L17
// pub mod deps                           L18
// mod markdown                           L19
// pub mod prosaic                        L20
// pub mod refs                           L21
// mod scanner                            L22
// mod scope                              L23
// pub struct VaultIndex                 L116
// pub struct CodeIndex                  L134
// pub struct SymbolKey                  L149
// pub struct SymbolRef                  L164
// pub struct WorkspaceIndex             L183
// pub struct FileEntry                  L192
// pub struct ProsaicBlock               L213
// pub struct Heading                    L224
// pub enum LinkKind                     L240
// pub struct LinkTarget                 L249
// pub struct Link                       L261
// pub struct HeadingKey                 L276
// pub struct LinkRef                    L285
// pub struct ParsedDocument             L300
// pub struct BrokenLink                 L311
// pub struct BrokenEmbed                L326
// pub struct ProcedureError             L340
// pub struct CheckReport                L355
//   pub fn has_errors()                 L369
//   pub fn print()                      L376
//   pub fn scoped_to()                  L452
// fn path_is_in_check_scope()           L465
//   pub fn build()                      L483
//   pub fn build_for_target()           L497
//   pub fn build_with_symbol_refs()     L514
//   pub fn build()                      L529
//   pub fn build_with_ignores()         L534
//   pub fn build_for_target()           L546
//   pub fn build_with_symbol_refs()     L561
//   pub fn build()                      L576
//   pub fn build_with_ignores()         L584
//   pub fn upsert_file()                L629
//   pub fn reload_file()                L653
//   pub fn remove_file()                L683
//   pub fn check()                      L691
//   fn check_procedures()               L761
//   fn resolve_use()                    L828
//   fn populate_inbound()               L844
//   fn resolve_link()                   L897
// enum ResolveError                     L930
//   fn message()                        L937
// fn procedure_ids_in()                 L954
// fn discover_markdown_files()          L963
// fn resolve_target_file()              L981
// pub fn resolve_target_path()          L994
// pub fn resolve_file_target()         L1026
// ------------------------------------------

/// Complete index of a markdown vault.
///
/// Built by scanning all `.md` files under a root directory. Provides the
/// foundation for link validation, reference lookups, and LSP features.
#[derive(Debug, Clone)]
pub struct VaultIndex {
    /// Canonical absolute path to the vault root directory.
    pub root: PathBuf,
    /// Compiled user ignore patterns used during discovery and incremental updates.
    ignore_set: GlobSet,
    /// All indexed markdown files, keyed by their path relative to `root`.
    pub files: BTreeMap<PathBuf, FileEntry>,
    /// Inbound links grouped by target file path.
    pub file_inbound: HashMap<PathBuf, Vec<LinkRef>>,
    /// Inbound links grouped by target heading (file + anchor).
    pub heading_inbound: HashMap<HeadingKey, Vec<LinkRef>>,
}

/// Index of workspace-level code imports and language caches.
///
/// Built by scanning all supported code files under the workspace root and
/// resolving their imports. Used by `kdb deps` for code dependency tracking.
#[derive(Debug, Clone, Default)]
pub struct CodeIndex {
    /// Workspace package map (`package.json` name -> local relative directory).
    pub workspace_packages: WorkspacePackages,
    /// Go workspace module cache used during import resolution.
    pub go_workspace: GoWorkspaceCache,
    /// Rust workspace crate/dependency cache used during import resolution.
    pub rust_workspace: RustWorkspaceCache,
    /// Per-code-file resolved imports used by `kdb deps` and code refs.
    pub code_imports: BTreeMap<PathBuf, Vec<ResolvedImport>>,
    /// Declaration symbols and inbound references for `kdb refs -s`.
    pub symbols: SymbolIndex,
}

/// Stable key for a symbol definition when indexing code references.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SymbolKey {
    /// Relative path to the file where the symbol is defined.
    pub file: PathBuf,
    /// Symbol name.
    pub name: String,
    /// Optional parent context (e.g. class or impl type).
    pub parent: Option<String>,
    /// Symbol kind.
    pub kind: SymbolKind,
    /// 1-based definition line (disambiguates duplicate names).
    pub line: usize,
}

/// A single inbound code reference row for `kdb refs -s`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SymbolRef {
    /// Relative path to the file containing the reference.
    pub source_file: PathBuf,
    /// 1-based line number of the reference.
    pub line: usize,
    /// 1-based column number of the reference.
    pub column: usize,
    /// Trimmed source line used for display.
    pub snippet: String,
    /// Whether this row is the declaration site itself.
    pub is_definition: bool,
}

/// Combined vault and code index for a workspace.
///
/// Commands that need both markdown and code data (e.g. `kdb deps`) build
/// this; commands that only need markdown (e.g. `kdb check`) use [`VaultIndex`]
/// directly.
#[derive(Debug, Clone)]
pub struct WorkspaceIndex {
    /// Markdown vault index (files, headings, links, inbound maps).
    pub vault: VaultIndex,
    /// Code import index (workspace packages, language caches, resolved imports).
    pub code: CodeIndex,
}

/// A single indexed markdown file.
#[derive(Debug, Clone)]
pub struct FileEntry {
    /// Path relative to the vault root (e.g. `notes/react.md`).
    pub rel_path: PathBuf,
    /// Absolute path on disk.
    pub abs_path: PathBuf,
    /// All headings found in this file, in document order.
    pub headings: Vec<Heading>,
    /// All internal links found in this file (external URLs are excluded).
    pub links: Vec<Link>,
    /// Prosaic (`prosaic`-fenced) code blocks found in this file.
    pub prosaic_blocks: Vec<ProsaicBlock>,
}

/// A `prosaic`-fenced code block extracted from a markdown file.
///
/// Prosaic composition (spec §5) declares procedures as `## <ID> :: <Name>`
/// headings and calls them with `use`/`run` inside these blocks. Only blocks
/// that are the body of a procedure (`enclosing_procedure` is `Some`) are
/// subject to the §5.4 resolution check — illustrative blocks in the spec or
/// templates sit under ordinary headings and are skipped.
#[derive(Debug, Clone)]
pub struct ProsaicBlock {
    /// 1-based line number of the first body line (the line after the opening fence).
    pub start_line: usize,
    /// Raw body text between the fences (fence delimiters excluded).
    pub body: String,
    /// ID of the procedure whose section contains this block, if any.
    pub enclosing_procedure: Option<String>,
}

/// A parsed heading from a markdown file.
#[derive(Debug, Clone)]
pub struct Heading {
    /// The heading text content (e.g. `"Getting Started"`).
    pub title: String,
    /// URL-safe anchor slug (e.g. `"getting-started"`). Deduplicated with
    /// numeric suffixes when multiple headings produce the same slug.
    pub anchor: String,
    /// Heading depth: 1 for `#`, 2 for `##`, etc.
    pub level: u8,
    /// 1-based line number in the source file.
    pub line: usize,
    /// 1-based column number in the source file.
    pub column: usize,
}

/// Whether a link uses standard markdown syntax or wikilink syntax.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LinkKind {
    /// Standard markdown: `[text](path.md#anchor)`
    Markdown,
    /// Obsidian-style wikilink: `[[path#anchor]]`
    Wikilink,
}

/// The parsed destination of a link, split into file and anchor components.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkTarget {
    /// Target file path (e.g. `"hooks.md"`), or `None` for same-file anchors like `#section`.
    pub file: Option<String>,
    /// Target heading anchor (e.g. `"useEffect"`), or `None` for file-only links.
    pub anchor: Option<String>,
    /// When `true` the file path is relative to the vault root (`kdb://` scheme)
    /// rather than the source file's directory.
    pub root_relative: bool,
}

/// A parsed link from a markdown file.
#[derive(Debug, Clone)]
pub struct Link {
    /// Whether this is a markdown link or wikilink.
    pub kind: LinkKind,
    /// The raw link text as it appears in the source (e.g. `"hooks.md#useEffect"` or `"[[hooks#useEffect]]"`).
    pub raw: String,
    /// The parsed target components.
    pub target: LinkTarget,
    /// 1-based line number in the source file.
    pub line: usize,
    /// 1-based column number in the source file.
    pub column: usize,
}

/// Uniquely identifies a heading within the vault (file path + anchor slug).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct HeadingKey {
    /// Relative path to the file containing the heading.
    pub file: PathBuf,
    /// The heading's anchor slug.
    pub anchor: String,
}

/// A record of where a link originates, used for inbound reference tracking.
#[derive(Debug, Clone, Serialize)]
pub struct LinkRef {
    /// Relative path to the file containing the link.
    pub source_file: PathBuf,
    /// 1-based line number of the link.
    #[serde(rename = "line")]
    pub source_line: usize,
    /// 1-based column number of the link.
    #[serde(rename = "column")]
    pub source_column: usize,
    /// Raw link text for display in diagnostics.
    pub raw: String,
}

/// The result of parsing a single markdown file.
#[derive(Debug, Clone)]
pub struct ParsedDocument {
    /// All headings found in the document.
    pub headings: Vec<Heading>,
    /// All internal links found in the document.
    pub links: Vec<Link>,
    /// All `prosaic`-fenced code blocks found in the document.
    pub prosaic_blocks: Vec<ProsaicBlock>,
}

/// A broken link detected during validation.
#[derive(Debug, Clone)]
pub struct BrokenLink {
    /// File containing the broken link.
    pub source_file: PathBuf,
    /// 1-based line number.
    pub line: usize,
    /// 1-based column number.
    pub column: usize,
    /// Raw link text.
    pub raw: String,
    /// Human-readable explanation of why the link is broken.
    pub reason: String,
}

/// A broken embed (`![[target]]`) detected during validation.
#[derive(Debug, Clone)]
pub struct BrokenEmbed {
    /// File containing the broken embed.
    pub source_file: PathBuf,
    /// 1-based line number.
    pub line: usize,
    /// Raw embed text.
    pub raw: String,
    /// Human-readable explanation of why the embed is broken.
    pub reason: String,
}

/// An unresolved Prosaic composition reference (`use`/`run`) detected during
/// validation (spec §5.4).
#[derive(Debug, Clone)]
pub struct ProcedureError {
    /// File containing the reference.
    pub source_file: PathBuf,
    /// 1-based line number.
    pub line: usize,
    /// 1-based column number.
    pub column: usize,
    /// Raw statement text.
    pub raw: String,
    /// Human-readable explanation of why the reference is unresolved.
    pub reason: String,
}

/// Results from running `kdb check` on a vault.
#[derive(Debug, Clone, Default)]
pub struct CheckReport {
    /// Links that reference files or headings that don't exist.
    pub broken_links: Vec<BrokenLink>,
    /// Embeds (`![[]]`) that reference files or headings that don't exist.
    pub broken_embeds: Vec<BrokenEmbed>,
    /// Prosaic `use`/`run` references that don't resolve (spec §5.4).
    pub procedure_errors: Vec<ProcedureError>,
    /// Files that have no inbound links from other files.
    pub orphans: Vec<PathBuf>,
}

impl CheckReport {
    /// Returns `true` if the report contains any broken links, embeds, or
    /// unresolved procedure references.
    pub fn has_errors(&self) -> bool {
        !self.broken_links.is_empty()
            || !self.broken_embeds.is_empty()
            || !self.procedure_errors.is_empty()
    }

    /// Print a human-readable summary to stdout.
    pub fn print(&self, list_orphans: bool) {
        for broken in &self.broken_links {
            println!(
                "{}:{}:{} broken link {} ({})",
                broken.source_file.display(),
                broken.line,
                broken.column,
                broken.raw,
                broken.reason
            );
        }

        for broken in &self.broken_embeds {
            println!(
                "{}:{} broken embed {} ({})",
                broken.source_file.display(),
                broken.line,
                broken.raw,
                broken.reason
            );
        }

        for error in &self.procedure_errors {
            println!(
                "{}:{}:{} unresolved reference {} ({})",
                error.source_file.display(),
                error.line,
                error.column,
                error.raw,
                error.reason
            );
        }

        if list_orphans {
            for orphan in &self.orphans {
                println!("{} orphan file (0 inbound links)", orphan.display());
            }
        } else if !self.orphans.is_empty() {
            let noun = if self.orphans.len() == 1 {
                "orphan file"
            } else {
                "orphan files"
            };
            println!(
                "{} {} (run `kdb check --orphans` to list)",
                self.orphans.len(),
                noun
            );
        }

        let error_count =
            self.broken_links.len() + self.broken_embeds.len() + self.procedure_errors.len();
        if error_count == 0 && self.orphans.is_empty() {
            println!("kdb check: no issues found");
            return;
        }

        if error_count > 0 {
            let noun = if error_count == 1 { "error" } else { "errors" };
            println!("{error_count} {noun}");
        }

        if !self.orphans.is_empty() {
            let noun = if self.orphans.len() == 1 {
                "warning"
            } else {
                "warnings"
            };
            println!("{} {noun}", self.orphans.len());
        }
    }

    /// Return a copy of this report limited to issues originating in `scope_rel`.
    ///
    /// When `scope_is_dir` is true, all descendants of `scope_rel` are kept.
    /// Otherwise, only entries whose source file exactly equals `scope_rel` are kept.
    pub fn scoped_to(mut self, scope_rel: &Path, scope_is_dir: bool) -> Self {
        self.broken_links
            .retain(|broken| path_is_in_check_scope(&broken.source_file, scope_rel, scope_is_dir));
        self.broken_embeds
            .retain(|broken| path_is_in_check_scope(&broken.source_file, scope_rel, scope_is_dir));
        self.procedure_errors
            .retain(|error| path_is_in_check_scope(&error.source_file, scope_rel, scope_is_dir));
        self.orphans
            .retain(|orphan| path_is_in_check_scope(orphan, scope_rel, scope_is_dir));
        self
    }
}

fn path_is_in_check_scope(path: &Path, scope_rel: &Path, scope_is_dir: bool) -> bool {
    if scope_rel.as_os_str().is_empty() {
        return true;
    }

    if scope_is_dir {
        path.starts_with(scope_rel)
    } else {
        path == scope_rel
    }
}

// ---------------------------------------------------------------------------
// Vault index
// ---------------------------------------------------------------------------

impl CodeIndex {
    /// Build a code index by scanning all supported code files under `root`.
    pub fn build(root: &Path, ignore_patterns: &[String]) -> Result<Self> {
        let import_index = build_workspace_import_index(root, ignore_patterns)?;
        Ok(Self {
            workspace_packages: import_index.workspace_packages,
            go_workspace: import_index.go_workspace,
            rust_workspace: import_index.rust_workspace,
            code_imports: import_index.file_imports,
            symbols: SymbolIndex::default(),
        })
    }

    /// Build a fresh import scan + targeted symbol build for `kdb refs -s <file>`.
    ///
    /// Only extracts symbols and scans usages for files that import from `target_file`.
    pub fn build_for_target(
        root: &Path,
        ignore_patterns: &[String],
        target_file: PathBuf,
    ) -> Result<Self> {
        let import_index = build_workspace_import_index(root, ignore_patterns)?;
        let symbols = SymbolIndex::build_targeted(root, &import_index.file_imports, target_file)?;
        Ok(Self {
            workspace_packages: import_index.workspace_packages,
            go_workspace: import_index.go_workspace,
            rust_workspace: import_index.rust_workspace,
            code_imports: import_index.file_imports,
            symbols,
        })
    }

    /// Build a code index and include symbol-reference maps for `kdb refs -s`.
    pub fn build_with_symbol_refs(root: &Path, ignore_patterns: &[String]) -> Result<Self> {
        let import_index = build_workspace_import_index(root, ignore_patterns)?;
        let symbols = SymbolIndex::build(root, &import_index.file_imports)?;
        Ok(Self {
            workspace_packages: import_index.workspace_packages,
            go_workspace: import_index.go_workspace,
            rust_workspace: import_index.rust_workspace,
            code_imports: import_index.file_imports,
            symbols,
        })
    }
}

impl WorkspaceIndex {
    /// Build a combined vault and code index for the workspace at `root`.
    pub fn build(root: &Path) -> Result<Self> {
        Self::build_with_ignores(root, &[])
    }

    /// Build a combined vault and code index with user-defined ignore patterns.
    pub fn build_with_ignores(root: &Path, ignore_patterns: &[String]) -> Result<Self> {
        let canonical = root
            .canonicalize()
            .with_context(|| format!("failed to canonicalize root {}", root.display()))?;
        let vault = VaultIndex::build_with_ignores(&canonical, ignore_patterns)?;
        let code = CodeIndex::build(&canonical, ignore_patterns)?;
        Ok(Self { vault, code })
    }

    /// Build a combined workspace index with targeted code symbol refs.
    ///
    /// Only extracts symbols and scans usages for files that import from `target_file`.
    pub fn build_for_target(
        root: &Path,
        ignore_patterns: &[String],
        target_file: &str,
    ) -> Result<Self> {
        let canonical = root
            .canonicalize()
            .with_context(|| format!("failed to canonicalize root {}", root.display()))?;
        let target = resolve_file_target(&canonical, target_file)?;
        let vault = VaultIndex::build_with_ignores(&canonical, ignore_patterns)?;
        let code = CodeIndex::build_for_target(&canonical, ignore_patterns, target)?;
        Ok(Self { vault, code })
    }

    /// Build a combined workspace index with code symbol refs enabled.
    pub fn build_with_symbol_refs(root: &Path, ignore_patterns: &[String]) -> Result<Self> {
        let canonical = root
            .canonicalize()
            .with_context(|| format!("failed to canonicalize root {}", root.display()))?;
        let vault = VaultIndex::build_with_ignores(&canonical, ignore_patterns)?;
        let code = CodeIndex::build_with_symbol_refs(&canonical, ignore_patterns)?;
        Ok(Self { vault, code })
    }
}

impl VaultIndex {
    /// Build an index of the entire vault by discovering and parsing all markdown files.
    ///
    /// Walks the directory tree under `root`, parses each `.md` file, and builds
    /// both the file map and inbound link graphs.
    pub fn build(root: &Path) -> Result<Self> {
        Self::build_with_ignores(root, &[])
    }

    /// Build an index of the entire vault with user-defined ignore patterns.
    ///
    /// Ignore patterns use glob syntax and are matched against slash-separated
    /// paths relative to `root`. Only discovers markdown files — no code scan.
    pub fn build_with_ignores(root: &Path, ignore_patterns: &[String]) -> Result<Self> {
        let root = root
            .canonicalize()
            .with_context(|| format!("failed to canonicalize root {}", root.display()))?;
        let ignore_set = build_ignore_globset(ignore_patterns)?;

        let discovered = discover_markdown_files(&root, &ignore_set)?;
        let parsed_files: Vec<_> = discovered
            .par_iter()
            .filter_map(|rel_path| {
                let abs_path = root.join(rel_path);
                let source = std::fs::read_to_string(&abs_path).ok()?;
                let parsed = parse_markdown(&source);
                Some((
                    rel_path.clone(),
                    FileEntry {
                        rel_path: rel_path.clone(),
                        abs_path,
                        headings: parsed.headings,
                        links: parsed.links,
                        prosaic_blocks: parsed.prosaic_blocks,
                    },
                ))
            })
            .collect();

        let mut files = BTreeMap::new();
        for (rel_path, entry) in parsed_files {
            files.insert(rel_path, entry);
        }

        let mut index = Self {
            root,
            ignore_set,
            files,
            file_inbound: HashMap::new(),
            heading_inbound: HashMap::new(),
        };
        index.populate_inbound();
        Ok(index)
    }

    /// Insert or replace a file in the index using provided source text.
    ///
    /// This supports incremental index updates from unsaved LSP document state.
    pub fn upsert_file(&mut self, rel_path: PathBuf, abs_path: PathBuf, source: &str) {
        if path_is_ignored(&self.ignore_set, &rel_path, false) {
            self.remove_file(&rel_path);
            return;
        }

        let parsed = parse_markdown(source);
        self.files.insert(
            rel_path.clone(),
            FileEntry {
                rel_path,
                abs_path,
                headings: parsed.headings,
                links: parsed.links,
                prosaic_blocks: parsed.prosaic_blocks,
            },
        );
        self.populate_inbound();
    }

    /// Reload a file from disk and update the index.
    ///
    /// If the file no longer exists or can't be read as UTF-8, it is removed
    /// from the index.
    pub fn reload_file(&mut self, rel_path: &Path) {
        if path_is_ignored(&self.ignore_set, rel_path, false) {
            self.remove_file(rel_path);
            return;
        }

        let abs_path = self.root.join(rel_path);
        let source = match std::fs::read_to_string(&abs_path) {
            Ok(source) => source,
            Err(_) => {
                self.remove_file(rel_path);
                return;
            }
        };

        let parsed = parse_markdown(&source);
        self.files.insert(
            rel_path.to_path_buf(),
            FileEntry {
                rel_path: rel_path.to_path_buf(),
                abs_path,
                headings: parsed.headings,
                links: parsed.links,
                prosaic_blocks: parsed.prosaic_blocks,
            },
        );
        self.populate_inbound();
    }

    /// Remove a file from the index.
    pub fn remove_file(&mut self, rel_path: &Path) {
        if self.files.remove(rel_path).is_some() {
            self.populate_inbound();
        }
    }

    /// Validate all links and embeds in the vault and return a report of broken
    /// references, broken embeds, and orphan files.
    pub fn check(&self) -> CheckReport {
        use crate::render::include::find_embeds;
        use crate::render::resolve::validate_embed_target;

        let mut report = CheckReport::default();

        for (source_file, file_entry) in &self.files {
            for link in &file_entry.links {
                if let Err(error) = self.resolve_link(source_file, link) {
                    report.broken_links.push(BrokenLink {
                        source_file: source_file.clone(),
                        line: link.line,
                        column: link.column,
                        raw: link.raw.clone(),
                        reason: error.message(),
                    });
                }
            }

            // Validate ![[]] embeds.
            let abs_path = self.root.join(source_file);
            if let Ok(source) = std::fs::read_to_string(&abs_path) {
                let lines: Vec<&str> = source.lines().collect();
                let embeds = find_embeds(&lines);
                for embed in &embeds {
                    if let Err(error) =
                        validate_embed_target(&self.root, source_file, &embed.directive)
                    {
                        report.broken_embeds.push(BrokenEmbed {
                            source_file: source_file.clone(),
                            line: embed.line + 1, // convert 0-based to 1-based
                            raw: format!(
                                "![[{}]]",
                                match &embed.directive.anchor {
                                    Some(a) => format!("{}#{}", embed.directive.file, a),
                                    None => embed.directive.file.clone(),
                                }
                            ),
                            reason: error.to_string(),
                        });
                    }
                }
            }
        }

        self.check_procedures(&mut report);

        for path in self.files.keys() {
            let inbound_from_other_files = self
                .file_inbound
                .get(path)
                .map(|refs| {
                    refs.iter()
                        .filter(|link_ref| link_ref.source_file != *path)
                        .count()
                })
                .unwrap_or(0);

            if inbound_from_other_files == 0 {
                report.orphans.push(path.clone());
            }
        }

        report.orphans.sort();
        report
    }

    /// Enforce the Prosaic composition resolution rules (spec §5.4) across every
    /// procedure-body prosaic block, appending unresolved `use`/`run` references
    /// to `report.procedure_errors`.
    fn check_procedures(&self, report: &mut CheckReport) {
        for (source_file, file_entry) in &self.files {
            let own_ids = procedure_ids_in(file_entry);

            for block in &file_entry.prosaic_blocks {
                // Only procedure bodies are checked; illustrative blocks in the
                // spec/templates sit under ordinary headings (§5.4, §5.1).
                if block.enclosing_procedure.is_none() {
                    continue;
                }

                let statements = prosaic::extract_statements(&block.body);
                let imported: HashSet<&str> = statements
                    .iter()
                    .filter_map(|stmt| match stmt {
                        prosaic::Statement::Use { id, .. } => Some(id.as_str()),
                        prosaic::Statement::Run { .. } => None,
                    })
                    .collect();

                for stmt in &statements {
                    match stmt {
                        prosaic::Statement::Use {
                            id,
                            path,
                            line,
                            column,
                            raw,
                        } => {
                            let reason = self.resolve_use(id, path);
                            if let Some(reason) = reason {
                                report.procedure_errors.push(ProcedureError {
                                    source_file: source_file.clone(),
                                    line: block.start_line + line,
                                    column: *column,
                                    raw: raw.clone(),
                                    reason,
                                });
                            }
                        }
                        prosaic::Statement::Run {
                            id,
                            line,
                            column,
                            raw,
                        } => {
                            if !own_ids.contains(id.as_str()) && !imported.contains(id.as_str()) {
                                report.procedure_errors.push(ProcedureError {
                                    source_file: source_file.clone(),
                                    line: block.start_line + line,
                                    column: *column,
                                    raw: raw.clone(),
                                    reason: format!(
                                        "run {id}: not defined in this file and no matching `use` import in this block"
                                    ),
                                });
                            }
                        }
                    }
                }
            }
        }
    }

    /// Validate a single `use <id> from <path>` import: the path must resolve to
    /// an indexed file, and that file must define a procedure with `id`. Returns
    /// `None` when the import resolves, or a failure reason.
    fn resolve_use(&self, id: &str, path: &str) -> Option<String> {
        let Some(target) = normalize_rel_path(Path::new(path)) else {
            return Some(format!("use target resolves outside root: {path}"));
        };
        let Some(target_entry) = self.files.get(&target) else {
            return Some(format!("use target file not found: {path}"));
        };
        if procedure_ids_in(target_entry).contains(id) {
            None
        } else {
            Some(format!("{path} does not define {id}"))
        }
    }

    /// Walk all links and record which files and headings they point to,
    /// building the inbound reference maps.
    fn populate_inbound(&mut self) {
        let mut file_inbound: HashMap<PathBuf, Vec<LinkRef>> = HashMap::new();
        let mut heading_inbound: HashMap<HeadingKey, Vec<LinkRef>> = HashMap::new();

        for (source_file, file_entry) in &self.files {
            for link in &file_entry.links {
                let Ok(target_file) = resolve_target_file(source_file, link) else {
                    continue;
                };

                if !self.files.contains_key(&target_file) {
                    continue;
                }

                let link_ref = LinkRef {
                    source_file: source_file.clone(),
                    source_line: link.line,
                    source_column: link.column,
                    raw: link.raw.clone(),
                };

                file_inbound
                    .entry(target_file.clone())
                    .or_default()
                    .push(link_ref.clone());

                if let Some(anchor) = link.target.anchor.as_deref().map(slug_anchor) {
                    let heading_exists = self.files.get(&target_file).is_some_and(|target| {
                        target
                            .headings
                            .iter()
                            .any(|heading| heading.anchor == anchor)
                    });

                    if heading_exists {
                        heading_inbound
                            .entry(HeadingKey {
                                file: target_file.clone(),
                                anchor,
                            })
                            .or_default()
                            .push(link_ref);
                    }
                }
            }
        }

        self.file_inbound = file_inbound;
        self.heading_inbound = heading_inbound;
    }

    /// Try to resolve a single link against the index. Returns `Err` with the
    /// reason if the target file or heading doesn't exist.
    fn resolve_link(
        &self,
        source_file: &Path,
        link: &Link,
    ) -> std::result::Result<(), ResolveError> {
        let target_file = resolve_target_file(source_file, link)?;
        if !self.files.contains_key(&target_file) {
            return Err(ResolveError::MissingFile(target_file));
        }

        if let Some(raw_anchor) = link.target.anchor.as_deref() {
            let anchor = slug_anchor(raw_anchor);
            let heading_exists = self.files.get(&target_file).is_some_and(|target| {
                target
                    .headings
                    .iter()
                    .any(|heading| heading.anchor == anchor)
            });

            if !heading_exists {
                return Err(ResolveError::MissingHeading {
                    file: target_file,
                    anchor,
                });
            }
        }

        Ok(())
    }
}

/// Why a link failed to resolve.
#[derive(Debug)]
enum ResolveError {
    OutsideRoot,
    MissingFile(PathBuf),
    MissingHeading { file: PathBuf, anchor: String },
}

impl ResolveError {
    fn message(&self) -> String {
        match self {
            Self::OutsideRoot => "target resolves outside root".to_string(),
            Self::MissingFile(path) => format!("target file not found: {}", path.display()),
            Self::MissingHeading { file, anchor } => {
                format!("target heading not found: {}#{}", file.display(), anchor)
            }
        }
    }
}

// ---------------------------------------------------------------------------
// File discovery
// ---------------------------------------------------------------------------

/// Collect the procedure IDs a file defines — every `## <ID> :: <Name>` heading
/// (spec §5.1).
fn procedure_ids_in(entry: &FileEntry) -> HashSet<String> {
    entry
        .headings
        .iter()
        .filter(|heading| heading.level == 2)
        .filter_map(|heading| prosaic::procedure_id_from_heading(&heading.title))
        .collect()
}

fn discover_markdown_files(root: &Path, ignore_set: &GlobSet) -> Result<Vec<PathBuf>> {
    let paths = discover_files(root, root, ignore_set)?;
    Ok(paths
        .into_iter()
        .filter(|rel| {
            rel.extension()
                .and_then(|ext| ext.to_str())
                .is_some_and(|ext| ext.eq_ignore_ascii_case("md"))
        })
        .collect())
}

// ---------------------------------------------------------------------------
// Link resolution
// ---------------------------------------------------------------------------

/// Resolve the target file path for a link, returning an error if it escapes
/// the vault root.
fn resolve_target_file(
    source_file: &Path,
    link: &Link,
) -> std::result::Result<PathBuf, ResolveError> {
    resolve_target_path(source_file, link.kind, &link.target).ok_or(ResolveError::OutsideRoot)
}

/// Resolve a link target to a relative path within the vault.
///
/// Handles both markdown and wikilink conventions:
/// - Markdown links are resolved relative to the source file's directory.
/// - Wikilinks auto-append `.md` if no extension is present.
/// - Returns `None` if the resolved path escapes the vault root.
pub fn resolve_target_path(
    source_file: &Path,
    kind: LinkKind,
    target: &LinkTarget,
) -> Option<PathBuf> {
    let candidate = match target.file.as_deref() {
        Some(raw_file) => {
            let mut rel = PathBuf::from(raw_file);
            if matches!(kind, LinkKind::Wikilink) && rel.extension().is_none() {
                rel.set_extension("md");
            }

            if rel.is_absolute() {
                return None;
            }

            if target.root_relative {
                rel
            } else {
                let base = source_file.parent().unwrap_or(Path::new(""));
                base.join(rel)
            }
        }
        None => source_file.to_path_buf(),
    };

    normalize_rel_path(&candidate)
}

/// Resolve a CLI file target (`<file>`) to a normalized path inside `root`.
///
/// Supports both absolute and root-relative paths.
pub fn resolve_file_target(root: &Path, file: &str) -> Result<PathBuf> {
    let path = Path::new(file);
    if path.is_absolute() {
        let canonical = path
            .canonicalize()
            .with_context(|| format!("failed to canonicalize {}", path.display()))?;
        let rel = canonical.strip_prefix(root).with_context(|| {
            format!(
                "target file {} is not inside kdb root {}",
                canonical.display(),
                root.display()
            )
        })?;
        return normalize_rel_path(rel)
            .with_context(|| format!("target path resolves outside root: {}", file));
    }

    normalize_rel_path(path).with_context(|| format!("target path resolves outside root: {file}"))
}
