//! The markdown link graph: every `.md` file in the workspace, its headings,
//! and its internal links and embeds, with inbound indexes for `check`,
//! `refs`, and the LSP.
//!
//! The graph lives in memory and is rebuilt from disk on each CLI run; the
//! LSP keeps one alive and patches it with [`Graph::upsert`] / [`Graph::remove`].
//!
//! The type and method signatures below are the contract the LSP builds
//! against. Change them only with the LSP in view.

pub mod check;
pub mod cli;
pub mod outline;
pub mod parse;
pub mod refs;
pub mod transclude;

use std::collections::{BTreeMap, HashMap};
use std::ops::Range;
use std::path::{Path, PathBuf};

use anyhow::Result;
use serde::Serialize;

use crate::workspace::Workspace;

/// Whether a link uses standard markdown or wikilink syntax.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum LinkKind {
    /// `[text](path.md#anchor)`
    Markdown,
    /// `[[path#anchor]]` or `![[path#anchor]]`
    Wikilink,
}

/// The parsed destination of a link, split into file and anchor.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LinkTarget {
    /// Target file as written (e.g. `"hooks.md"`), `None` for same-file `#anchor` links.
    pub file: Option<String>,
    /// Heading anchor slug, `None` for file-only links.
    pub anchor: Option<String>,
    /// `true` when `file` is root-relative (`kdb://` scheme) instead of relative to the source file.
    pub root_relative: bool,
}

/// A heading in a document.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Heading {
    pub title: String,
    /// GitHub-style slug, deduplicated with `-1`, `-2` suffixes within the file.
    pub anchor: String,
    /// 1 for `#`, 2 for `##`, …
    pub level: u8,
    /// 1-based line.
    pub line: usize,
    /// 1-based byte offset within the line (tree-sitter convention).
    pub column: usize,
}

/// An internal link or embed in a document. External URLs are never recorded.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Link {
    pub kind: LinkKind,
    /// `true` for `![[target]]` (and `![text](target.md)`) transclusions.
    pub embed: bool,
    /// The raw target text as written, e.g. `"hooks.md#useEffect"` or `"[[hooks#useEffect]]"`.
    pub raw: String,
    pub target: LinkTarget,
    /// 1-based line.
    pub line: usize,
    /// 1-based byte offset within the line (tree-sitter convention). For `![[x]]` embeds
    /// this points at the `[[`, as v1 did; `span` still starts at the `!`.
    pub column: usize,
    /// Byte range of the whole link in the document source (for LSP ranges and completion).
    pub span: Range<usize>,
}

/// One parsed markdown document.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Doc {
    /// Root-relative path.
    pub rel: PathBuf,
    pub headings: Vec<Heading>,
    pub links: Vec<Link>,
}

impl Doc {
    /// The link whose span contains the 1-based `line` and 1-based byte `column`, if any.
    pub fn link_at(&self, line: usize, column: usize) -> Option<&Link> {
        self.links.iter().find(|l| l.line == line && l.column <= column && column < l.column + (l.span.end - l.span.start))
    }
}

/// Where a link originates; used for inbound lookups and `refs`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LinkRef {
    /// Serialized as `source_file` to keep v1's `refs --json` shape.
    #[serde(rename = "source_file")]
    pub source: PathBuf,
    pub line: usize,
    pub column: usize,
    pub raw: String,
}

/// A heading identified across the workspace.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct HeadingKey {
    pub file: PathBuf,
    pub anchor: String,
}

/// Result of resolving a link from a given source document.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Resolution<'g> {
    /// Target file exists; `heading` is set when an anchor was given and found.
    Ok { file: PathBuf, heading: Option<&'g Heading> },
    /// The resolved file path does not exist in the graph.
    MissingFile { file: PathBuf },
    /// The file exists but has no heading with this anchor.
    MissingAnchor { file: PathBuf, anchor: String },
    /// The target could not be turned into a workspace path (escapes root, etc.).
    Invalid { reason: String },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum ProblemKind {
    BrokenLink,
    BrokenEmbed,
}

/// A broken link or embed. `reason` matches v1 wording exactly:
/// `target file not found: a/b.md`, `target heading not found: a/b.md#foo`,
/// `target resolves outside root`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Problem {
    pub kind: ProblemKind,
    pub file: PathBuf,
    /// 1-based line.
    pub line: usize,
    /// 1-based byte offset within the line (tree-sitter convention).
    pub column: usize,
    pub raw: String,
    pub reason: String,
    /// Byte range of the offending link in the document source (copied from [`Link::span`]).
    pub span: Range<usize>,
}

