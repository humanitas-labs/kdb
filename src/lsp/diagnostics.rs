//! Publish broken links and embeds as diagnostics.
//!
//! The graph is already patched with the buffer text by the time this runs;
//! each publish is `Graph::check_file` for one document, with ranges taken
//! from the matching `Link.span` converted against the document text.

use tower_lsp::lsp_types::{Diagnostic, DiagnosticSeverity, Position, Range, Url};

use crate::graph::{Doc, Problem};

use super::backend::{State, offset_to_position};

/// Re-publish diagnostics for every open document. Inbound breakage in one
/// file changes the diagnostics of others, so a change anywhere refreshes all.
pub(super) async fn publish_all(state: &State) {
    for uri in state.open_uris().await {
        publish(state, &uri).await;
    }
}

async fn publish(state: &State, uri: &Url) {
    let Some(rel) = state.rel(uri) else { return };
    let text = state.text(uri).await.unwrap_or_default();
    let diagnostics = state
        .with_graph(|graph| {
            let doc = graph.doc(&rel);
            graph
                .check_file(&rel)
                .into_iter()
                .map(|p| to_diagnostic(&text, doc, p))
                .collect::<Vec<_>>()
        })
        .await
        .unwrap_or_default();
    state.client.publish_diagnostics(uri.clone(), diagnostics, None).await;
}

fn to_diagnostic(text: &str, doc: Option<&Doc>, problem: Problem) -> Diagnostic {
    Diagnostic {
        range: problem_range(text, doc, &problem),
        severity: Some(DiagnosticSeverity::ERROR),
        source: Some("kdb".into()),
        message: problem.reason,
        ..Diagnostic::default()
    }
}

/// The span of the link the problem refers to (matched by line and column);
/// falls back to the problem's position plus the raw text width.
fn problem_range(text: &str, doc: Option<&Doc>, problem: &Problem) -> Range {
    let link = doc.and_then(|d| {
        d.links.iter().find(|l| l.line == problem.line && l.column == problem.column)
    });
    match link {
        Some(link) => Range {
            start: offset_to_position(text, link.span.start),
            end: offset_to_position(text, link.span.end),
        },
        None => {
            let line = problem.line.saturating_sub(1) as u32;
            let col = problem.column.saturating_sub(1) as u32;
            let width = problem.raw.encode_utf16().count().max(1) as u32;
            Range { start: Position::new(line, col), end: Position::new(line, col + width) }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::{Link, LinkKind, LinkTarget, ProblemKind};
    use std::path::PathBuf;

    fn problem(line: usize, column: usize, raw: &str) -> Problem {
        Problem {
            kind: ProblemKind::BrokenLink,
            file: PathBuf::from("a.md"),
            line,
            column,
            raw: raw.into(),
            reason: "target file not found: x.md".into(),
        }
    }

    #[test]
    fn range_prefers_link_span() {
        let text = "# A\n\nSee [x](x.md) here\n";
        let start = text.find("[x]").unwrap();
        let doc = Doc {
            rel: PathBuf::from("a.md"),
            headings: vec![],
            links: vec![Link {
                kind: LinkKind::Markdown,
                embed: false,
                raw: "x.md".into(),
                target: LinkTarget { file: Some("x.md".into()), anchor: None, root_relative: false },
                line: 3,
                column: 5,
                span: start..start + "[x](x.md)".len(),
            }],
        };
        let d = to_diagnostic(text, Some(&doc), problem(3, 5, "x.md"));
        assert_eq!(d.range, Range::new(Position::new(2, 4), Position::new(2, 13)));
        assert_eq!(d.severity, Some(DiagnosticSeverity::ERROR));
        assert_eq!(d.message, "target file not found: x.md");
    }

    #[test]
    fn range_falls_back_to_raw_width() {
        let d = to_diagnostic("", None, problem(3, 5, "x.md"));
        assert_eq!(d.range, Range::new(Position::new(2, 4), Position::new(2, 8)));
    }
}
