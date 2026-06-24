//! `kdb codemap check` — coverage and freshness lints over discovered maps.
//!
//! Findings: dangling roots, coverage gaps (orphan subtrees), stale maps
//! (git-derived, [`super::git`]), and frontmatter parse problems. `--strict`
//! turns any finding into a non-zero exit for CI/pre-commit.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use anyhow::Result;
use serde::Serialize;

use crate::workspace::paths::normalize_rel_path;

use super::discover;
use super::git::{self, Staleness};
use super::CodemapDoc;

// ---------------------------------------------------------
// projects/kdb/src/codemap/check.rs
//
// pub const DEFAULT_MIN_FILES                           L42
// pub struct Lints                                      L46
//   fn run_stale()                                      L52
//   fn run_orphans()                                    L55
//   fn is_all()                                         L58
// pub enum Finding                                      L66
// pub fn check()                                        L86
// fn dangling_reason()                                 L169
// pub(crate) fn orphan_candidates()                    L187
// fn render_findings()                                 L232
// mod tests                                            L335
// fn doc()                                             L338
// fn covered_files_are_not_orphans()                   L351
// fn uncovered_subtree_above_threshold_is_flagged()    L365
// fn maximal_candidate_only_no_nested_duplicates()     L380
// fn below_threshold_is_quiet()                        L393
// ---------------------------------------------------------

/// Default minimum supported-code-file count for a subtree to be flagged as a
/// coverage gap (orphan candidate). Conservative to keep noise down.
pub const DEFAULT_MIN_FILES: usize = 5;

/// Which lint families to run. An empty selection (no flags) means "all".
#[derive(Debug, Clone, Copy, Default)]
pub struct Lints {
    pub stale: bool,
    pub orphans: bool,
}

impl Lints {
    fn run_stale(self) -> bool {
        self.stale || self.is_all()
    }
    fn run_orphans(self) -> bool {
        self.orphans || self.is_all()
    }
    fn is_all(self) -> bool {
        !self.stale && !self.orphans
    }
}

/// A single lint finding.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Finding {
    /// The map's `root` does not exist, or points outside the workspace.
    Dangling { file: PathBuf, root: PathBuf, reason: String },
    /// A significant code subtree no map covers.
    Orphan { dir: PathBuf, file_count: usize },
    /// Files changed under the map's scope since it was written.
    Stale {
        file: PathBuf,
        root: PathBuf,
        changed: usize,
        commit_distance: Option<usize>,
        sample: Vec<PathBuf>,
    },
    /// Frontmatter could not be parsed.
    Problem { file: PathBuf, message: String },
    /// Map staleness could not be verified (no git / no commit pin).
    Unverifiable { file: PathBuf, reason: String },
}

/// `kdb codemap check [path] [--stale] [--orphans] [--strict] [--min-files N]`.
pub fn check(
    path: Option<PathBuf>,
    lints: Lints,
    strict: bool,
    min_files: usize,
    json: bool,
) -> Result<()> {
    let (ctx, scope) = super::resolve_scope(path.as_deref())?;

    let discovery = discover::discover(&ctx.workspace, &scope)?;
    let mut findings: Vec<Finding> = Vec::new();

    // Parse problems are always reported — a map that can't be read can't be trusted.
    for problem in &discovery.problems {
        findings.push(Finding::Problem {
            file: problem.file.clone(),
            message: problem.message.clone(),
        });
    }

    // Dangling roots are always checked (cheap; a dangling root invalidates every other lint).
    for doc in &discovery.docs {
        if let Some(reason) = dangling_reason(&scope, doc) {
            findings.push(Finding::Dangling {
                file: doc.file.clone(),
                root: doc.root.clone(),
                reason,
            });
        }
    }

    if lints.run_orphans() {
        let code_files = discover::discover_code_files(&ctx.workspace, &scope)?;
        for (dir, count) in orphan_candidates(&discovery.docs, &code_files, min_files) {
            findings.push(Finding::Orphan { dir, file_count: count });
        }
    }

    if lints.run_stale() {
        for doc in &discovery.docs {
            // Skip stale analysis for maps whose root is already dangling.
            if dangling_reason(&scope, doc).is_some() {
                continue;
            }
            match git::staleness(&scope, doc) {
                Staleness::Stale { changed, commit_distance, sample } => {
                    findings.push(Finding::Stale {
                        file: doc.file.clone(),
                        root: doc.root.clone(),
                        changed,
                        commit_distance,
                        sample,
                    });
                }
                Staleness::Unverifiable { reason } => {
                    findings.push(Finding::Unverifiable {
                        file: doc.file.clone(),
                        reason,
                    });
                }
                Staleness::Fresh => {}
            }
        }
    }

    if json {
        println!("{}", serde_json::to_string_pretty(&findings)?);
    } else {
        print!("{}", render_findings(&findings));
    }

    // Unverifiable is advisory, not a failure. Everything else is actionable.
    let actionable = findings
        .iter()
        .filter(|f| !matches!(f, Finding::Unverifiable { .. }))
        .count();
    if strict && actionable > 0 {
        std::process::exit(1);
    }
    Ok(())
}

