//! Prosaic composition parsing for `kdb check`.
//!
//! Prosaic (spec: `kernel/prosaic.md`) is the pseudocode language SOP bodies are
//! written in. §5 formalizes *composition*: procedures are declared as
//! `## <ID> :: <Name>` markdown headings, imported with `use <ID> from <path>`,
//! and invoked with `run <ID>`. This module extracts those `use`/`run`
//! statements from a `prosaic` code block so [`crate::index::VaultIndex::check`]
//! can enforce the §5.4 resolution rules.
//!
//! The extraction is a deliberate regex line-scan rather than the
//! tree-sitter-prosaic grammar (which lags the spec — see §8): the checker only
//! needs `use`/`run`/heading recognition, and a line-scan handles that robustly.
//! Comment stripping is load-bearing — a commented `// run SOP-027 later` must
//! not count as an invocation (§5.4.3).

use regex::Regex;
use std::sync::LazyLock;

// -----------------------------------------------------
// projects/kdb/src/index/prosaic.rs
//
// static PROCEDURE_HEADING_RE                       L46
// static USE_RE                                     L52
// static RUN_RE                                     L59
// pub fn procedure_id_from_heading()                L64
// pub enum Statement                                L75
// pub fn extract_statements()                       L97
// fn strip_comments()                              L132
// fn leading_whitespace()                          L158
// mod tests                                        L163
// fn heading_ids_across_forms()                    L167
// fn extracts_use_and_run()                        L185
// fn use_alias_form_is_parsed()                    L205
// fn use_template_is_not_an_import()               L215
// fn run_with_non_procedure_target_is_ignored()    L221
// fn line_comment_run_is_ignored()                 L228
// fn block_comment_spanning_lines_is_ignored()     L235
// fn inline_block_comment_preserves_statement()    L247
// fn unicode_result_arrow_survives_stripping()     L254
// -----------------------------------------------------

/// A level-2 heading title that names a procedure: `SOP-<ID> :: <Name>`.
/// The ID accepts an optional uppercase alias/domain segment so both the legacy
/// `SOP-O04` form and the target `SOP-004` / space-tier `SOP-INP-004` forms
/// resolve during the transition.
static PROCEDURE_HEADING_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^(SOP-(?:[A-Z]+-?)?\d+)\s*::").expect("valid procedure heading regex")
});

/// `use <ID> from <path> [as <alias>]` — the only structural form of `use`
/// (§5.2). `use template: …` and any other `use` fall through unmatched.
static USE_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^\s*use\s+(SOP-(?:[A-Z]+-?)?\d+)\s+from\s+(\S+)(?:\s+as\s+(\S+))?\s*$")
        .expect("valid use regex")
});

/// `run <ID> …` — reserved statement head (§5.3). Only the ID is structural;
/// free-form arguments and a `→ result` may follow.
static RUN_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^\s*run\s+(SOP-(?:[A-Z]+-?)?\d+)\b").expect("valid run regex")
});

/// If `title` names a procedure (`SOP-<ID> :: <Name>`), return its ID.
pub fn procedure_id_from_heading(title: &str) -> Option<String> {
    PROCEDURE_HEADING_RE
        .captures(title.trim())
        .map(|caps| caps[1].to_string())
}

/// A structural composition statement extracted from a prosaic block.
///
/// `line` is the 0-based offset of the statement within the block body; callers
/// add the block's 1-based `start_line` to recover the file line number.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Statement {
    /// `use <id> from <path> [as <alias>]`
    Use {
        id: String,
        path: String,
        line: usize,
        column: usize,
        raw: String,
    },
    /// `run <id> …`
    Run {
        id: String,
        line: usize,
        column: usize,
        raw: String,
    },
}

/// Extract every `use`/`run` statement from a prosaic block body.
///
/// Comments (`/* … */`, possibly multi-line, and `// …` to end of line) are
/// stripped before matching so commented-out references are ignored (§5.4.3).
pub fn extract_statements(body: &str) -> Vec<Statement> {
    let mut out = Vec::new();
    let mut in_block_comment = false;

    for (idx, raw_line) in body.lines().enumerate() {
        let cleaned = strip_comments(raw_line, &mut in_block_comment);
        if cleaned.trim().is_empty() {
            continue;
        }
        let column = leading_whitespace(&cleaned) + 1;

        if let Some(caps) = USE_RE.captures(&cleaned) {
            out.push(Statement::Use {
                id: caps[1].to_string(),
                path: caps[2].to_string(),
                line: idx,
                column,
                raw: cleaned.trim().to_string(),
            });
        } else if let Some(caps) = RUN_RE.captures(&cleaned) {
            out.push(Statement::Run {
                id: caps[1].to_string(),
                line: idx,
                column,
                raw: cleaned.trim().to_string(),
            });
        }
    }

    out
}

