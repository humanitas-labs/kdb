//! tower-lsp language server over the in-memory [`crate::graph::Graph`].

use std::path::PathBuf;

use anyhow::{Result, bail};

/// Serve LSP over stdio. `path` locates the workspace (defaults to cwd).
pub async fn serve(path: Option<PathBuf>) -> Result<()> {
    let _ = path;
    bail!("lsp: not implemented")
}