/// Why a map's root is dangling, or `None` if it resolves to an existing path.
fn dangling_reason(repo_root: &Path, doc: &CodemapDoc) -> Option<String> {
    if normalize_rel_path(&doc.root).is_none() {
        return Some("root points outside the repo".to_string());
    }
    let abs = repo_root.join(&doc.root);
    if abs.exists() {
        None
    } else {
        Some("root no longer exists".to_string())
    }
}

/// Find maximal uncovered code subtrees meeting the `min_files` threshold.
///
/// A code file is *covered* if some map root is an ancestor of it. We count
/// uncovered files per ancestor directory (recursively), keep directories at or
/// above the threshold, then drop any that nest inside another candidate so we
/// report the outermost orphan subtree once.
pub(crate) fn orphan_candidates(
    docs: &[CodemapDoc],
    code_files: &[PathBuf],
    min_files: usize,
) -> Vec<(PathBuf, usize)> {
    let roots: Vec<&PathBuf> = docs.iter().map(|d| &d.root).collect();
    let covered = |file: &Path| roots.iter().any(|root| file.starts_with(root));

    let mut counts: HashMap<PathBuf, usize> = HashMap::new();
    for file in code_files {
        if covered(file) {
            continue;
        }
        // Walk ancestor directories, incrementing each (including the root "").
        let mut dir = file.parent();
        while let Some(d) = dir {
            *counts.entry(d.to_path_buf()).or_default() += 1;
            if d.as_os_str().is_empty() {
                break;
            }
            dir = d.parent();
        }
    }

    // A dir is a clean orphan candidate only if it meets the threshold and does
    // not enclose any map root — otherwise it straddles covered territory and the
    // real gap is a deeper subtree (report `src/api`, not `src` or the repo root).
    let mut candidates: Vec<(PathBuf, usize)> = counts
        .into_iter()
        .filter(|(dir, n)| *n >= min_files && !roots.iter().any(|root| root.starts_with(dir)))
        .collect();

    // Keep only maximal candidates: drop any dir nested under another candidate.
    let dirs: Vec<PathBuf> = candidates.iter().map(|(d, _)| d.clone()).collect();
    candidates.retain(|(dir, _)| {
        !dirs
            .iter()
            .any(|other| other != dir && dir.starts_with(other))
    });

    candidates.sort_by(|a, b| a.0.cmp(&b.0));
    candidates
}

