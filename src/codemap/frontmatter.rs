//! Parse `CODEMAP.md` YAML frontmatter into a [`CodemapDoc`].
//!
//! The frontmatter contract (defined by `~/.claude/templates/codemap.md`):
//!
//! ```yaml
//! domain: …   repo: …   root: …   owner: …
//! updated: YYYY.MM.DD   commit: <short SHA>   confidence: high|medium|low
//! ```
//!
//! `confidence` is intentionally **not** modeled — it is an authored
//! self-assessment, not a derived signal, so kdb neither types nor lints it.
//! serde ignores unknown fields by default, so it is silently dropped.

use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::workspace::paths::normalize_rel_path;

use super::{CodemapDoc, ParseProblem};

// -----------------------------------------------
// projects/kdb/src/codemap/frontmatter.rs
//
// struct RawFrontmatter                       L42
// pub fn parse()                              L55
// fn split_frontmatter()                      L94
// fn clean_opt()                             L107
// fn normalize_root()                        L115
// mod tests                                  L122
// fn parses_full_frontmatter()               L126
// fn confidence_is_ignored_not_an_error()    L138
// fn root_defaults_to_map_directory()        L145
// fn missing_domain_is_a_problem()           L152
// fn missing_frontmatter_is_a_problem()      L159
// fn malformed_yaml_is_a_problem()           L166
// -----------------------------------------------

/// Raw frontmatter shape as authored. All fields optional so a missing
/// required field surfaces as a [`ParseProblem`] rather than a serde error.
#[derive(Debug, Deserialize)]
struct RawFrontmatter {
    domain: Option<String>,
    repo: Option<String>,
    root: Option<String>,
    owner: Option<String>,
    updated: Option<String>,
    commit: Option<String>,
}

/// Parse a single `CODEMAP.md`'s content into a [`CodemapDoc`].
///
/// `file_rel` is the map's repo-relative path (used for defaulting `root` to
/// the map's own directory and for error reporting).
pub fn parse(file_rel: &Path, content: &str) -> Result<CodemapDoc, ParseProblem> {
    let problem = |msg: &str| ParseProblem {
        file: file_rel.to_path_buf(),
        message: msg.to_string(),
    };

    let block = split_frontmatter(content)
        .ok_or_else(|| problem("missing YAML frontmatter (expected leading `---` block)"))?;

    let raw: RawFrontmatter = serde_yaml::from_str(block)
        .map_err(|e| problem(&format!("invalid YAML frontmatter: {e}")))?;

    let domain = raw
        .domain
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .ok_or_else(|| problem("frontmatter missing required `domain`"))?;

    let root = match raw.root.map(|s| s.trim().to_string()).filter(|s| !s.is_empty()) {
        Some(authored) => normalize_root(&authored),
        // Colocated default: the directory holding the map file.
        None => file_rel.parent().map(Path::to_path_buf).unwrap_or_default(),
    };

    Ok(CodemapDoc {
        file: file_rel.to_path_buf(),
        domain,
        repo: clean_opt(raw.repo),
        root,
        owner: clean_opt(raw.owner),
        updated: clean_opt(raw.updated),
        commit: clean_opt(raw.commit),
    })
}

/// Extract the YAML frontmatter block from a markdown document.
///
/// Returns the content between the leading `---` fence and its closing `---`,
/// or `None` if the document does not open with a frontmatter fence.
fn split_frontmatter(content: &str) -> Option<&str> {
    let rest = content.strip_prefix("---\n").or_else(|| content.strip_prefix("---\r\n"))?;
    // Find the closing fence at the start of a line.
    for marker in ["\n---\n", "\n---\r\n", "\r\n---\r\n"] {
        if let Some(end) = rest.find(marker) {
            return Some(&rest[..end]);
        }
    }
    // Allow a document that is *only* frontmatter (closing fence at EOF).
    rest.strip_suffix("\n---").or(Some(rest))
}

/// Trim whitespace and drop empty strings from an optional field.
fn clean_opt(value: Option<String>) -> Option<String> {
    value.map(|s| s.trim().to_string()).filter(|s| !s.is_empty())
}

/// Normalize an authored, repo-relative `root` value.
///
/// Falls back to a lexically cleaned path when the value escapes the repo
/// (`..`) or is absolute — `check` flags those as dangling/outside.
fn normalize_root(authored: &str) -> PathBuf {
    let trimmed = authored.trim_start_matches("./").trim_end_matches('/');
    let path = PathBuf::from(trimmed);
    normalize_rel_path(&path).unwrap_or(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_full_frontmatter() {
        let content = "---\ndomain: auth\nrepo: kdb\nroot: src/auth\nowner: andrew\nupdated: 2026.06.24\ncommit: abc1234\nconfidence: high\n---\n\n# Auth — Code Map\n";
        let doc = parse(Path::new("src/auth/CODEMAP.md"), content).unwrap();
        assert_eq!(doc.domain, "auth");
        assert_eq!(doc.repo.as_deref(), Some("kdb"));
        assert_eq!(doc.root, PathBuf::from("src/auth"));
        assert_eq!(doc.owner.as_deref(), Some("andrew"));
        assert_eq!(doc.updated.as_deref(), Some("2026.06.24"));
        assert_eq!(doc.commit.as_deref(), Some("abc1234"));
    }

    #[test]
    fn confidence_is_ignored_not_an_error() {
        let content = "---\ndomain: x\nconfidence: nonsense\n---\n";
        let doc = parse(Path::new("x/CODEMAP.md"), content).unwrap();
        assert_eq!(doc.domain, "x");
    }

    #[test]
    fn root_defaults_to_map_directory() {
        let content = "---\ndomain: fmt\n---\n";
        let doc = parse(Path::new("src/fmt/CODEMAP.md"), content).unwrap();
        assert_eq!(doc.root, PathBuf::from("src/fmt"));
    }

    #[test]
    fn missing_domain_is_a_problem() {
        let content = "---\nrepo: kdb\n---\n";
        let err = parse(Path::new("CODEMAP.md"), content).unwrap_err();
        assert!(err.message.contains("domain"));
    }

    #[test]
    fn missing_frontmatter_is_a_problem() {
        let content = "# Just a heading\n";
        let err = parse(Path::new("CODEMAP.md"), content).unwrap_err();
        assert!(err.message.contains("missing YAML frontmatter"));
    }

    #[test]
    fn malformed_yaml_is_a_problem() {
        let content = "---\ndomain: [unterminated\n---\n";
        let err = parse(Path::new("CODEMAP.md"), content).unwrap_err();
        assert!(err.message.contains("invalid YAML"));
    }
}
