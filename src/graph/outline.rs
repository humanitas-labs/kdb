//! `outline`: heading rows for files, and section bodies for `-s` selectors. v1 formats.

use std::fmt::Write as _;
use std::path::Path;

use anyhow::{Result, bail};
use serde::Serialize;

use super::parse::{self, Parsed, section_byte_bounds, section_line_bounds};

/// One heading row of `outline` (v1 `SymbolRow`, markdown fields only).
#[derive(Debug, Clone, Serialize)]
pub struct Row {
    #[serde(skip)]
    pub display: String,
    pub kind: &'static str,
    pub name: String,
    pub line: usize,
    pub level: u8,
    pub anchor: String,
    pub public: bool,
}

/// One section body of `outline -s` (v1 `SymbolBodyRow`).
#[derive(Debug, Clone, Serialize)]
pub struct BodyRow {
    pub file: String,
    pub kind: &'static str,
    pub display_kind: String,
    pub name: String,
    pub line: usize,
    pub end_line: usize,
    pub public: bool,
    pub body: String,
}

pub fn rows(parsed: &Parsed) -> Vec<Row> {
    parsed
        .headings
        .iter()
        .map(|h| Row {
            display: format!("{} {}", "#".repeat(usize::from(h.level)), h.title),
            kind: "heading",
            name: h.title.clone(),
            line: h.line,
            level: h.level,
            anchor: h.anchor.clone(),
            public: true,
        })
        .collect()
}

/// The section selected by `selector` (`#slug` or `slug`, case-insensitive), if the heading exists.
pub fn body_row(rel: &Path, source: &str, parsed: &Parsed, selector: &str) -> Result<Option<BodyRow>> {
    let wanted = selector.trim().trim_start_matches('#').trim().to_ascii_lowercase();
    if wanted.is_empty() {
        bail!("invalid symbol selector: {selector}");
    }
    let Some(heading) = parsed.headings.iter().find(|h| h.anchor.eq_ignore_ascii_case(&wanted)) else {
        return Ok(None);
    };
    let (start, end) = section_byte_bounds(source, &parsed.headings, Some(&heading.anchor))
        .ok_or_else(|| anyhow::anyhow!("failed to extract body for symbol `{selector}` in {}", rel.display()))?;
    let (_, next) = section_line_bounds(&parsed.headings, Some(&heading.anchor)).expect("bounds exist");
    Ok(Some(BodyRow {
        file: rel.to_string_lossy().replace('\\', "/"),
        kind: "heading",
        display_kind: "#".repeat(usize::from(heading.level)),
        name: heading.title.clone(),
        line: heading.line,
        end_line: next.unwrap_or_else(|| source.lines().count()).max(heading.line),
        public: true,
        body: source[start..end].to_string(),
    }))
}

/// Rows for one file (`display  L<line>`), or `(no symbols)`.
pub fn render_rows(rows: &[Row]) -> String {
    if rows.is_empty() {
        return "(no symbols)\n".to_string();
    }
    let width = rows.iter().map(|r| r.display.len()).max().unwrap_or(0);
    rows.iter().fold(String::new(), |mut out, r| {
        let _ = writeln!(out, "{:<width$}  L{}", r.display, r.line);
        out
    })
}

/// Rows for several files, each under a `── path` header, blank line between files.
pub fn render_multi(files: &[(std::path::PathBuf, Vec<Row>)]) -> String {
    if files.iter().all(|(_, rows)| rows.is_empty()) {
        return "(no symbols)\n".to_string();
    }
    let width = files.iter().flat_map(|(_, rows)| rows.iter().map(|r| r.display.len())).max().unwrap_or(0);
    let mut out = String::new();
    for (i, (path, rows)) in files.iter().enumerate() {
        if i > 0 {
            out.push('\n');
        }
        let _ = writeln!(out, "── {}", path.display());
        for r in rows {
            let _ = writeln!(out, "{:<width$}  L{}", r.display, r.line);
        }
    }
    out
}

/// Section bodies with a right-aligned line-number gutter, blank line between sections.
pub fn render_bodies(rows: &[BodyRow]) -> String {
    if rows.is_empty() {
        return "(no symbols)\n".to_string();
    }
    let max_line = rows.iter().map(|r| r.end_line).max().unwrap_or(0).max(1);
    let gutter = max_line.ilog10() as usize + 1;
    let mut out = String::new();
    for (i, row) in rows.iter().enumerate() {
        if i > 0 {
            out.push('\n');
        }
        for (offset, line) in row.body.lines().enumerate() {
            let _ = writeln!(out, "{:>gutter$} | {line}", row.line + offset);
        }
    }
    out
}

/// Parse a file from disk in isolation (no graph build; works on ignored files too).
pub fn parse_file(abs: &Path) -> Result<(String, Parsed)> {
    let source = std::fs::read_to_string(abs).map_err(|e| anyhow::anyhow!("failed to read {}: {e}", abs.display()))?;
    let parsed = parse::parse(&source);
    Ok((source, parsed))
}