/// Everything `check` reports.
#[derive(Debug, Clone, Default)]
pub struct CheckReport {
    pub problems: Vec<Problem>,
    /// Files with no inbound links from other files, sorted.
    pub orphans: Vec<PathBuf>,
}

impl CheckReport {
    pub fn has_errors(&self) -> bool {
        !self.problems.is_empty()
    }
}

/// The in-memory link graph.
#[derive(Debug, Clone)]
pub struct Graph {
    docs: BTreeMap<PathBuf, Doc>,
    file_inbound: HashMap<PathBuf, Vec<LinkRef>>,
    heading_inbound: HashMap<HeadingKey, Vec<LinkRef>>,
}

impl Graph {
    /// Walk the workspace and parse every markdown file.
    pub fn build(ws: &Workspace) -> Result<Self> {
        let paths = ws.walk_markdown()?;
        let threads = std::thread::available_parallelism().map(usize::from).unwrap_or(1).min(paths.len().max(1));
        let chunk = paths.len().div_ceil(threads);
        let parsed: Vec<Doc> = std::thread::scope(|scope| {
            let workers: Vec<_> = paths
                .chunks(chunk.max(1))
                .map(|chunk| {
                    scope.spawn(move || {
                        chunk
                            .iter()
                            .filter_map(|rel| std::fs::read_to_string(ws.abs(rel)).ok().map(|src| parse_doc(rel.clone(), &src)))
                            .collect::<Vec<_>>()
                    })
                })
                .collect();
            workers.into_iter().flat_map(|w| w.join().expect("parse worker")).collect()
        });
        Ok(Self::from_docs(parsed.into_iter().map(|d| (d.rel.clone(), d)).collect()))
    }

    fn from_docs(docs: BTreeMap<PathBuf, Doc>) -> Self {
        let mut graph = Self { docs, file_inbound: HashMap::new(), heading_inbound: HashMap::new() };
        graph.rebuild_inbound();
        graph
    }

    /// Insert or replace one document from its source text and refresh inbound indexes.
    pub fn upsert(&mut self, rel: &Path, source: &str) {
        self.docs.insert(rel.to_path_buf(), parse_doc(rel.to_path_buf(), source));
        self.rebuild_inbound();
    }

    /// Remove a document (file deleted) and refresh inbound indexes.
    pub fn remove(&mut self, rel: &Path) {
        if self.docs.remove(rel).is_some() {
            self.rebuild_inbound();
        }
    }

    pub fn doc(&self, rel: &Path) -> Option<&Doc> {
        self.docs.get(rel)
    }

    pub fn docs(&self) -> impl Iterator<Item = &Doc> {
        self.docs.values()
    }

    /// Inbound links to a file (from other files or itself).
    pub fn inbound(&self, rel: &Path) -> &[LinkRef] {
        self.file_inbound.get(rel).map(Vec::as_slice).unwrap_or(&[])
    }

    /// Inbound links to a specific heading.
    pub fn inbound_heading(&self, key: &HeadingKey) -> &[LinkRef] {
        self.heading_inbound.get(key).map(Vec::as_slice).unwrap_or(&[])
    }

