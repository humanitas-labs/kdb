//! Import legacy `T-NNNN.md` task files into the relational layer.
//!
//! Reads every `T-*.md` file in a directory, parses its YAML frontmatter
//! (id, title, status, priority), and inserts a task row using the
//! numeric portion of the id as the per-project `seq`.

use anyhow::{Context, Result, bail};
use rusqlite::Connection;
use std::fs;
use std::path::Path;

use crate::projects::Project;
use crate::tasks::{self, AddArgs};

// -------------------------------------
// projects/kdb/src/tasks_import.rs
//
// pub struct ImportReport           L39
// pub struct ImportedRow            L45
// pub struct SkipReason             L51
// struct ParsedFile                 L56
// pub fn import_dir()               L65
// fn parse_task_file()             L140
// fn split_frontmatter()           L163
// fn parse_frontmatter()           L186
// fn unquote_scalar()              L220
// fn parse_seq()                   L234
// fn map_status()                  L248
// fn map_priority()                L258
// fn strip_leading_heading()       L274
// mod tests                        L289
// fn parses_basic_frontmatter()    L293
// fn maps_priorities()             L304
// fn maps_statuses()               L312
// -------------------------------------

/// Result of an import run.
#[derive(Debug, Default)]
pub struct ImportReport {
    pub imported: Vec<ImportedRow>,
    pub skipped: Vec<SkipReason>,
}

#[derive(Debug)]
pub struct ImportedRow {
    pub source: String,
    pub external_id: String,
}

#[derive(Debug)]
pub struct SkipReason {
    pub source: String,
    pub reason: String,
}

struct ParsedFile {
    seq: i64,
    title: String,
    status: String,
    priority: i64,
    body: String,
}

/// Import every `T-*.md` file from `dir` into `project`.
pub fn import_dir(conn: &mut Connection, project: &Project, dir: &Path) -> Result<ImportReport> {
    let mut report = ImportReport::default();
    if !dir.exists() {
        bail!("source directory does not exist: {}", dir.display());
    }

    let mut entries: Vec<_> = fs::read_dir(dir)
        .with_context(|| format!("failed to read {}", dir.display()))?
        .filter_map(|e| e.ok())
        .collect();
    entries.sort_by_key(|e| e.file_name());

    for entry in entries {
        let path = entry.path();
        let name = entry.file_name().into_string().unwrap_or_default();
        if !name.starts_with("T-") || !name.ends_with(".md") {
            continue;
        }
        let source = name.clone();

        let contents = match fs::read_to_string(&path) {
            Ok(c) => c,
            Err(e) => {
                report.skipped.push(SkipReason {
                    source,
                    reason: format!("read failed: {e}"),
                });
                continue;
            }
        };
        let parsed = match parse_task_file(&contents) {
            Ok(p) => p,
            Err(e) => {
                report.skipped.push(SkipReason {
                    source,
                    reason: format!("parse failed: {e}"),
                });
                continue;
            }
        };

        let body = if parsed.body.trim().is_empty() {
            None
        } else {
            Some(parsed.body.as_str())
        };
        let result = tasks::add(
            conn,
            AddArgs {
                project_id: Some(project.id),
                space_id: None,
                title: &parsed.title,
                body,
                priority: Some(parsed.priority),
                cycle_id: None,
                parent_id: None,
                seq: Some(parsed.seq),
                status: Some(&parsed.status),
                order: None,
            },
        );
        match result {
            Ok(view) => report.imported.push(ImportedRow {
                source,
                external_id: view.external_id(),
            }),
            Err(e) => report.skipped.push(SkipReason {
                source,
                reason: format!("insert failed: {e:#}"),
            }),
        }
    }
    Ok(report)
}

fn parse_task_file(contents: &str) -> Result<ParsedFile> {
    let (fm, body) = split_frontmatter(contents)?;
    let fm = parse_frontmatter(fm)?;

    let id = fm.get("id").context("missing `id` in frontmatter")?.clone();
    let seq = parse_seq(&id).with_context(|| format!("unable to parse seq from id '{id}'"))?;

    let title = fm.get("title").context("missing `title`")?.clone();
    let status_raw = fm.get("status").map(String::as_str).unwrap_or("planned");
    let status = map_status(status_raw);
    let priority_raw = fm.get("priority").map(String::as_str).unwrap_or("medium");
    let priority = map_priority(priority_raw);

    let body = strip_leading_heading(body, &title);
    Ok(ParsedFile {
        seq,
        title,
        status: status.to_string(),
        priority,
        body,
    })
}

fn split_frontmatter(s: &str) -> Result<(&str, &str)> {
    let s = s.strip_prefix('\u{feff}').unwrap_or(s);
    let rest = s
        .strip_prefix("---\n")
        .or_else(|| s.strip_prefix("---\r\n"))
        .context("file does not start with `---`")?;
    let end = rest
        .find("\n---\n")
        .or_else(|| rest.find("\n---\r\n"))
        .or_else(|| rest.find("\r\n---\r\n"))
        .context("frontmatter terminator `---` not found")?;
    let fm = &rest[..end];
    let body_start = rest[end..]
        .find('\n')
        .map(|i| end + i + 1)
        .and_then(|i| rest[i..].find('\n').map(|j| i + j + 1))
        .unwrap_or(rest.len());
    let body = &rest[body_start..];
    Ok((fm, body))
}

