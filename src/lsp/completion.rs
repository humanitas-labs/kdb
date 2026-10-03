//! Completion inside link targets.
//!
//! - `[text](|` or `[[|`: markdown files, as paths relative to the current
//!   file (`.md` kept for markdown links, dropped for wikilinks).
//! - `[text](file.md#|` or `[[file#|`: headings of the target file; bare `#`
//!   completes the current file's headings.
//!
//! A `kdb://` prefix switches to root-relative paths.

use std::path::Path;

use tower_lsp::lsp_types::{
    CompletionItem, CompletionItemKind, CompletionParams, CompletionResponse, CompletionTextEdit,
    Position, Range, TextEdit,
};

use crate::graph::parse::slug_anchor;
use crate::graph::{Graph, LinkKind, LinkTarget, Resolution};

use super::backend::{State, position_to_offset, relative_path, slash};

pub(super) async fn completion(state: &State, params: CompletionParams) -> Option<CompletionResponse> {
    let uri = params.text_document_position.text_document.uri;
    let pos = params.text_document_position.position;
    let rel = state.rel(&uri)?;
    let text = state.text(&uri).await?;
    let ctx = context(&text, pos)?;
    let items = state
        .with_graph(|graph| match &ctx.what {
            What::File(prefix) => files(graph, &rel, &ctx, prefix),
            What::Heading { file, prefix } => headings(graph, &rel, &ctx, file.as_deref(), prefix),
        })
        .await?;
    Some(CompletionResponse::Array(items))
}

/// What the cursor is completing, and the typed fragment to replace.
#[derive(Debug, PartialEq, Eq)]
struct Context {
    kind: LinkKind,
    root_relative: bool,
    edit: Range,
    what: What,
}

#[derive(Debug, PartialEq, Eq)]
enum What {
    /// Typed file prefix.
    File(String),
    /// Target file as typed (`None` for a bare `#`) and typed anchor prefix.
    Heading { file: Option<String>, prefix: String },
}

/// Find the nearest `[[` or `](` before the cursor on this line and parse the fragment.
fn context(text: &str, pos: Position) -> Option<Context> {
    let offset = position_to_offset(text, pos)?;
    let line_start = text[..offset].rfind('\n').map_or(0, |i| i + 1);
    let before = &text[line_start..offset];
    let wiki = before.rfind("[[");
    let md = before.rfind("](");
    let (start, kind) = match (wiki, md) {
        (Some(w), Some(m)) if w > m => (w, LinkKind::Wikilink),
        (Some(w), None) => (w, LinkKind::Wikilink),
        (_, Some(m)) => (m, LinkKind::Markdown),
        (None, None) => return None,
    };
    let mut fragment = &before[start + 2..];
    match kind {
        LinkKind::Wikilink => {
            if fragment.contains("]]") {
                return None;
            }
            fragment = fragment.split('|').next().unwrap_or(fragment);
        }
        LinkKind::Markdown => {
            if fragment.contains(')') {
                return None;
            }
        }
    }
    let fragment = fragment.trim();
    let (fragment, root_relative) = match fragment.strip_prefix("kdb://") {
        Some(rest) => (rest, true),
        None => (fragment, false),
    };
    let what = match fragment.split_once('#') {
        Some((file, anchor)) => What::Heading {
            file: Some(file.trim()).filter(|f| !f.is_empty()).map(str::to_owned),
            prefix: anchor.to_string(),
        },
        None => What::File(fragment.to_string()),
    };
    let target_col = text[line_start..line_start + start + 2].encode_utf16().count() as u32;
    let edit = Range::new(Position::new(pos.line, target_col), pos);
    Some(Context { kind, root_relative, edit, what })
}

