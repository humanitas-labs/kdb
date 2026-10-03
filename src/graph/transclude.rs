//! `render <file>`: recursive resolution of standalone `![[file#heading]]` embeds.
//!
//! Reads from disk (not the graph) so ignored files render too. An embed is resolved
//! only when it is the sole content of its line; embeds inside code blocks are left alone.
//! Error wording matches v1.

use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};

use super::parse::{self, section_byte_bounds, slug_anchor};
use super::{LinkTarget, resolve_target_path};

const MAX_DEPTH: usize = 10;

#[derive(Debug)]
pub enum IncludeError {
    FileNotFound(PathBuf),
    HeadingNotFound { file: PathBuf, anchor: String },
    CycleDetected { chain: Vec<String> },
    MaxDepthExceeded(usize),
    ReadError { path: PathBuf, source: std::io::Error },
}

impl fmt::Display for IncludeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::FileNotFound(p) => write!(f, "include target file not found: {}", p.display()),
            Self::HeadingNotFound { file, anchor } => {
                write!(f, "include target heading not found: {}#{anchor}", file.display())
            }
            Self::CycleDetected { chain } => write!(f, "include cycle detected: {}", chain.join(" -> ")),
            Self::MaxDepthExceeded(d) => write!(f, "include depth exceeded maximum of {d}"),
            Self::ReadError { path, source } => write!(f, "failed to read {}: {source}", path.display()),
        }
    }
}

impl std::error::Error for IncludeError {}

/// Render the file at root-relative `rel` with all embeds resolved.
pub fn render_file(root: &Path, rel: &Path) -> Result<String, IncludeError> {
    let content = read(root, rel)?;
    let mut chain = vec![rel.display().to_string()];
    render_content(root, rel, &content, 0, &mut chain)
}

/// Resolve the embeds in `content` (which lives at `rel`); `chain` is the include path so far.
pub fn render_content(
    root: &Path,
    rel: &Path,
    content: &str,
    depth: usize,
    chain: &mut Vec<String>,
) -> Result<String, IncludeError> {
    if depth > MAX_DEPTH {
        return Err(IncludeError::MaxDepthExceeded(depth));
    }
    let lines: Vec<&str> = content.lines().collect();
    let embeds = standalone_embeds(content, &lines);
    if embeds.is_empty() {
        return Ok(content.to_string());
    }
    let mut out: Vec<String> = lines.iter().map(|l| l.to_string()).collect();
    for (line_idx, target) in embeds.into_iter().rev() {
        let resolved = resolve_embed(root, rel, &target, depth, chain)?;
        let replacement = resolved.trim_end_matches('\n').lines().map(String::from);
        out.splice(line_idx..=line_idx, replacement);
    }
    let mut result = out.join("\n");
    if content.ends_with('\n') && !result.ends_with('\n') {
        result.push('\n');
    }
    Ok(result)
}

/// `(0-based line, target)` for each embed that is the whole trimmed line, outside code.
fn standalone_embeds(content: &str, lines: &[&str]) -> Vec<(usize, LinkTarget)> {
    parse::parse(content)
        .links
        .into_iter()
        .filter(|l| l.embed && l.target.file.is_some())
        .filter(|l| lines.get(l.line - 1).is_some_and(|line| line.trim() == format!("!{}", l.raw)))
        .map(|l| (l.line - 1, l.target))
        .collect()
}

fn resolve_embed(
    root: &Path,
    rel: &Path,
    target: &LinkTarget,
    depth: usize,
    chain: &mut Vec<String>,
) -> Result<String, IncludeError> {
    let candidate = resolve_target_path(rel, target)
        .ok_or_else(|| IncludeError::FileNotFound(PathBuf::from(target.file.clone().unwrap_or_default())))?;
    if !root.join(&candidate).is_file() {
        return Err(IncludeError::FileNotFound(candidate));
    }
    let key = match &target.anchor {
        Some(a) => format!("{}#{a}", candidate.display()),
        None => candidate.display().to_string(),
    };
    if chain.contains(&key) {
        let mut cycle = chain.clone();
        cycle.push(key);
        return Err(IncludeError::CycleDetected { chain: cycle });
    }
    chain.push(key);
    let section = extract_section(root, &candidate, target.anchor.as_deref())?;
    let rendered = render_content(root, &candidate, &section, depth + 1, chain)?;
    chain.pop();
    Ok(rendered)
}

/// The whole file, or the section under `anchor` (heading line through the line before
/// the next heading of equal or higher level).
pub fn extract_section(root: &Path, rel: &Path, anchor: Option<&str>) -> Result<String, IncludeError> {
    let content = read(root, rel)?;
    let Some(anchor) = anchor else { return Ok(content) };
    let parsed = parse::parse(&content);
    let (start, end) = section_byte_bounds(&content, &parsed.headings, Some(&slug_anchor(anchor)))
        .ok_or_else(|| IncludeError::HeadingNotFound { file: rel.to_path_buf(), anchor: anchor.to_string() })?;
    Ok(content[start..end].to_string())
}

fn read(root: &Path, rel: &Path) -> Result<String, IncludeError> {
    fs::read_to_string(root.join(rel)).map_err(|source| IncludeError::ReadError { path: rel.to_path_buf(), source })
}
