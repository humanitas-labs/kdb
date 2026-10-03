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
    /// 1-based column.
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
    /// 1-based column.
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
    /// The link whose span contains the 1-based `line`/`column`, if any.
    pub fn link_at(&self, line: usize, column: usize) -> Option<&Link> {
        todo!("graph agent")
    }

    /// Links that are embeds.
    pub fn embeds(&self) -> impl Iterator<Item = &Link> {
        self.links.iter().filter(|l| l.embed)
    }
}

/// Where a link originates; used for inbound lookups and `refs`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LinkRef {
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

/// A broken link or embed. `reason` matches v1 wording, e.g.
/// `target file not found: a/b.md` or `heading not found: #foo`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Problem {
    pub kind: ProblemKind,
    pub file: PathBuf,
    pub line: usize,
    pub column: usize,
    pub raw: String,
    pub reason: String,
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
    root: PathBuf,
    docs: BTreeMap<PathBuf, Doc>,
    file_inbound: HashMap<PathBuf, Vec<LinkRef>>,
    heading_inbound: HashMap<HeadingKey, Vec<LinkRef>>,
}

impl Graph {
    /// Walk the workspace and parse every markdown file.
    pub fn build(ws: &Workspace) -> Result<Self> {
        todo!("graph agent")
    }

    /// Build from a pre-read set of `(rel path, source)` pairs; the LSP and tests use this.
    pub fn from_sources(root: &Path, sources: impl IntoIterator<Item = (PathBuf, String)>) -> Self {
        todo!("graph agent")
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Insert or replace one document from its source text and refresh inbound indexes.
    pub fn upsert(&mut self, rel: &Path, source: &str) {
        todo!("graph agent")
    }

    /// Remove a document (file deleted) and refresh inbound indexes.
    pub fn remove(&mut self, rel: &Path) {
        todo!("graph agent")
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

    /// Resolve a link target relative to the document at `from`.
    ///
    /// Markdown targets resolve relative to `from`'s directory; `root_relative`
    /// targets from the root; wikilinks without a directory resolve by basename
    /// anywhere in the graph (`.md` optional), then relative, then root.
    pub fn resolve(&self, from: &Path, target: &LinkTarget) -> Resolution<'_> {
        todo!("graph agent")
    }

    /// Broken links/embeds in one document, in source order. The LSP publishes this per file.
    pub fn check_file(&self, rel: &Path) -> Vec<Problem> {
        todo!("graph agent")
    }

    /// Full workspace report: every document's problems plus orphans.
    pub fn check(&self) -> CheckReport {
        todo!("graph agent")
    }
}
