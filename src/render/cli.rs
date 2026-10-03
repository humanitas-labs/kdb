//! clap flags for `render` without a FILE argument.

use anyhow::{Result, bail};
use clap::Args;

use crate::workspace::Workspace;

#[derive(Args, Debug)]
pub struct MaterializeArgs {
    /// Materialize the board for a single project slug
    #[arg(short = 'P', long)]
    pub project: Option<String>,
    /// Materialize the rollup board for a space slug
    #[arg(short = 'S', long)]
    pub space: Option<String>,
    /// Materialize every non-archived project
    #[arg(long)]
    pub all: bool,
    /// Cap per-task file materialization to N top-priority open tasks (defaults to `meta.top_n`)
    #[arg(short = 'n', long)]
    pub limit: Option<i64>,
}

pub fn run(ws: &Workspace, args: MaterializeArgs) -> Result<()> {
    let _ = (ws, args);
    bail!("render: not implemented")
}