fn files(graph: &Graph, source: &Path, ctx: &Context, prefix: &str) -> Vec<CompletionItem> {
    let source_dir = source.parent().unwrap_or(Path::new(""));
    let mut items: Vec<CompletionItem> = graph
        .docs()
        .filter_map(|doc| {
            let mut candidate = if ctx.root_relative {
                doc.rel.clone()
            } else {
                relative_path(source_dir, &doc.rel)
            };
            if ctx.kind == LinkKind::Wikilink {
                candidate.set_extension("");
            }
            let label = slash(&candidate);
            if !label.starts_with(prefix) {
                return None;
            }
            let insert = if ctx.root_relative { format!("kdb://{label}") } else { label.clone() };
            Some(CompletionItem {
                label,
                kind: Some(CompletionItemKind::FILE),
                detail: Some(slash(&doc.rel)),
                filter_text: Some(insert.clone()),
                text_edit: Some(CompletionTextEdit::Edit(TextEdit::new(ctx.edit, insert))),
                ..CompletionItem::default()
            })
        })
        .collect();
    items.sort_by(|a, b| a.label.cmp(&b.label));
    items
}

fn headings(
    graph: &Graph,
    source: &Path,
    ctx: &Context,
    file: Option<&str>,
    prefix: &str,
) -> Vec<CompletionItem> {
    let target = match file {
        None => source.to_path_buf(),
        Some(file) => {
            let resolve = |name: String| {
                let target = LinkTarget { file: Some(name), anchor: None, root_relative: ctx.root_relative };
                match graph.resolve(source, &target) {
                    Resolution::Ok { file, .. } => Some(file),
                    _ => None,
                }
            };
            let with_md = || {
                (ctx.kind == LinkKind::Markdown && Path::new(file).extension().is_none())
                    .then(|| resolve(format!("{file}.md")))
                    .flatten()
            };
            match resolve(file.to_string()).or_else(with_md) {
                Some(rel) => rel,
                None => return Vec::new(),
            }
        }
    };
    let Some(doc) = graph.doc(&target) else { return Vec::new() };
    let prefix = if prefix.trim().is_empty() { String::new() } else { slug_anchor(prefix) };
    let mut items: Vec<CompletionItem> = doc
        .headings
        .iter()
        .filter(|h| h.anchor.starts_with(&prefix))
        .map(|h| CompletionItem {
            label: h.title.clone(),
            kind: Some(CompletionItemKind::TEXT),
            text_edit: Some(CompletionTextEdit::Edit(TextEdit::new(ctx.edit, h.anchor.clone()))),
            ..CompletionItem::default()
        })
        .collect();
    items.sort_by(|a, b| a.label.cmp(&b.label));
    items
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx(line: &str) -> Option<Context> {
        let text = format!("# T\n\n{line}");
        context(&text, Position::new(2, line.encode_utf16().count() as u32))
    }

    #[test]
    fn detects_file_and_heading_contexts() {
        let c = ctx("See [[").unwrap();
        assert_eq!(c.kind, LinkKind::Wikilink);
        assert_eq!(c.what, What::File(String::new()));
        assert_eq!(c.edit, Range::new(Position::new(2, 6), Position::new(2, 6)));

        let c = ctx("See [[b#ta").unwrap();
        assert_eq!(c.what, What::Heading { file: Some("b".into()), prefix: "ta".into() });
        assert_eq!(c.edit.start.character, 6);

        let c = ctx("[x](docs/").unwrap();
        assert_eq!(c.kind, LinkKind::Markdown);
        assert_eq!(c.what, What::File("docs/".into()));
        assert!(!c.root_relative);

        let c = ctx("[x](kdb://a.md#").unwrap();
        assert!(c.root_relative);
        assert_eq!(c.what, What::Heading { file: Some("a.md".into()), prefix: String::new() });

        let c = ctx("[x](#").unwrap();
        assert_eq!(c.what, What::Heading { file: None, prefix: String::new() });

        let c = ctx("[[b|alias").unwrap();
        assert_eq!(c.what, What::File("b".into()));
    }

    #[test]
    fn ignores_closed_links_and_plain_text() {
        assert!(ctx("See [[b]] and").is_none());
        assert!(ctx("[x](b.md) tail").is_none());
        assert!(ctx("plain words").is_none());
    }
}
