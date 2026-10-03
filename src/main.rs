//! kdb — markdown link graph, task tracker, and LSP for a knowledge workspace.
//!
//! This file is dispatch only. Each domain owns its clap argument types and a
//! `run()` in its own `cli.rs`; nothing here should need to change when a
//! domain grows.

mod db;
mod graph;
mod lsp;
mod plan;
mod render;
mod workspace;

use std::path::PathBuf;

use anyhow::Result;
use clap::{Args, Parser, Subcommand};

use workspace::Workspace;

#[derive(Parser)]
#[command(name = "kdb", version, about = "Markdown link graph, task tracker, and LSP for a knowledge workspace")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Initialize a kdb workspace in a directory
    Init {
        /// Directory to initialize (defaults to the current directory)
        path: Option<PathBuf>,
    },
    /// Print the absolute path of the workspace root
    Root,
    /// Report broken links, broken embeds, and orphan files
    Check(graph::cli::CheckArgs),
    /// Print the heading outline for files and/or directories
    Outline(graph::cli::OutlineArgs),
    /// Find inbound references to a file or heading
    Refs(graph::cli::RefsArgs),
    /// Resolve markdown embeds to stdout, or materialize task boards
    Render(RenderArgs),
    /// Run the language server over stdio
    Lsp {
        /// Workspace path (defaults to the current directory)
        path: Option<PathBuf>,
    },
    /// Manage projects
    Projects(plan::cli::ProjectsArgs),
    /// Manage spaces (named groupings of projects)
    Spaces(plan::cli::SpacesArgs),
    /// Manage tasks
    Tasks(plan::cli::TasksArgs),
    /// Manage cycles
    Cycles(plan::cli::CyclesArgs),
    /// Manage labels
    Labels(plan::cli::LabelsArgs),
    /// Manage task and project statuses
    Statuses(plan::cli::StatusesArgs),
}

/// `render` is shared by two domains: with a FILE it resolves embeds (graph);
/// without one it materializes task boards (render).
#[derive(Args)]
struct RenderArgs {
    /// File to render; resolves `![[]]` embeds to stdout
    file: Option<PathBuf>,
    #[command(flatten)]
    materialize: render::cli::MaterializeArgs,
}

fn main() {
    if let Err(err) = run() {
        eprintln!("{err:#}");
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Command::Init { path } => {
            let root = workspace::init(path.as_deref())?;
            println!("initialized workspace at {}", root.display());
            Ok(())
        }
        Command::Root => {
            let ws = Workspace::from_cwd()?;
            println!("{}", ws.root.display());
            Ok(())
        }
        Command::Check(args) => graph::cli::check(&Workspace::from_cwd()?, args),
        Command::Outline(args) => graph::cli::outline(&Workspace::from_cwd()?, args),
        Command::Refs(args) => graph::cli::refs(&Workspace::from_cwd()?, args),
        Command::Render(RenderArgs { file: Some(file), .. }) => {
            graph::cli::render_file(&Workspace::from_cwd()?, &file)
        }
        Command::Render(RenderArgs { file: None, materialize }) => {
            render::cli::run(&Workspace::from_cwd()?, materialize)
        }
        Command::Lsp { path } => {
            let rt = tokio::runtime::Runtime::new()?;
            rt.block_on(lsp::serve(path))
        }
        Command::Projects(args) => plan::cli::projects(&Workspace::from_cwd()?, args),
        Command::Spaces(args) => plan::cli::spaces(&Workspace::from_cwd()?, args),
        Command::Tasks(args) => plan::cli::tasks(&Workspace::from_cwd()?, args),
        Command::Cycles(args) => plan::cli::cycles(&Workspace::from_cwd()?, args),
        Command::Labels(args) => plan::cli::labels(&Workspace::from_cwd()?, args),
        Command::Statuses(args) => plan::cli::statuses(&Workspace::from_cwd()?, args),
    }
}