/// Remove `//` line comments and `/* … */` block comments from one line,
/// threading `in_block` across lines for block comments that span lines.
/// Character-based to stay UTF-8 safe (bodies contain `→ × ÷`).
fn strip_comments(line: &str, in_block: &mut bool) -> String {
    let mut out = String::new();
    let mut chars = line.chars().peekable();

    while let Some(ch) = chars.next() {
        if *in_block {
            if ch == '*' && chars.peek() == Some(&'/') {
                chars.next();
                *in_block = false;
            }
            continue;
        }
        if ch == '/' && chars.peek() == Some(&'*') {
            chars.next();
            *in_block = true;
            continue;
        }
        if ch == '/' && chars.peek() == Some(&'/') {
            break;
        }
        out.push(ch);
    }

    out
}

fn leading_whitespace(s: &str) -> usize {
    s.chars().take_while(|c| c.is_whitespace()).count()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn heading_ids_across_forms() {
        assert_eq!(
            procedure_id_from_heading("SOP-O07 :: Opportunity Scan").as_deref(),
            Some("SOP-O07")
        );
        assert_eq!(
            procedure_id_from_heading("SOP-027 :: Opportunity Scan").as_deref(),
            Some("SOP-027")
        );
        assert_eq!(
            procedure_id_from_heading("SOP-INP-004 :: Local Thing").as_deref(),
            Some("SOP-INP-004")
        );
        assert_eq!(procedure_id_from_heading("5. Procedures and composition"), None);
        assert_eq!(procedure_id_from_heading("Index"), None);
    }

    #[test]
    fn extracts_use_and_run() {
        let body = "\
use SOP-M04 from kernel/SOP/marina.md
for each entity mentioned in the document:
    run SOP-M04 to register the map's entity mentions → registered
";
        let stmts = extract_statements(body);
        assert_eq!(stmts.len(), 2);
        assert!(matches!(
            &stmts[0],
            Statement::Use { id, path, line: 0, .. }
                if id == "SOP-M04" && path == "kernel/SOP/marina.md"
        ));
        assert!(matches!(
            &stmts[1],
            Statement::Run { id, line: 2, .. } if id == "SOP-M04"
        ));
    }

    #[test]
    fn use_alias_form_is_parsed() {
        let stmts = extract_statements("use SOP-O07 from kernel/SOP/sched/cycle.md as scan\n");
        assert!(matches!(
            &stmts[0],
            Statement::Use { id, path, .. }
                if id == "SOP-O07" && path == "kernel/SOP/sched/cycle.md"
        ));
    }

    #[test]
    fn use_template_is_not_an_import() {
        let stmts = extract_statements("use template: kernel/templates/comms/wpr.md\n");
        assert!(stmts.is_empty());
    }

    #[test]
    fn run_with_non_procedure_target_is_ignored() {
        // `run tests`, `run E2E`, `run `kdb check`` are ordinary verbs, not calls.
        let body = "run tests\nrun as many tests as you can\nrun `kdb check` — no broken links\n";
        assert!(extract_statements(body).is_empty());
    }

    #[test]
    fn line_comment_run_is_ignored() {
        let stmts = extract_statements("// run SOP-027 later\nrun SOP-027 now\n");
        assert_eq!(stmts.len(), 1);
        assert!(matches!(&stmts[0], Statement::Run { line: 1, .. }));
    }

    #[test]
    fn block_comment_spanning_lines_is_ignored() {
        let body = "\
/* fold into SOP-O01 step 1
   run SOP-O01 (not really) */
run SOP-027 for real
";
        let stmts = extract_statements(body);
        assert_eq!(stmts.len(), 1);
        assert!(matches!(&stmts[0], Statement::Run { id, line: 2, .. } if id == "SOP-027"));
    }

    #[test]
    fn inline_block_comment_preserves_statement() {
        let stmts = extract_statements("run SOP-M02   /* create */   \n");
        assert_eq!(stmts.len(), 1);
        assert!(matches!(&stmts[0], Statement::Run { id, .. } if id == "SOP-M02"));
    }

    #[test]
    fn unicode_result_arrow_survives_stripping() {
        let stmts = extract_statements("run SOP-O07 on candidates → ranked, budgeted set\n");
        assert!(matches!(&stmts[0], Statement::Run { id, .. } if id == "SOP-O07"));
    }
}
