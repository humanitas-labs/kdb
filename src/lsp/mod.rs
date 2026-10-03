//! tower-lsp language server over the in-memory [`crate::graph::Graph`].
//!
//! Diagnostics for broken links and embeds, go-to-definition on links, and
//! file/heading completion inside link targets. No hover, symbols, or formatting.

mod backend;
mod completion;
mod definition;
mod diagnostics;

use std::env;
use std::path::PathBuf;

use anyhow::{Context, Result};
use tower_lsp::{LspService, Server};

use crate::workspace::Workspace;
use backend::Backend;

/// Serve LSP over stdio. `path` locates the workspace (defaults to cwd); when
/// neither contains `.kdb/`, the client's root URI at `initialize` is tried.
pub async fn serve(path: Option<PathBuf>) -> Result<()> {
    let start = match path {
        Some(path) => path,
        None => env::current_dir().context("failed to read current directory")?,
    };
    let ws = Workspace::find(&start).ok();
    let (service, socket) = LspService::new(move |client| Backend::new(client, ws.clone()));
    Server::new(tokio::io::stdin(), tokio::io::stdout(), socket).serve(service).await;
    Ok(())
}
