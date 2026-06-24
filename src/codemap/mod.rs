//! `kdb codemap` — deterministic index/freshness layer over colocated
//! `CODEMAP.md` domain maps.
//!
//! Colocated `CODEMAP.md` files are the source of truth (authored separately);
//! this module is the read-only index over them: discover the maps, parse their
//! frontmatter, check coverage + staleness, and render a derived index. Pure
//! derivation — no LLM, no persisted state.

pub mod check;
pub mod discover;
pub mod frontmatter;
pub mod git;
pub mod render;

use std::path::{Path, PathBuf};

use anyhow::{bail, Result};
use serde::Serialize;

use crate::cmd::CmdContext;

// --------------------------------------------
// projects/kdb/src/codemap/mod.rs
//
// pub mod check                             L9
// pub mod discover                         L10
// pub mod frontmatter                      L11
// pub mod git                              L12
// pub mod render                           L13
// pub struct CodemapDoc                    L45
// pub struct ParseProblem                  L65
// pub fn resolve_scope()                   L79
// fn enclosing_repo()                      L95
// pub fn ls()                             L110
// fn render_ls_table()                    L131
// mod tests                               L166
// fn doc()                                L170
// fn empty_table()                        L183
// fn table_lists_domains_and_updated()    L188
// --------------------------------------------

/// A parsed `CODEMAP.md` and the subtree it covers. All paths are relative to
/// the repo root (the resolved scope), keeping maps portable with the codebase.
#[derive(Debug, Clone, Serialize)]
pub struct CodemapDoc {
    /// Repo-relative path to the `CODEMAP.md` file itself.
    pub file: PathBuf,
    /// Domain name (required frontmatter field).
    pub domain: String,
    /// Originating repo, if recorded.
    pub repo: Option<String>,
    /// Repo-relative subtree the map covers. Defaults to the map's directory.
    pub root: PathBuf,
    /// Owning person/team, if recorded.
    pub owner: Option<String>,
    /// Authored last-updated date (`YYYY.MM.DD`), as written.
    pub updated: Option<String>,
    /// Short commit SHA the map was written against, if pinned.
    pub commit: Option<String>,
}

/// A `CODEMAP.md` whose frontmatter could not be parsed. Surfaced by `check`,
/// never a hard error in `ls`.
#[derive(Debug, Clone, Serialize)]
pub struct ParseProblem {
    /// Repo-relative path to the offending `CODEMAP.md`.
    pub file: PathBuf,
    /// Human-readable description of what went wrong.
    pub message: String,
}

/// Resolve the discovery scope for a codemap command.
///
/// With an explicit `path`, that path is the scope. Without one, the scope is
/// the nearest enclosing git repository of the current directory — codemaps are
/// a per-repo concern, and the kdb workspace may span many repos. The walk
/// never ascends above the workspace root; if no repo is found it falls back to
/// the current directory.
pub fn resolve_scope(path: Option<&Path>) -> Result<(CmdContext, PathBuf)> {
    let has_explicit_path = path.is_some();
    let ctx = CmdContext::from_path(path)?;
    if !ctx.start.exists() {
        bail!("path does not exist: {}", ctx.start.display());
    }
    let scope = if has_explicit_path {
        ctx.start.clone()
    } else {
        enclosing_repo(&ctx.start, &ctx.workspace.root)
    };
    Ok((ctx, scope))
}

/// Nearest ancestor of `start` (inclusive) containing a `.git` entry, bounded
/// at `workspace_root`. Falls back to `start` when no repo marker is found.
fn enclosing_repo(start: &Path, workspace_root: &Path) -> PathBuf {
    let mut dir = Some(start);
    while let Some(d) = dir {
        if d.join(".git").exists() {
            return d.to_path_buf();
        }
        if d == workspace_root {
            break;
        }
        dir = d.parent();
    }
    start.to_path_buf()
}

/// `kdb codemap ls [path] [--json]` — discover maps and list them.
pub fn ls(path: Option<PathBuf>, json: bool) -> Result<()> {
    let (ctx, scope) = resolve_scope(path.as_deref())?;

    let discovery = discover::discover(&ctx.workspace, &scope)?;

    if json {
        println!("{}", serde_json::to_string_pretty(&discovery.docs)?);
        return Ok(());
    }

    print!("{}", render_ls_table(&discovery.docs));
    if !discovery.problems.is_empty() {
        eprintln!(
            "kdb codemap: {} map(s) with frontmatter problems (run `kdb codemap check`)",
            discovery.problems.len()
        );
    }
    Ok(())
}

/// Render the discovered maps as an aligned text table.
fn render_ls_table(docs: &[CodemapDoc]) -> String {
    if docs.is_empty() {
        return String::from("(no codemaps)\n");
    }

    let display_root = |doc: &CodemapDoc| {
        let s = doc.root.to_string_lossy();
        if s.is_empty() { ".".to_string() } else { s.into_owned() }
    };

    let domain_w = docs.iter().map(|d| d.domain.len()).max().unwrap_or(6).max(6);
    let root_w = docs
        .iter()
        .map(|d| display_root(d).len())
        .max()
        .unwrap_or(4)
        .max(4);

    let mut out = String::new();
    out.push_str(&format!(
        "{:<domain_w$}  {:<root_w$}  updated\n",
        "domain", "root",
    ));
    for doc in docs {
        out.push_str(&format!(
            "{:<domain_w$}  {:<root_w$}  {}\n",
            doc.domain,
            display_root(doc),
            doc.updated.as_deref().unwrap_or("—"),
        ));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn doc(domain: &str, root: &str, updated: Option<&str>) -> CodemapDoc {
        CodemapDoc {
            file: Path::new(root).join("CODEMAP.md"),
            domain: domain.to_string(),
            repo: None,
            root: PathBuf::from(root),
            owner: None,
            updated: updated.map(str::to_string),
            commit: None,
        }
    }

    #[test]
    fn empty_table() {
        assert_eq!(render_ls_table(&[]), "(no codemaps)\n");
    }

    #[test]
    fn table_lists_domains_and_updated() {
        let docs = vec![
            doc("auth", "src/auth", Some("2026.06.24")),
            doc("fmt", "src/fmt", None),
        ];
        let table = render_ls_table(&docs);
        assert!(table.contains("auth"));
        assert!(table.contains("src/fmt"));
        assert!(table.contains("2026.06.24"));
        assert!(table.contains('—'));
    }
}
