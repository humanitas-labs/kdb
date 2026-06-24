//! `kdb codemap render` — assemble the deterministic index from discovered maps.
//!
//! Pure derivation to stdout: a domains table (with git-derived staleness) and
//! a coverage section. The caller decides where to write it (placement isn't
//! kdb's call). No prose, no LLM — the trustworthy machine-derived core that an
//! authoring workflow can build prose on top of.

use std::path::{Path, PathBuf};

use anyhow::Result;

use super::check::{self, DEFAULT_MIN_FILES};
use super::git::{self, Staleness};
use super::{discover, CodemapDoc};

// -----------------------------------------------
// projects/kdb/src/codemap/render.rs
//
// pub fn render()                             L30
// fn assemble()                               L44
// fn status_cell()                           L102
// fn display_root()                          L116
// mod tests                                  L126
// fn doc()                                   L129
// fn assembles_table_and_links()             L142
// fn reports_coverage_gaps_and_problems()    L154
// -----------------------------------------------

/// `kdb codemap render [path]` — print the index scaffold to stdout.
pub fn render(path: Option<PathBuf>) -> Result<()> {
    let (ctx, scope) = super::resolve_scope(path.as_deref())?;
    let discovery = discover::discover(&ctx.workspace, &scope)?;
    let code_files = discover::discover_code_files(&ctx.workspace, &scope)?;
    let gaps = check::orphan_candidates(&discovery.docs, &code_files, DEFAULT_MIN_FILES);

    print!(
        "{}",
        assemble(&scope, &discovery.docs, &discovery.problems, &gaps)
    );
    Ok(())
}

/// Assemble the index markdown from the derived facts.
fn assemble(
    repo_root: &Path,
    docs: &[CodemapDoc],
    problems: &[super::ParseProblem],
    gaps: &[(PathBuf, usize)],
) -> String {
    let mut out = String::new();
    out.push_str("# Codemap Index\n\n");
    out.push_str(
        "> Machine-derived by `kdb codemap render`. The domains table and coverage\n\
         > section are regenerated from the colocated `CODEMAP.md` files — do not edit by hand.\n\n",
    );

    out.push_str("## Domains\n\n");
    if docs.is_empty() {
        out.push_str("_No codemaps found._\n\n");
    } else {
        out.push_str("| Domain | Root | Owner | Updated | Status |\n");
        out.push_str("|---|---|---|---|---|\n");
        for doc in docs {
            let status = status_cell(&git::staleness(repo_root, doc));
            out.push_str(&format!(
                "| [{}]({}) | `{}` | {} | {} | {} |\n",
                doc.domain,
                doc.file.display(),
                display_root(&doc.root),
                doc.owner.as_deref().unwrap_or("—"),
                doc.updated.as_deref().unwrap_or("—"),
                status,
            ));
        }
        out.push('\n');
    }

    out.push_str("## Coverage\n\n");
    if gaps.is_empty() {
        out.push_str("_No significant uncovered subtrees._\n\n");
    } else {
        out.push_str("Subtrees with no map (candidate domains):\n\n");
        for (dir, count) in gaps {
            out.push_str(&format!("- `{}` — {count} uncovered code file(s)\n", display_root(dir)));
        }
        out.push('\n');
    }

    if !problems.is_empty() {
        out.push_str("## Problems\n\n");
        out.push_str("Maps whose frontmatter could not be parsed:\n\n");
        for p in problems {
            out.push_str(&format!("- `{}` — {}\n", p.file.display(), p.message));
        }
        out.push('\n');
    }

    out
}

/// The status cell for a map, from its staleness verdict.
fn status_cell(staleness: &Staleness) -> String {
    match staleness {
        Staleness::Fresh => "current".to_string(),
        Staleness::Stale { changed, commit_distance, .. } => {
            let dist = commit_distance
                .map(|d| format!(", {d} commits behind"))
                .unwrap_or_default();
            format!("⚠ {changed} file(s) changed{dist}")
        }
        Staleness::Unverifiable { reason } => format!("unverified ({reason})"),
    }
}

/// Display a repo-relative path, mapping the empty root to `.`.
fn display_root(path: &Path) -> String {
    let s = path.to_string_lossy();
    if s.is_empty() {
        ".".to_string()
    } else {
        s.into_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn doc(domain: &str, root: &str) -> CodemapDoc {
        CodemapDoc {
            file: Path::new(root).join("CODEMAP.md"),
            domain: domain.to_string(),
            repo: None,
            root: PathBuf::from(root),
            owner: None,
            updated: Some("2026.06.24".to_string()),
            commit: None,
        }
    }

    #[test]
    fn assembles_table_and_links() {
        let docs = vec![doc("deps", "src/deps")];
        let md = assemble(Path::new("/repo"), &docs, &[], &[]);
        assert!(md.contains("# Codemap Index"));
        assert!(md.contains("[deps](src/deps/CODEMAP.md)"));
        assert!(md.contains("`src/deps`"));
        // No commit pin ⇒ unverified status.
        assert!(md.contains("unverified"));
        assert!(md.contains("_No significant uncovered subtrees._"));
    }

    #[test]
    fn reports_coverage_gaps_and_problems() {
        let gaps = vec![(PathBuf::from("src/api"), 7)];
        let problems = vec![super::super::ParseProblem {
            file: PathBuf::from("src/x/CODEMAP.md"),
            message: "missing required `domain`".to_string(),
        }];
        let md = assemble(Path::new("/repo"), &[], &problems, &gaps);
        assert!(md.contains("_No codemaps found._"));
        assert!(md.contains("`src/api` — 7 uncovered"));
        assert!(md.contains("## Problems"));
        assert!(md.contains("missing required `domain`"));
    }
}
