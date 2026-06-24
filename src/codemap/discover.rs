//! Discover `CODEMAP.md` files in a repo and parse their frontmatter.
//!
//! Reuses the shared `ignore`-aware walker (`workspace::discover::discover_files`)
//! so discovery honors `.gitignore`/`.ignore` rules. Paths are **repo-relative**:
//! the repo root (the resolved scope) is the base, so a map records `root:
//! src/deps`, not a workspace-relative path. This keeps maps portable with the
//! codebase, independent of where the repo sits inside a meta-workspace.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use crate::lang::CodeLanguage;
use crate::workspace::discover::discover_files;
use crate::workspace::WorkspaceContext;

use super::frontmatter;
use super::{CodemapDoc, ParseProblem};

// -----------------------------------
// projects/kdb/src/codemap/discover.rs
//
// pub const CODEMAP_FILE          L30
// pub fn discover_code_files()    L35
// pub struct Discovery            L49
// pub fn discover()               L58
// -----------------------------------

/// The fixed filename that marks a colocated domain map.
pub const CODEMAP_FILE: &str = "CODEMAP.md";

/// Discover supported code files under `repo_root`, as repo-relative paths.
///
/// Used by `check` coverage analysis to find subtrees no map covers.
pub fn discover_code_files(
    workspace: &WorkspaceContext,
    repo_root: &Path,
) -> Result<Vec<PathBuf>> {
    let files = discover_files(repo_root, repo_root, &workspace.ignore_set)?;
    Ok(files
        .into_iter()
        .filter(|rel| CodeLanguage::from_path(rel).is_some())
        .collect())
}

/// Result of scanning a repo for codemaps: the parsed docs plus any files
/// whose frontmatter could not be parsed (surfaced by `check`, not `ls`).
#[derive(Debug, Default)]
pub struct Discovery {
    pub docs: Vec<CodemapDoc>,
    pub problems: Vec<ParseProblem>,
}

/// Discover and parse all `CODEMAP.md` files under `repo_root`.
///
/// `repo_root` must be an absolute path; returned `file`/`root` paths are
/// relative to it. Results are sorted by the map's root for stable output.
pub fn discover(workspace: &WorkspaceContext, repo_root: &Path) -> Result<Discovery> {
    let files = discover_files(repo_root, repo_root, &workspace.ignore_set)?;

    let mut discovery = Discovery::default();
    for rel in files {
        if rel.file_name().and_then(|n| n.to_str()) != Some(CODEMAP_FILE) {
            continue;
        }
        // The repo-root CODEMAP.md is the rendered index, not a domain map.
        // Domain maps are colocated in subtrees; skip a root-level CODEMAP.md.
        if rel.parent().map(|p| p.as_os_str().is_empty()).unwrap_or(true) {
            continue;
        }
        let abs = repo_root.join(&rel);
        let content = std::fs::read_to_string(&abs)
            .with_context(|| format!("failed to read {}", abs.display()))?;
        match frontmatter::parse(&rel, &content) {
            Ok(doc) => discovery.docs.push(doc),
            Err(problem) => discovery.problems.push(problem),
        }
    }

    discovery.docs.sort_by(|a, b| a.root.cmp(&b.root).then(a.domain.cmp(&b.domain)));
    discovery.problems.sort_by(|a, b| a.file.cmp(&b.file));
    Ok(discovery)
}
