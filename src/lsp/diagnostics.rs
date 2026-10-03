//! Publish broken links and embeds as diagnostics.
//!
//! The graph is already patched with the buffer text by the time this runs;
//! each publish is `Graph::check_file` for one document, with ranges taken
//! from `Problem.span` converted against the document text.

use tower_lsp::lsp_types::{Diagnostic, DiagnosticSeverity, Range, Url};

use crate::graph::Problem;

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
            graph
                .check_file(&rel)
                .into_iter()
                .map(|p| to_diagnostic(&text, p))
                .collect::<Vec<_>>()
        })
        .await
        .unwrap_or_default();
    state
        .client
        .publish_diagnostics(uri.clone(), diagnostics, None)
        .await;
}

fn to_diagnostic(text: &str, problem: Problem) -> Diagnostic {
    Diagnostic {
        range: Range {
            start: offset_to_position(text, problem.span.start),
            end: offset_to_position(text, problem.span.end),
        },
        severity: Some(DiagnosticSeverity::ERROR),
        source: Some("kdb".into()),
        message: problem.reason,
        ..Diagnostic::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::ProblemKind;
    use std::path::PathBuf;
    use tower_lsp::lsp_types::Position;

    #[test]
    fn range_comes_from_span() {
        let text = "# A\n\nSee [x](x.md) here\n";
        let start = text.find("[x]").unwrap();
        let d = to_diagnostic(
            text,
            Problem {
                kind: ProblemKind::BrokenLink,
                file: PathBuf::from("a.md"),
                line: 3,
                column: 5,
                raw: "x.md".into(),
                reason: "target file not found: x.md".into(),
                span: start..start + "[x](x.md)".len(),
            },
        );
        assert_eq!(
            d.range,
            Range::new(Position::new(2, 4), Position::new(2, 13))
        );
        assert_eq!(d.severity, Some(DiagnosticSeverity::ERROR));
        assert_eq!(d.message, "target file not found: x.md");
    }
}