    /// Resolve a link target relative to the document at `from` (v1 `resolve_target_path` rules).
    ///
    /// Targets resolve relative to `from`'s directory, `root_relative` targets from the
    /// root, `None` file means `from` itself. A target without an extension gets `.md`
    /// (wikilink convention; markdown targets always carry `.md` after parsing). Anchors
    /// are slugged with [`parse::slug_anchor`] before heading lookup. There is no
    /// basename lookup: v1 had none and `check` must diff clean against it.
    pub fn resolve(&self, from: &Path, target: &LinkTarget) -> Resolution<'_> {
        let Some(file) = resolve_target_path(from, target) else {
            return Resolution::Invalid { reason: "target resolves outside root".to_string() };
        };
        let Some(doc) = self.docs.get(&file) else {
            return Resolution::MissingFile { file };
        };
        match target.anchor.as_deref().map(parse::slug_anchor) {
            None => Resolution::Ok { file, heading: None },
            Some(anchor) => match doc.headings.iter().find(|h| h.anchor == anchor) {
                Some(h) => Resolution::Ok { file, heading: Some(h) },
                None => Resolution::MissingAnchor { file, anchor },
            },
        }
    }

    /// Broken links/embeds in one document, in source order. The LSP publishes this per file.
    pub fn check_file(&self, rel: &Path) -> Vec<Problem> {
        let Some(doc) = self.docs.get(rel) else { return Vec::new() };
        doc.links
            .iter()
            .filter_map(|link| {
                let reason = match self.resolve(rel, &link.target) {
                    Resolution::Ok { .. } => return None,
                    Resolution::Invalid { reason } => reason,
                    Resolution::MissingFile { file } => format!("target file not found: {}", file.display()),
                    Resolution::MissingAnchor { file, anchor } => {
                        format!("target heading not found: {}#{anchor}", file.display())
                    }
                };
                Some(Problem {
                    kind: if link.embed { ProblemKind::BrokenEmbed } else { ProblemKind::BrokenLink },
                    file: rel.to_path_buf(),
                    line: link.line,
                    column: link.column,
                    raw: link.raw.clone(),
                    reason,
                    span: link.span.clone(),
                })
            })
            .collect()
    }

    /// Full workspace report: every document's problems plus orphans.
    pub fn check(&self) -> CheckReport {
        let problems = self.docs.keys().flat_map(|rel| self.check_file(rel)).collect();
        let orphans = self
            .docs
            .keys()
            .filter(|rel| !self.inbound(rel).iter().any(|r| r.source != **rel))
            .cloned()
            .collect();
        CheckReport { problems, orphans }
    }

    /// Recompute the inbound indexes from scratch. Only links whose target file exists
    /// are recorded; heading entries only when the heading exists.
    fn rebuild_inbound(&mut self) {
        let mut file_inbound: HashMap<PathBuf, Vec<LinkRef>> = HashMap::new();
        let mut heading_inbound: HashMap<HeadingKey, Vec<LinkRef>> = HashMap::new();
        for (source, doc) in &self.docs {
            for link in &doc.links {
                let (file, heading) = match self.resolve(source, &link.target) {
                    Resolution::Ok { file, heading } => (file, heading),
                    Resolution::MissingAnchor { file, .. } => (file, None),
                    _ => continue,
                };
                let link_ref = LinkRef { source: source.clone(), line: link.line, column: link.column, raw: link.raw.clone() };
                if let Some(h) = heading {
                    heading_inbound
                        .entry(HeadingKey { file: file.clone(), anchor: h.anchor.clone() })
                        .or_default()
                        .push(link_ref.clone());
                }
                file_inbound.entry(file).or_default().push(link_ref);
            }
        }
        self.file_inbound = file_inbound;
        self.heading_inbound = heading_inbound;
    }
}

fn parse_doc(rel: PathBuf, source: &str) -> Doc {
    let parsed = parse::parse(source);
    Doc { rel, headings: parsed.headings, links: parsed.links }
}

/// Root-relative path a target points at, before existence checks (v1 `resolve_target_path`).
/// `None` when the path is absolute or escapes the root.
pub fn resolve_target_path(from: &Path, target: &LinkTarget) -> Option<PathBuf> {
    let candidate = match target.file.as_deref() {
        None => from.to_path_buf(),
        Some(raw) => {
            let mut rel = PathBuf::from(raw);
            if rel.extension().is_none() {
                rel.set_extension("md");
            }
            if rel.is_absolute() {
                return None;
            }
            if target.root_relative { rel } else { from.parent().unwrap_or(Path::new("")).join(rel) }
        }
    };
    crate::workspace::normalize_rel(&candidate)
}