/// Minimal YAML-ish parser: top-level `key: value` pairs only. List and
/// multi-line block values are ignored (sufficient for id/title/status/priority).
fn parse_frontmatter(fm: &str) -> Result<std::collections::HashMap<String, String>> {
    let mut out = std::collections::HashMap::new();
    let mut lines = fm.lines().peekable();
    while let Some(line) = lines.next() {
        let trimmed = line.trim_end();
        if trimmed.is_empty() {
            continue;
        }
        // Skip indented continuation lines and nested block content.
        if line.starts_with(' ') || line.starts_with('\t') {
            continue;
        }
        let Some(colon) = line.find(':') else {
            continue;
        };
        let key = line[..colon].trim().to_string();
        let raw_value = line[colon + 1..].trim();
        if raw_value.is_empty() {
            // Block scalar or list — skip its indented body and record empty.
            while let Some(next) = lines.peek() {
                if next.starts_with(' ') || next.starts_with('\t') || next.is_empty() {
                    lines.next();
                } else {
                    break;
                }
            }
            out.insert(key, String::new());
            continue;
        }
        out.insert(key, unquote_scalar(raw_value));
    }
    Ok(out)
}

fn unquote_scalar(s: &str) -> String {
    let s = s.trim();
    if (s.starts_with('"') && s.ends_with('"') && s.len() >= 2)
        || (s.starts_with('\'') && s.ends_with('\'') && s.len() >= 2)
    {
        s[1..s.len() - 1]
            .replace("\\\"", "\"")
            .replace("\\\\", "\\")
    } else {
        s.to_string()
    }
}

/// Extract the numeric tail of an id like "T-0100" or "T-100" → 100.
fn parse_seq(id: &str) -> Result<i64> {
    let tail: String = id
        .chars()
        .rev()
        .take_while(|c| c.is_ascii_digit())
        .collect();
    if tail.is_empty() {
        bail!("no trailing digits in id '{id}'");
    }
    let tail: String = tail.chars().rev().collect();
    tail.parse::<i64>()
        .with_context(|| format!("id '{id}' has invalid seq"))
}

fn map_status(s: &str) -> &'static str {
    match s.to_ascii_lowercase().as_str() {
        "in_progress" | "in-progress" | "active" => "in_progress",
        "done" | "completed" | "closed" => "done",
        "cycle" | "scheduled" => "cycle",
        "parked" | "deferred" => "parked",
        _ => "backlog",
    }
}

fn map_priority(s: &str) -> i64 {
    if let Ok(n) = s.parse::<i64>() {
        return n.clamp(1, 5);
    }
    match s.to_ascii_lowercase().as_str() {
        "critical" | "p0" | "urgent" => 1,
        "high" | "p1" => 1,
        "medium" | "med" | "p2" => 2,
        "low" | "p3" => 3,
        _ => 3,
    }
}

/// If the body starts with `# <title>` (optionally followed by an
/// `id — title` shape), strip that heading so it isn't duplicated below
/// the generated one.
fn strip_leading_heading(body: &str, title: &str) -> String {
    let trimmed = body.trim_start_matches(['\n', '\r', ' ', '\t']);
    if let Some(rest) = trimmed.strip_prefix("# ") {
        let (first, rest_after) = match rest.split_once('\n') {
            Some(pair) => pair,
            None => (rest, ""),
        };
        if first.contains(title) {
            return rest_after.trim_start_matches(['\n', '\r']).to_string();
        }
    }
    body.trim_start_matches(['\n', '\r']).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_basic_frontmatter() {
        let src = "---\nid: T-0100\ntitle: \"Do the thing\"\nstatus: planned\npriority: high\nlabels: [a, b]\n---\n\n# T-0100 — Do the thing\n\nbody text\n";
        let parsed = parse_task_file(src).unwrap();
        assert_eq!(parsed.seq, 100);
        assert_eq!(parsed.title, "Do the thing");
        assert_eq!(parsed.status, "backlog");
        assert_eq!(parsed.priority, 1);
        assert!(parsed.body.starts_with("body text"));
    }

    #[test]
    fn maps_priorities() {
        assert_eq!(map_priority("high"), 1);
        assert_eq!(map_priority("medium"), 2);
        assert_eq!(map_priority("low"), 3);
        assert_eq!(map_priority("2"), 2);
    }

    #[test]
    fn maps_statuses() {
        assert_eq!(map_status("planned"), "backlog");
        assert_eq!(map_status("in_progress"), "in_progress");
        assert_eq!(map_status("backlog"), "backlog");
        assert_eq!(map_status("cycle"), "cycle");
        assert_eq!(map_status("done"), "done");
        assert_eq!(map_status("proposed"), "backlog");
    }
}
