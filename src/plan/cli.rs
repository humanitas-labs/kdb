//! clap argument types and entry points for the planning subcommands.

use anyhow::{Result, bail};
use clap::Args;

use crate::workspace::Workspace;

#[derive(Args, Debug)]
pub struct ProjectsArgs {}
#[derive(Args, Debug)]
pub struct SpacesArgs {}
#[derive(Args, Debug)]
pub struct TasksArgs {}
#[derive(Args, Debug)]
pub struct CyclesArgs {}
#[derive(Args, Debug)]
pub struct LabelsArgs {}
#[derive(Args, Debug)]
pub struct StatusesArgs {}

pub fn projects(ws: &Workspace, args: ProjectsArgs) -> Result<()> {
    let _ = (ws, args);
    bail!("projects: not implemented")
}
pub fn spaces(ws: &Workspace, args: SpacesArgs) -> Result<()> {
    let _ = (ws, args);
    bail!("spaces: not implemented")
}
pub fn tasks(ws: &Workspace, args: TasksArgs) -> Result<()> {
    let _ = (ws, args);
    bail!("tasks: not implemented")
}
pub fn cycles(ws: &Workspace, args: CyclesArgs) -> Result<()> {
    let _ = (ws, args);
    bail!("cycles: not implemented")
}
pub fn labels(ws: &Workspace, args: LabelsArgs) -> Result<()> {
    let _ = (ws, args);
    bail!("labels: not implemented")
}
pub fn statuses(ws: &Workspace, args: StatusesArgs) -> Result<()> {
    let _ = (ws, args);
    bail!("statuses: not implemented")
}
