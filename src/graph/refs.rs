//! `refs`: inbound links to a file or heading, v1 formats.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};

use super::parse::{parse_markdown_target, slug_anchor};
use super::{Graph, HeadingKey, LinkRef};

/// A `refs` target: root-relative file plus optional slugged anchor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Target {
    pub file: String,
    pub anchor: Option<String>,
}

/// Parse `<file.md>` or `<file.md>#<heading>`; the anchor is slugged.
pub fn parse_target(raw: &str) -> Result<Target> {
    let msg = || format!("invalid refs target `{raw}` (expected <file.md> or <file.md>#<heading>)");
    let target = parse_markdown_target(raw).with_context(msg)?;
    let file = target.file.with_context(msg)?;
    Ok(Target { file, anchor: target.anchor.map(|a| slug_anchor(&a)) })
}

/// Inbound references sorted by source, line, column, raw. Errors when the file is not
/// in the graph or the heading does not exist.
pub fn collect(graph: &Graph, file: &Path, anchor: Option<&str>) -> Result<Vec<LinkRef>> {
    let Some(doc) = graph.doc(file) else {
        bail!("target file is not an indexed markdown file: {}", file.display());
    };
    let mut refs: Vec<LinkRef> = match anchor {
        Some(anchor) => {
            if !doc.headings.iter().any(|h| h.anchor == anchor) {
                bail!("target heading not found: {}#{anchor}", file.display());
            }
            graph.inbound_heading(&HeadingKey { file: file.to_path_buf(), anchor: anchor.to_string() }).to_vec()
        }
        None => graph.inbound(file).to_vec(),
    };
    refs.sort_by(|a, b| {
        a.source.cmp(&b.source).then(a.line.cmp(&b.line)).then(a.column.cmp(&b.column)).then(a.raw.cmp(&b.raw))
    });
    Ok(refs)
}

/// `source:line:col  raw` per reference, or `(no references)`.
pub fn render_text(refs: &[LinkRef]) -> String {
    if refs.is_empty() {
        return "(no references)\n".to_string();
    }
    refs.iter().fold(String::new(), |mut out, r| {
        let _ = writeln!(out, "{}:{}:{}  {}", r.source.display(), r.line, r.column, r.raw);
        out
    })
}

/// Unique source files in first-seen order.
pub fn files(refs: &[LinkRef]) -> Vec<PathBuf> {
    let mut seen: Vec<PathBuf> = Vec::new();
    for r in refs {
        if !seen.contains(&r.source) {
            seen.push(r.source.clone());
        }
    }
    seen
}
