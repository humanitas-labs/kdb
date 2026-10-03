//! clap flags for `render` without a FILE argument.

use anyhow::{Result, bail};
use clap::Args;

use crate::db;
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
    #[arg(short = 'n', long, allow_negative_numbers = true)]
    pub limit: Option<i64>,
}

pub fn run(ws: &Workspace, args: MaterializeArgs) -> Result<()> {
    let selectors = [args.all, args.project.is_some(), args.space.is_some()];
    match selectors.iter().filter(|b| **b).count() {
        0 => bail!("missing file argument — pass a markdown file, --project <slug>, --space <slug>, or --all"),
        1 => {}
        _ => bail!("--project, --space, and --all are mutually exclusive"),
    }
    let conn = db::open(&ws.root)?;
    let written = if args.all {
        super::materialize_all(&conn, &ws.root, args.limit)?
    } else if let Some(slug) = &args.space {
        vec![super::materialize_space(&conn, &ws.root, slug)?]
    } else if let Some(slug) = &args.project {
        vec![super::materialize_project(&conn, &ws.root, slug, args.limit)?]
    } else {
        unreachable!("selector count checked above")
    };
    for p in &written {
        println!("wrote {}", p.display());
    }
    Ok(())
}
