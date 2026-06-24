//! Git-derived staleness for codemaps, with graceful fallback.
//!
//! A map is stale when files under its scope changed since the commit it was
//! pinned against (`git diff --name-only <commit>..HEAD -- <root>`). Git is run
//! from the map's *subtree directory* rather than the workspace root: the kdb
//! workspace may be a multi-repo meta-workspace whose root isn't a git repo at
//! all, so `git -C <subtree>` lets git resolve the repo that actually owns the
//! code. Everything degrades to `Unverifiable` when git or the commit pin is
//! missing — kdb must keep working in a non-git tree.

use std::path::{Path, PathBuf};
use std::process::Command;

use super::CodemapDoc;

// --------------------------
// projects/kdb/src/codemap/git.rs
//
// const SAMPLE_LIMIT     L26
// pub enum Staleness     L30
// pub fn staleness()     L48
// fn run_git()          L106
// --------------------------

/// Maximum number of sample changed-file paths to surface in a stale finding.
const SAMPLE_LIMIT: usize = 3;

/// Staleness verdict for a single map.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Staleness {
    /// No files changed under the map's scope since its pinned commit.
    Fresh,
    /// Files changed under the map's scope since its pinned commit.
    Stale {
        changed: usize,
        commit_distance: Option<usize>,
        sample: Vec<PathBuf>,
    },
    /// Could not verify (no git, no commit pin, or unknown commit).
    Unverifiable { reason: String },
}

/// Determine a map's staleness against its pinned `commit`.
///
/// `repo_root` joined with `doc.root` yields the absolute subtree git runs
/// against; the caller guarantees that path exists (dangling roots are filtered
/// out earlier).
pub fn staleness(repo_root: &Path, doc: &CodemapDoc) -> Staleness {
    let Some(commit) = doc.commit.as_deref().filter(|c| !c.is_empty()) else {
        return Staleness::Unverifiable {
            reason: "no commit pin in frontmatter".to_string(),
        };
    };

    let subtree = repo_root.join(&doc.root);

    if run_git(&subtree, &["rev-parse", "--git-dir"]).is_none() {
        return Staleness::Unverifiable {
            reason: "not a git repository".to_string(),
        };
    }

    // Confirm the pinned commit is reachable in this repo.
    if run_git(&subtree, &["cat-file", "-e", &format!("{commit}^{{commit}}")]).is_none() {
        return Staleness::Unverifiable {
            reason: format!("pinned commit {commit} not found in repo"),
        };
    }

    let range = format!("{commit}..HEAD");
    // `-- .` scopes the diff to the subtree (cwd is the subtree via `-C`).
    let Some(diff) = run_git(&subtree, &["diff", "--name-only", &range, "--", "."]) else {
        return Staleness::Unverifiable {
            reason: "git diff failed".to_string(),
        };
    };

    let changed_paths: Vec<PathBuf> = diff
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(PathBuf::from)
        // A map's own CODEMAP.md (or a nested one) is not code drift — committing
        // the map after its pin must not flag the map as stale against itself.
        .filter(|p| p.file_name().and_then(|n| n.to_str()) != Some("CODEMAP.md"))
        .collect();

    if changed_paths.is_empty() {
        return Staleness::Fresh;
    }

    let commit_distance = run_git(&subtree, &["rev-list", "--count", &range])
        .and_then(|s| s.trim().parse::<usize>().ok());

    let sample = changed_paths.iter().take(SAMPLE_LIMIT).cloned().collect();

    Staleness::Stale {
        changed: changed_paths.len(),
        commit_distance,
        sample,
    }
}

/// Run `git -C <dir> <args>`, returning trimmed stdout on success or `None` on
/// any failure (git absent, non-zero exit, non-UTF-8 output).
fn run_git(dir: &Path, args: &[&str]) -> Option<String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    String::from_utf8(output.stdout).ok()
}
