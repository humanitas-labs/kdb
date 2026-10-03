//! Go-to-definition: the link under the cursor resolves to a file, and to a
//! heading line when the anchor resolved.

use tower_lsp::lsp_types::{
    GotoDefinitionParams, GotoDefinitionResponse, Location, Position, Range,
};

use crate::graph::Resolution;

use super::backend::{State, position_to_offset};

pub(super) async fn goto_definition(
    state: &State,
    params: GotoDefinitionParams,
) -> Option<GotoDefinitionResponse> {
    let uri = params.text_document_position_params.text_document.uri;
    let pos = params.text_document_position_params.position;
    let rel = state.rel(&uri)?;
    let text = state.text(&uri).await?;
    // Graph columns are 1-based bytes within the line, as tree-sitter reports them.
    let offset = position_to_offset(&text, pos)?;
    let line_start = text[..offset].rfind('\n').map_or(0, |i| i + 1);
    let column = offset - line_start + 1;

    state
        .with_graph(|graph| {
            let link = graph.doc(&rel)?.link_at(pos.line as usize + 1, column)?;
            let Resolution::Ok { file, heading } = graph.resolve(&rel, &link.target) else {
                return None;
            };
            let target = heading
                .map(|h| {
                    Position::new(h.line.saturating_sub(1) as u32, h.column.saturating_sub(1) as u32)
                })
                .unwrap_or_default();
            Some(GotoDefinitionResponse::Scalar(Location {
                uri: state.uri_for(&file)?,
                range: Range::new(target, target),
            }))
        })
        .await
        .flatten()
}
