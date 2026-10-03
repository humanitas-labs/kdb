//! clap argument types and entry points for `check`, `outline`, `refs`, and `render <file>`.

use std::env;
use std::io::Write as _;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use clap::Args;

use super::{Graph, check, outline, refs, transclude};
use crate::workspace::Workspace;

#[derive(Args, Debug)]
pub struct CheckArgs {
    /// Print each orphan file path
    #[arg(long)]
    pub orphans: bool,
    /// Optional file or directory path to scope check output to
    pub path: Option<PathBuf>,
}

#[derive(Args, Debug)]
pub struct OutlineArgs {
    /// File or directory paths to inspect (accepts multiple)
    #[arg(required = true)]
    pub paths: Vec<PathBuf>,
    /// Select heading sections by anchor slug (single file only)
    #[arg(short = 's', long = "symbol", num_args = 1..)]
    pub symbols: Vec<String>,
    /// Emit structured JSON output
    #[arg(long)]
    pub json: bool,
}

#[derive(Args, Debug)]
pub struct RefsArgs {
    /// Markdown target (e.g. `notes.md#getting-started`)
    pub target: String,
    /// Emit structured JSON output
    #[arg(long)]
    pub json: bool,
    /// Print only the number of inbound references
    #[arg(long)]
    pub count: bool,
    /// Print only unique file paths containing references
    #[arg(short = 'l', long = "files")]
    pub files: bool,
}

/// `check [--orphans] [PATH]`: exits 1 when there are broken links or embeds.
pub fn check(ws: &Workspace, args: CheckArgs) -> Result<()> {
    let graph = Graph::build(ws)?;
    let mut report = graph.check();
    if let Some(path) = &args.path {
        let abs = absolute(path)?;
        let rel = ws.rel(&abs).with_context(|| format!("path {} is not inside kdb root {}", abs.display(), ws.root.display()))?;
        check::scope(&mut report, &rel, abs.is_dir());
    }
    print(&check::render(&report, args.orphans))?;
    if report.has_errors() {
        std::process::exit(1);
    }
    Ok(())
}

pub fn outline(ws: &Workspace, args: OutlineArgs) -> Result<()> {
    let files = expand_paths(ws, &args.paths)?;
    if files.is_empty() {
        bail!("no markdown files found in given paths");
    }
    if args.symbols.is_empty() {
        let mut all = Vec::new();
        for rel in &files {
            let (_, parsed) = outline::parse_file(&ws.abs(rel))?;
            all.push((rel.clone(), outline::rows(&parsed)));
        }
        if args.json {
            let flat: Vec<_> = all.iter().flat_map(|(_, rows)| rows).collect();
            return print(&format!("{}\n", serde_json::to_string_pretty(&flat)?));
        }
        return print(&if all.len() > 1 { outline::render_multi(&all) } else { outline::render_rows(&all[0].1) });
    }
    if files.len() > 1 {
        bail!("-s/--symbol requires a single definition file, got {} files", files.len());
    }
    let rel = &files[0];
    let (source, parsed) = outline::parse_file(&ws.abs(rel))?;
    let mut rows = Vec::new();
    for selector in &args.symbols {
        match outline::body_row(rel, &source, &parsed, selector)? {
            Some(row) => rows.push(row),
            None => bail!("symbol not found: {selector} in {}", rel.display()),
        }
    }
    if args.json {
        return print(&format!("{}\n", serde_json::to_string_pretty(&rows)?));
    }
    print(&outline::render_bodies(&rows))
}

pub fn refs(ws: &Workspace, args: RefsArgs) -> Result<()> {
    let target = refs::parse_target(&args.target)?;
    let file = resolve_file_target(ws, &target.file)?;
    let graph = Graph::build(ws)?;
    let inbound = refs::collect(&graph, &file, target.anchor.as_deref())?;
    if args.count {
        return print(&format!("{}\n", inbound.len()));
    }
    if args.files {
        return print(&refs::files(&inbound).iter().map(|p| format!("{}\n", p.display())).collect::<String>());
    }
    if args.json {
        return print(&format!("{}\n", serde_json::to_string_pretty(&inbound)?));
    }
    print(&refs::render_text(&inbound))
}

/// `render <file>`: resolve `![[]]` embeds and print the result to stdout.
pub fn render_file(ws: &Workspace, file: &Path) -> Result<()> {
    let abs = absolute(file)?;
    let rel = ws.rel(&abs).with_context(|| format!("path {} is not inside kdb root {}", abs.display(), ws.root.display()))?;
    let out = transclude::render_file(&ws.root, &rel).with_context(|| format!("failed to render {}", rel.display()))?;
    print(&out)
}

/// Markdown files for each path: a file as given (even if ignored), a directory walked
/// with the workspace ignore rules. Sorted and deduplicated by root-relative path.
fn expand_paths(ws: &Workspace, paths: &[PathBuf]) -> Result<Vec<PathBuf>> {
    let mut out = Vec::new();
    for path in paths {
        let abs = absolute(path)?;
        if !abs.exists() {
            bail!("path does not exist: {}", abs.display());
        }
        let rel = ws.rel(&abs).with_context(|| format!("path {} is not inside kdb root {}", abs.display(), ws.root.display()))?;
        if abs.is_dir() {
            out.extend(ws.walk_markdown_under(&abs)?);
        } else {
            out.push(rel);
        }
    }
    out.sort();
    out.dedup();
    Ok(out)
}

/// A CLI `<file>` for `refs`: absolute paths must be inside the root; relative paths are root-relative.
fn resolve_file_target(ws: &Workspace, file: &str) -> Result<PathBuf> {
    let path = Path::new(file);
    if path.is_absolute() {
        return ws.rel(path).with_context(|| format!("target file {} is not inside kdb root {}", path.display(), ws.root.display()));
    }
    crate::workspace::normalize_rel(path).with_context(|| format!("target path resolves outside root: {file}"))
}

fn absolute(path: &Path) -> Result<PathBuf> {
    if path.is_absolute() {
        return Ok(path.to_path_buf());
    }
    Ok(env::current_dir().context("failed to read current directory")?.join(path))
}

/// Write to stdout, treating a closed pipe as success.
fn print(text: &str) -> Result<()> {
    let mut stdout = std::io::stdout().lock();
    match stdout.write_all(text.as_bytes()).and_then(|()| stdout.flush()) {
        Err(e) if e.kind() == std::io::ErrorKind::BrokenPipe => Ok(()),
        other => Ok(other?),
    }
}