/// Render findings grouped by kind.
fn render_findings(findings: &[Finding]) -> String {
    if findings.is_empty() {
        return String::from("kdb codemap: no findings ✓\n");
    }

    let display = |p: &Path| {
        let s = p.to_string_lossy();
        if s.is_empty() { ".".to_string() } else { s.into_owned() }
    };

    let mut out = String::new();
    let section = |title: &str, lines: Vec<String>, out: &mut String| {
        if lines.is_empty() {
            return;
        }
        out.push_str(&format!("{} ({}):\n", title, lines.len()));
        for line in lines {
            out.push_str(&format!("  {line}\n"));
        }
        out.push('\n');
    };

    section(
        "dangling",
        findings
            .iter()
            .filter_map(|f| match f {
                Finding::Dangling { file, root, reason } => {
                    Some(format!("{} — {} ({})", display(file), display(root), reason))
                }
                _ => None,
            })
            .collect(),
        &mut out,
    );

    section(
        "stale",
        findings
            .iter()
            .filter_map(|f| match f {
                Finding::Stale { file, changed, commit_distance, sample, .. } => {
                    let dist = commit_distance
                        .map(|d| format!(", {d} commits behind"))
                        .unwrap_or_default();
                    let eg = if sample.is_empty() {
                        String::new()
                    } else {
                        format!(
                            " [{}]",
                            sample.iter().map(|p| display(p)).collect::<Vec<_>>().join(", ")
                        )
                    };
                    Some(format!("{} — {changed} file(s) changed{dist}{eg}", display(file)))
                }
                _ => None,
            })
            .collect(),
        &mut out,
    );

    section(
        "coverage gaps",
        findings
            .iter()
            .filter_map(|f| match f {
                Finding::Orphan { dir, file_count } => {
                    Some(format!("{} — {file_count} uncovered code file(s)", display(dir)))
                }
                _ => None,
            })
            .collect(),
        &mut out,
    );

    section(
        "parse problems",
        findings
            .iter()
            .filter_map(|f| match f {
                Finding::Problem { file, message } => Some(format!("{} — {message}", display(file))),
                _ => None,
            })
            .collect(),
        &mut out,
    );

    section(
        "unverifiable (advisory)",
        findings
            .iter()
            .filter_map(|f| match f {
                Finding::Unverifiable { file, reason } => Some(format!("{} — {reason}", display(file))),
                _ => None,
            })
            .collect(),
        &mut out,
    );

    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn doc(root: &str) -> CodemapDoc {
        CodemapDoc {
            file: Path::new(root).join("CODEMAP.md"),
            domain: root.to_string(),
            repo: None,
            root: PathBuf::from(root),
            owner: None,
            updated: None,
            commit: None,
        }
    }

    #[test]
    fn covered_files_are_not_orphans() {
        let docs = vec![doc("src/auth")];
        let files = vec![
            PathBuf::from("src/auth/a.rs"),
            PathBuf::from("src/auth/b.rs"),
            PathBuf::from("src/auth/c.rs"),
            PathBuf::from("src/auth/d.rs"),
            PathBuf::from("src/auth/e.rs"),
        ];
        let orphans = orphan_candidates(&docs, &files, 3);
        assert!(orphans.is_empty());
    }

    #[test]
    fn uncovered_subtree_above_threshold_is_flagged() {
        let docs = vec![doc("src/auth")];
        let files = vec![
            PathBuf::from("src/auth/a.rs"),
            PathBuf::from("src/api/x.rs"),
            PathBuf::from("src/api/y.rs"),
            PathBuf::from("src/api/z.rs"),
        ];
        // `src` and the repo root straddle the covered `src/auth`, so the clean
        // gap reported is the deepest fully-uncovered subtree.
        let orphans = orphan_candidates(&docs, &files, 3);
        assert_eq!(orphans, vec![(PathBuf::from("src/api"), 3)]);
    }

    #[test]
    fn maximal_candidate_only_no_nested_duplicates() {
        let docs: Vec<CodemapDoc> = vec![];
        let files = vec![
            PathBuf::from("src/api/x.rs"),
            PathBuf::from("src/api/y.rs"),
            PathBuf::from("src/api/z.rs"),
        ];
        // Both "" , "src" and "src/api" hit the threshold; only the outermost ("") is kept.
        let orphans = orphan_candidates(&docs, &files, 3);
        assert_eq!(orphans, vec![(PathBuf::from(""), 3)]);
    }

    #[test]
    fn below_threshold_is_quiet() {
        let docs: Vec<CodemapDoc> = vec![];
        let files = vec![PathBuf::from("src/api/x.rs")];
        let orphans = orphan_candidates(&docs, &files, 5);
        assert!(orphans.is_empty());
    }
}
