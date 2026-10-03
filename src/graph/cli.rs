//! clap argument types and entry points for `check`, `outline`, `refs`, and `render <file>`.

use std::path::Path;

use anyhow::{Result, bail};
use clap::Args;

use crate::workspace::Workspace;

#[derive(Args, Debug)]
pub struct CheckArgs {}

#[derive(Args, Debug)]
pub struct OutlineArgs {}

#[derive(Args, Debug)]
pub struct RefsArgs {}

pub fn check(ws: &Workspace, args: CheckArgs) -> Result<()> {
    let _ = (ws, args);
    bail!("check: not implemented")
}

pub fn outline(ws: &Workspace, args: OutlineArgs) -> Result<()> {
    let _ = (ws, args);
    bail!("outline: not implemented")
}

pub fn refs(ws: &Workspace, args: RefsArgs) -> Result<()> {
    let _ = (ws, args);
    bail!("refs: not implemented")
}

/// `render <file>`: resolve `![[]]` embeds and print the result to stdout.
pub fn render_file(ws: &Workspace, file: &Path) -> Result<()> {
    let _ = (ws, file);
    bail!("render <file>: not implemented")
}
