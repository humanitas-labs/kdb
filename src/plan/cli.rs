//! clap argument types and entry points for the planning subcommands. The
//! flags, help text, output and error messages follow v1 `kdb`; additions are
//! the dependency commands (iss-0071) and status icons (iss-0070).

use std::collections::HashMap;
use std::env;

use anyhow::{Context, Result, bail};
use clap::{Args, Subcommand};
use rusqlite::Connection;
use serde::Serialize;

use super::statuses::Kind;
use super::tasks::{ListFilters, MoveTarget, TaskView};
use super::{TaskId, cycles, deps, labels, projects, spaces, statuses, tasks};
use crate::db;
use crate::workspace::Workspace;

fn json<T: Serialize>(v: &T) -> Result<()> {
    println!("{}", serde_json::to_string_pretty(v).context("failed to serialize as JSON")?);
    Ok(())
}

/// Pretty JSON of `v`, or the text rendering.
fn emit<T: Serialize>(as_json: bool, v: &T, text: impl FnOnce() -> String) -> Result<()> {
    if as_json {
        return json(v);
    }
    print!("{}", text());
    Ok(())
}

fn space_id(conn: &Connection, slug: &str) -> Result<i64> {
    Ok(spaces::require(conn, slug)?.id)
}

/// `--project` slug, else the project registered at the current directory.
fn resolve_project(conn: &Connection, ws: &Workspace, explicit: Option<&str>) -> Result<projects::Project> {
    if let Some(slug) = explicit {
        return projects::require(conn, slug);
    }
    let cwd = env::current_dir().context("failed to read current directory")?;
    let cwd = cwd.canonicalize().unwrap_or(cwd);
    projects::resolve_active(conn, &ws.root, &cwd)?
        .context("no project for current directory — pass -P/--project or register one with `kdb projects add`")
}

/// `None`: unchanged; `Some(None)`: clear; `Some(Some(id))`: set.
fn resolve_cycle(conn: &Connection, key: Option<&str>) -> Result<Option<Option<i64>>> {
    match key {
        None => Ok(None),
        Some("") => Ok(Some(None)),
        Some(k) => Ok(Some(Some(cycles::require(conn, k)?.id))),
    }
}

fn resolve_parent(conn: &Connection, id: Option<&str>) -> Result<Option<Option<i64>>> {
    match id {
        None => Ok(None),
        Some("") => Ok(Some(None)),
        Some(s) => {
            let view = tasks::get(conn, &TaskId::parse(s)?)?.with_context(|| format!("parent task not found: {s}"))?;
            Ok(Some(Some(view.task.id)))
        }
    }
}

fn task_by_id(conn: &Connection, id: &str) -> Result<TaskView> {
    tasks::get(conn, &TaskId::parse(id)?)?.with_context(|| format!("task not found: {id}"))
}

/// "open" (every non-closed status), "all" (`None`), or a comma list.
fn parse_statuses(conn: &Connection, s: &str) -> Result<Option<Vec<String>>> {
    if s == "all" {
        return Ok(None);
    }
    let known = statuses::list(conn, Kind::Task)?;
    if s == "open" {
        return Ok(Some(known.into_iter().filter(|st| !st.flag).map(|st| st.slug).collect()));
    }
    let parts: Vec<String> = s.split(',').map(str::trim).filter(|p| !p.is_empty()).map(String::from).collect();
    for p in &parts {
        if !known.iter().any(|k| k.slug == *p) {
            let names: Vec<&str> = known.iter().map(|k| k.slug.as_str()).collect();
            bail!("invalid status '{p}' (expected {} or 'all'/'open')", names.join(", "));
        }
    }
    Ok(Some(parts))
}

fn blockers_map(conn: &Connection, rows: &[TaskView]) -> Result<HashMap<i64, Vec<String>>> {
    let mut map = HashMap::new();
    for t in rows.iter().filter(|t| t.blocked) {
        let ids = deps::blockers_of(conn, t.task.id)?.iter().map(TaskView::external_id).collect();
        map.insert(t.task.id, ids);
    }
    Ok(map)
}

// --- projects -----------------------------------------------------------------

#[derive(Args, Debug)]
pub struct ProjectsArgs {
    #[command(subcommand)]
    action: ProjectsCmd,
}

#[derive(Subcommand, Debug)]
enum ProjectsCmd {
    /// List projects.
    #[command(alias = "ls")]
    List {
        /// Include archived projects.
        #[arg(short = 'a', long)]
        all: bool,
        /// Filter to projects in this space (by slug).
        #[arg(short = 's', long)]
        space: Option<String>,
        /// Emit structured JSON output.
        #[arg(long)]
        json: bool,
    },
    /// Add a new project.
    Add {
        /// Unique slug (e.g. "hermaeus").
        slug: String,
        /// 2–6 char uppercase alias used in task ids (e.g. "HRM").
        #[arg(short = 'a', long)]
        alias: String,
        /// Relative path from kdb root (e.g. "projects/hermaeus").
        #[arg(short = 'p', long)]
        path: String,
        /// Display name (defaults to slug).
        #[arg(short = 'n', long)]
        name: Option<String>,
        /// Optional description.
        #[arg(short = 'd', long)]
        description: Option<String>,
        /// Assign to this space (by slug).
        #[arg(short = 's', long)]
        space: Option<String>,
    },
    /// Edit an existing project.
    Edit {
        /// Project slug to edit.
        slug: String,
        /// New 2–6 char uppercase alias.
        #[arg(short = 'a', long)]
        alias: Option<String>,
        #[arg(short = 'n', long)]
        name: Option<String>,
        #[arg(short = 'p', long)]
        path: Option<String>,
        /// Status slug (must exist in project_statuses).
        #[arg(long)]
        status: Option<String>,
        #[arg(short = 'd', long)]
        description: Option<String>,
        /// Assign to this space (by slug); pass "" to detach (make loose).
        #[arg(short = 's', long)]
        space: Option<String>,
    },
    /// Show a project.
    Show {
        /// Project slug to show.
        slug: String,
        /// Emit structured JSON output.
        #[arg(long)]
        json: bool,
    },
}

pub fn projects(ws: &Workspace, args: ProjectsArgs) -> Result<()> {
    let conn = db::open(&ws.root)?;
    match args.action {
        ProjectsCmd::List { all, space, json: as_json } => {
            if let Some(s) = &space {
                spaces::require(&conn, s)?;
            }
            let rows = projects::list(&conn, all, space.as_deref())?;
            emit(as_json, &rows, || projects::render_list(&rows))
        }
        ProjectsCmd::Add { slug, alias, path, name, description, space } => {
            let space_id = match space.as_deref() {
                Some(s) if !s.is_empty() => Some(space_id(&conn, s)?),
                _ => None,
            };
            let p = projects::add(
                &conn,
                projects::AddArgs { slug: &slug, alias: &alias, name: name.as_deref(), path: &path, description: description.as_deref(), space_id },
            )?;
            println!("added project {} [{}] ({})", p.slug, p.alias, p.path);
            Ok(())
        }
        ProjectsCmd::Edit { slug, alias, name, path, status, description, space } => {
            let space_id = match space.as_deref() {
                None => None,
                Some("") => Some(None),
                Some(s) => Some(Some(space_id(&conn, s)?)),
            };
            let p = projects::edit(
                &conn,
                &slug,
                projects::EditArgs {
                    alias: alias.as_deref(),
                    name: name.as_deref(),
                    path: path.as_deref(),
                    status: status.as_deref(),
                    description: description.as_deref(),
                    space_id,
                },
            )?;
            println!("updated project {}", p.slug);
            Ok(())
        }
        ProjectsCmd::Show { slug, json: as_json } => {
            let p = projects::require(&conn, &slug)?;
            emit(as_json, &p, || projects::render_show(&p))
        }
    }
}

// --- spaces -------------------------------------------------------------------

#[derive(Args, Debug)]
pub struct SpacesArgs {
    #[command(subcommand)]
    action: SpacesCmd,
}

#[derive(Subcommand, Debug)]
enum SpacesCmd {
    /// List spaces.
    #[command(alias = "ls")]
    List {
        /// Include archived spaces.
        #[arg(short = 'a', long)]
        all: bool,
        /// Emit structured JSON output.
        #[arg(long)]
        json: bool,
    },
    /// Add a new space.
    Add {
        /// Unique slug (e.g. "iceberg").
        slug: String,
        /// Uppercase task-id alias (e.g. "ICE"). Used in ids for the space's
        /// own tasks; must not collide with any project or space alias.
        #[arg(short = 'a', long)]
        alias: String,
        /// Display name (defaults to slug).
        #[arg(short = 'n', long)]
        name: Option<String>,
        /// Optional relative path from kdb root (e.g. "projects/iceberg").
        #[arg(short = 'p', long)]
        path: Option<String>,
        /// Optional description.
        #[arg(short = 'd', long)]
        description: Option<String>,
    },
    /// Edit an existing space.
    Edit {
        /// Space slug to edit.
        slug: String,
        #[arg(short = 'n', long)]
        name: Option<String>,
        /// Uppercase task-id alias.
        #[arg(short = 'a', long)]
        alias: Option<String>,
        #[arg(short = 'p', long)]
        path: Option<String>,
        /// Status slug (must exist in project_statuses).
        #[arg(long)]
        status: Option<String>,
        #[arg(short = 'd', long)]
        description: Option<String>,
    },
    /// Show a space and its member projects.
    Show {
        /// Space slug to show.
        slug: String,
        /// Emit structured JSON output.
        #[arg(long)]
        json: bool,
    },
}

pub fn spaces(ws: &Workspace, args: SpacesArgs) -> Result<()> {
    let conn = db::open(&ws.root)?;
    match args.action {
        SpacesCmd::List { all, json: as_json } => {
            let rows = spaces::list(&conn, all)?;
            emit(as_json, &rows, || spaces::render_list(&rows))
        }
        SpacesCmd::Add { slug, alias, name, path, description } => {
            let s = spaces::add(
                &conn,
                spaces::AddArgs { slug: &slug, name: name.as_deref(), alias: &alias, path: path.as_deref(), description: description.as_deref() },
            )?;
            println!("added space {}", s.slug);
            Ok(())
        }
        SpacesCmd::Edit { slug, name, alias, path, status, description } => {
            let s = spaces::edit(
                &conn,
                &slug,
                spaces::EditArgs {
                    name: name.as_deref(),
                    alias: alias.as_deref(),
                    path: path.as_deref(),
                    status: status.as_deref(),
                    description: description.as_deref(),
                },
            )?;
            println!("updated space {}", s.slug);
            Ok(())
        }
        SpacesCmd::Show { slug, json: as_json } => {
            let s = spaces::require(&conn, &slug)?;
            let members = projects::list(&conn, true, Some(&slug))?;
            if as_json {
                return json(&serde_json::json!({ "space": s, "projects": members }));
            }
            print!("{}", spaces::render_show(&s));
            println!("\nprojects:");
            if members.is_empty() {
                println!("  (none)");
            } else {
                print!("{}", projects::render_list(&members));
            }
            Ok(())
        }
    }
}

// --- tasks --------------------------------------------------------------------

#[derive(Args, Debug)]
pub struct TasksArgs {
    #[command(subcommand)]
    action: Option<TasksCmd>,
}

#[derive(Subcommand, Debug)]
enum TasksCmd {
    /// List tasks.
    #[command(alias = "ls")]
    List {
        /// Comma-separated status slugs, or "open" (every non-closed status,
        /// resolved at runtime — the default) or "all".
        #[arg(short = 's', long, default_value = "open")]
        status: String,
        /// Filter by project slug.
        #[arg(short = 'P', long)]
        project: Option<String>,
        /// Filter to all projects in this space (by slug); combine with -P to intersect.
        #[arg(short = 'S', long)]
        space: Option<String>,
        /// Filter by cycle key.
        #[arg(short = 'c', long)]
        cycle: Option<String>,
        /// Filter by priority (1-5).
        #[arg(short = 'p', long)]
        priority: Option<i64>,
        /// Limit to N rows.
        #[arg(short = 'n', long)]
        limit: Option<i64>,
        /// Include subtasks (rows with a parent). Off by default —
        /// subtasks surface inside their parent task's view instead.
        #[arg(long)]
        include_children: bool,
        /// Only tasks with an unresolved blocker (own or inherited).
        #[arg(long, conflicts_with = "ready")]
        blocked: bool,
        /// Only tasks with no unresolved blocker.
        #[arg(long, conflicts_with = "blocked")]
        ready: bool,
        /// Emit structured JSON output.
        #[arg(long)]
        json: bool,
    },
    /// Add a new task.
    Add {
        /// Task title.
        title: String,
        /// Project slug (defaults to the active project).
        #[arg(short = 'P', long)]
        project: Option<String>,
        /// Space slug — file the task as space-native (owned by the space,
        /// not a project). Mutually exclusive with -P.
        #[arg(short = 'S', long, conflicts_with = "project")]
        space: Option<String>,
        /// Body text.
        #[arg(short = 'b', long)]
        body: Option<String>,
        /// Priority (1-5, default 3).
        #[arg(short = 'p', long)]
        priority: Option<i64>,
        /// Cycle key (e.g. C-14).
        #[arg(short = 'c', long)]
        cycle: Option<String>,
        /// Parent task id.
        #[arg(long)]
        parent: Option<String>,
        /// Insert immediately before this task (inherits its parent unless --parent given).
        #[arg(long, conflicts_with = "after")]
        before: Option<String>,
        /// Depend on this task (repeatable): adds a `blocks` edge. A single
        /// --after with the same owner also positions the new task right after it.
        #[arg(long)]
        after: Vec<String>,
    },
    /// Move a task to a new position within its sibling list.
    #[command(alias = "mv")]
    Move {
        /// Task id (e.g. KDB-4).
        id: String,
        /// Place immediately before this task.
        #[arg(long, conflicts_with_all = ["after", "top", "bottom"])]
        before: Option<String>,
        /// Place immediately after this task.
        #[arg(long, conflicts_with_all = ["before", "top", "bottom"])]
        after: Option<String>,
        /// Move to the top of the sibling list.
        #[arg(long, conflicts_with_all = ["before", "after", "bottom"])]
        top: bool,
        /// Move to the bottom of the sibling list.
        #[arg(long, conflicts_with_all = ["before", "after", "top"])]
        bottom: bool,
    },
    /// Edit an existing task.
    Edit {
        /// Task id (e.g. hermaeus-42).
        id: String,
        #[arg(short = 't', long)]
        title: Option<String>,
        #[arg(short = 'b', long)]
        body: Option<String>,
        #[arg(short = 'p', long)]
        priority: Option<i64>,
        /// Set cycle key (use empty string to clear).
        #[arg(short = 'c', long)]
        cycle: Option<String>,
        /// Set parent task id (use empty string to clear).
        #[arg(long)]
        parent: Option<String>,
        /// Set status slug (must exist in task_statuses).
        #[arg(short = 's', long)]
        status: Option<String>,
    },
    /// View a task.
    #[command(alias = "show")]
    View {
        id: String,
        #[arg(long)]
        json: bool,
    },
    /// Delete a task. Soft-delete by default (sets `deleted_at`,
    /// hides from list/render); `--hard` removes the row + subtree
    /// permanently.
    #[command(alias = "d", alias = "rm")]
    Delete {
        id: String,
        /// Permanently delete the row and its entire subtree.
        #[arg(long)]
        hard: bool,
    },
    /// Restore a soft-deleted task (and its subtree).
    Restore { id: String },
    /// Permanently delete tasks matching filters (and their subtrees).
    /// Requires at least one of `--status` or `--deleted`.
    Purge {
        /// Limit to a project slug.
        #[arg(short = 'P', long)]
        project: Option<String>,
        /// Match tasks with this status (e.g. `done`).
        #[arg(short = 's', long)]
        status: Option<String>,
        /// Match soft-deleted tasks (`deleted_at IS NOT NULL`).
        #[arg(long)]
        deleted: bool,
        /// Show what would be deleted without making changes.
        #[arg(long)]
        dry_run: bool,
    },
    /// Mark a task as done.
    Done { id: String },
    /// Mark a task as parked.
    Park { id: String },
    /// Reopen a parked or done task.
    Reopen { id: String },
    /// Manage labels on a task.
    Label {
        #[command(subcommand)]
        action: TaskLabelCmd,
    },
    /// Manage dependencies between tasks.
    Deps {
        #[command(subcommand)]
        action: TaskDepsCmd,
    },
    /// Open tasks that can start now: no unresolved blocker, not in progress,
    /// not hidden (parked). Ordered by priority, then owner and order key.
    Ready {
        /// Limit to a project slug.
        #[arg(short = 'P', long)]
        project: Option<String>,
        /// Limit to a space (its projects and its own tasks).
        #[arg(short = 'S', long)]
        space: Option<String>,
        /// Limit to a cycle key.
        #[arg(short = 'c', long)]
        cycle: Option<String>,
        /// Limit to N rows.
        #[arg(short = 'n', long)]
        limit: Option<i64>,
        /// Emit structured JSON output.
        #[arg(long)]
        json: bool,
    },
}

#[derive(Subcommand, Debug)]
enum TaskLabelCmd {
    /// Attach one or more labels to a task (unknown slugs are created).
    Add {
        /// Task id (e.g. HRM-0120).
        id: String,
        /// Label slugs to attach.
        #[arg(required = true)]
        labels: Vec<String>,
    },
    /// Detach one or more labels from a task.
    Rm {
        id: String,
        #[arg(required = true)]
        labels: Vec<String>,
    },
}

#[derive(Subcommand, Debug)]
enum TaskDepsCmd {
    /// Make a task depend on one or more blockers.
    Add {
        /// The dependent task id.
        id: String,
        /// Ids of the tasks it waits on.
        #[arg(required = true)]
        blockers: Vec<String>,
        /// Edge kind: `blocks` (affects readiness) or `related` (informational).
        #[arg(long, default_value = "blocks", value_parser = deps::KINDS.to_vec())]
        kind: String,
    },
    /// Remove dependencies.
    Rm {
        id: String,
        #[arg(required = true)]
        blockers: Vec<String>,
    },
    /// Show what a task waits on and what waits on it.
    Show { id: String },
}

pub fn tasks(ws: &Workspace, args: TasksArgs) -> Result<()> {
    let mut conn = db::open(&ws.root)?;
    let action = args.action.unwrap_or(TasksCmd::List {
        status: "open".into(),
        project: None,
        space: None,
        cycle: None,
        priority: None,
        limit: None,
        include_children: false,
        blocked: false,
        ready: false,
        json: false,
    });
    match action {
        TasksCmd::List { status, project, space, cycle, priority, limit, include_children, blocked, ready, json: as_json } => {
            let statuses = parse_statuses(&conn, &status)?;
            let space = match space.as_deref() {
                Some("all") | Some("") | None => None,
                Some(s) => Some(spaces::require(&conn, s)?.slug),
            };
            let project = project.filter(|p| p != "all" && !p.is_empty());
            let rows = tasks::list(
                &conn,
                ListFilters {
                    statuses: statuses.as_deref(),
                    project_slug: project.as_deref(),
                    space_slug: space.as_deref(),
                    cycle_key: cycle.as_deref(),
                    priority,
                    limit,
                    top_level_only: !include_children,
                    blocked: if blocked { Some(true) } else if ready { Some(false) } else { None },
                    by_priority: false,
                },
            )?;
            let blockers = blockers_map(&conn, &rows)?;
            emit(as_json, &rows, || tasks::render_list(&rows, &blockers))
        }
        TasksCmd::Ready { project, space, cycle, limit, json: as_json } => {
            if let Some(s) = &space {
                spaces::require(&conn, s)?;
            }
            let open: Vec<String> = statuses::list(&conn, Kind::Task)?
                .into_iter()
                .filter(|s| !s.flag && !s.is_hidden && s.slug != "in_progress")
                .map(|s| s.slug)
                .collect();
            let rows = tasks::list(
                &conn,
                ListFilters {
                    statuses: Some(&open),
                    project_slug: project.as_deref(),
                    space_slug: space.as_deref(),
                    cycle_key: cycle.as_deref(),
                    limit,
                    blocked: Some(false),
                    by_priority: true,
                    ..Default::default()
                },
            )?;
            emit(as_json, &rows, || tasks::render_list(&rows, &HashMap::new()))
        }
        TasksCmd::Add { title, project, space, body, priority, cycle, parent, before, after } => {
            let (owner_project, owner_space) = match space.as_deref() {
                Some(slug) => {
                    let sp = spaces::require(&conn, slug)?;
                    if sp.alias.is_none() {
                        bail!("space {slug} has no alias — set one with `kdb spaces edit {slug} --alias <ABC>`");
                    }
                    (None, Some(sp.id))
                }
                None => (Some(resolve_project(&conn, ws, project.as_deref())?.id), None),
            };
            let cycle_id = resolve_cycle(&conn, cycle.as_deref())?.flatten();
            let explicit_parent = resolve_parent(&conn, parent.as_deref())?;
            // Positional anchor: --before, or a single --after that shares the owner.
            let anchor = match (&before, after.as_slice()) {
                (Some(b), _) => Some((b.as_str(), false)),
                (None, [a]) => Some((a.as_str(), true)),
                _ => None,
            };
            let mut position = None;
            if let Some((raw, is_after)) = anchor {
                let view = tasks::get(&conn, &TaskId::parse(raw)?)?.with_context(|| format!("anchor task not found: {raw}"))?;
                let same_owner = view.task.project_id == owner_project && view.task.space_id == owner_space;
                if !same_owner && !is_after {
                    bail!("anchor task {} belongs to {}, not the target owner", view.external_id(), view.project_slug);
                }
                if same_owner {
                    position = Some((view, is_after));
                }
            }
            let parent_id = match (explicit_parent, &position) {
                (Some(explicit), _) => explicit,
                (None, Some((anchor, _))) => anchor.task.parent_id,
                (None, None) => None,
            };
            let order = position.as_ref().map(|(a, side)| tasks::order_key_adjacent(&conn, a, *side)).transpose()?;
            let view = tasks::add(
                &mut conn,
                tasks::AddArgs {
                    project_id: owner_project,
                    space_id: owner_space,
                    title: &title,
                    body: body.as_deref(),
                    priority,
                    cycle_id,
                    parent_id,
                    order: order.as_deref(),
                },
            )?;
            for raw in &after {
                deps::add(&conn, view.task.id, task_by_id(&conn, raw)?.task.id, "blocks")?;
            }
            materialize_for(&conn, ws, &view)?;
            println!("added task {}", view.external_id());
            Ok(())
        }
        TasksCmd::Move { id, before, after, top, bottom } => {
            let parsed = TaskId::parse(&id)?;
            if [before.is_some(), after.is_some(), top, bottom].iter().filter(|b| **b).count() != 1 {
                bail!("exactly one of --before, --after, --top, --bottom must be given");
            }
            let before = before.as_deref().map(TaskId::parse).transpose()?;
            let after = after.as_deref().map(TaskId::parse).transpose()?;
            let target = match (&before, &after, top) {
                (Some(b), _, _) => MoveTarget::Before(b),
                (_, Some(a), _) => MoveTarget::After(a),
                (_, _, true) => MoveTarget::Top,
                _ => MoveTarget::Bottom,
            };
            let view = tasks::move_task(&conn, &parsed, target)?;
            materialize_for(&conn, ws, &view)?;
            println!("moved {} (order={})", view.external_id(), view.task.order);
            Ok(())
        }
        TasksCmd::Edit { id, title, body, priority, cycle, parent, status } => {
            let parsed = TaskId::parse(&id)?;
            let cycle_id = resolve_cycle(&conn, cycle.as_deref())?;
            let parent_id = resolve_parent(&conn, parent.as_deref())?;
            let core = title.is_some() || body.is_some() || priority.is_some() || cycle_id.is_some() || parent_id.is_some();
            if !core && status.is_none() {
                bail!("no fields to update");
            }
            let mut view = if core {
                tasks::edit(&mut conn, &parsed, tasks::EditArgs { title: title.as_deref(), body: body.as_deref(), priority, cycle_id, parent_id })?
            } else {
                task_by_id(&conn, &id)?
            };
            if let Some(s) = status.as_deref() {
                view = set_status(&conn, &parsed, s)?;
            }
            materialize_for(&conn, ws, &view)?;
            println!("updated task {}", view.external_id());
            Ok(())
        }
        TasksCmd::View { id, json: as_json } => {
            let view = task_by_id(&conn, &id)?;
            let task_labels = labels::for_task(&conn, view.task.id)?;
            let children = tasks::children(&conn, view.task.id)?;
            let blocked_by = deps::edges(&conn, view.task.id, true)?;
            let blocks = deps::edges(&conn, view.task.id, false)?;
            if as_json {
                #[derive(Serialize)]
                struct Out<'a> {
                    #[serde(flatten)]
                    task: &'a TaskView,
                    labels: &'a [labels::Label],
                    children: &'a [tasks::ChildTask],
                    blocked_by: &'a [tasks::DepLine],
                    blocks: &'a [tasks::DepLine],
                }
                return json(&Out { task: &view, labels: &task_labels, children: &children, blocked_by: &blocked_by, blocks: &blocks });
            }
            let slugs: Vec<&str> = task_labels.iter().map(|l| l.slug.as_str()).collect();
            print!("{}", tasks::render_show(&view, &slugs, &children, &blocked_by, &blocks));
            Ok(())
        }
        TasksCmd::Delete { id, hard } => {
            let parsed = TaskId::parse(&id)?;
            let view = if hard {
                let view = tasks::hard_delete(&mut conn, &parsed)?;
                println!("hard-deleted {}", view.external_id());
                view
            } else {
                let view = tasks::soft_delete(&mut conn, &parsed)?;
                println!("deleted {}", view.external_id());
                view
            };
            materialize_for(&conn, ws, &view)?;
            Ok(())
        }
        TasksCmd::Restore { id } => {
            let view = tasks::restore(&mut conn, &TaskId::parse(&id)?)?;
            materialize_for(&conn, ws, &view)?;
            println!("restored {}", view.external_id());
            Ok(())
        }
        TasksCmd::Purge { project, status, deleted, dry_run } => {
            let matches = tasks::purge(
                &mut conn,
                tasks::PurgeFilters { project_slug: project.as_deref(), status: status.as_deref(), deleted_only: deleted, dry_run },
            )?;
            if matches.is_empty() {
                println!("(no tasks matched)");
                return Ok(());
            }
            println!("{} {} tasks:", if dry_run { "would purge" } else { "purged" }, matches.len());
            for m in &matches {
                println!("  {}  {}", m.external_id(), m.task.title);
            }
            if !dry_run {
                // Re-materialize each distinct owner board once.
                let mut seen = std::collections::HashSet::new();
                for m in &matches {
                    if seen.insert((m.task.project_id, m.task.space_id)) {
                        materialize_for(&conn, ws, m)?;
                    }
                }
            }
            Ok(())
        }
        TasksCmd::Done { id } => transition(&conn, ws, &id, "done"),
        TasksCmd::Park { id } => transition(&conn, ws, &id, "parked"),
        TasksCmd::Reopen { id } => transition(&conn, ws, &id, "backlog"),
        TasksCmd::Label { action } => match action {
            TaskLabelCmd::Add { id, labels: slugs } => {
                let view = task_by_id(&conn, &id)?;
                for slug in &slugs {
                    labels::attach(&conn, view.task.id, labels::upsert(&conn, slug)?.id)?;
                }
                materialize_for(&conn, ws, &view)?;
                println!("attached {} label(s) to {}", slugs.len(), view.external_id());
                Ok(())
            }
            TaskLabelCmd::Rm { id, labels: slugs } => {
                let view = task_by_id(&conn, &id)?;
                let mut removed = 0;
                for slug in &slugs {
                    if let Some(l) = labels::get(&conn, slug)? {
                        removed += usize::from(labels::detach(&conn, view.task.id, l.id)?);
                    }
                }
                materialize_for(&conn, ws, &view)?;
                println!("detached {removed} label(s) from {}", view.external_id());
                Ok(())
            }
        },
        TasksCmd::Deps { action } => match action {
            TaskDepsCmd::Add { id, blockers, kind } => {
                let view = task_by_id(&conn, &id)?;
                for raw in &blockers {
                    deps::add(&conn, view.task.id, task_by_id(&conn, raw)?.task.id, &kind)?;
                }
                materialize_for(&conn, ws, &view)?;
                println!("{} now depends on {} ({kind})", view.external_id(), blockers.len());
                Ok(())
            }
            TaskDepsCmd::Rm { id, blockers } => {
                let view = task_by_id(&conn, &id)?;
                let mut removed = 0;
                for raw in &blockers {
                    removed += usize::from(deps::remove(&conn, view.task.id, task_by_id(&conn, raw)?.task.id)?);
                }
                materialize_for(&conn, ws, &view)?;
                println!("removed {removed} dependency(ies) from {}", view.external_id());
                Ok(())
            }
            TaskDepsCmd::Show { id } => {
                let view = task_by_id(&conn, &id)?;
                let up = deps::edges(&conn, view.task.id, true)?;
                let down = deps::edges(&conn, view.task.id, false)?;
                println!("{}  {}  {}", view.external_id(), if view.blocked { "blocked" } else { "unblocked" }, view.task.title);
                for (heading, lines) in [("blocked by", &up), ("blocks", &down)] {
                    println!("\n{heading}:");
                    if lines.is_empty() {
                        println!("  (none)");
                    }
                    for d in lines {
                        let note = match (d.kind.as_str(), d.resolved) {
                            ("related", _) => "  (related)",
                            (_, true) => "  (resolved)",
                            _ => "",
                        };
                        println!("- {}  {}  p{}  {}{note}", d.id, tasks::status_glyph(&d.status), d.priority, d.title);
                    }
                }
                Ok(())
            }
        },
    }
}

/// Re-render the board of the task's owner (project or space) after a mutation, as v1 did.
fn materialize_for(conn: &Connection, ws: &Workspace, view: &TaskView) -> Result<()> {
    if view.task.space_id.is_some() {
        crate::render::materialize_space(conn, &ws.root, &view.project_slug)?;
    } else {
        crate::render::materialize_project(conn, &ws.root, &view.project_slug, None)?;
    }
    Ok(())
}

/// Status change that reports the dependents it released (iss-0071).
fn set_status(conn: &Connection, id: &TaskId, status: &str) -> Result<TaskView> {
    let before = tasks::require(conn, id)?;
    let waiting = deps::blocked_dependents(conn, before.task.id)?;
    let view = tasks::set_status(conn, id, status)?;
    let mut released = Vec::new();
    for rid in waiting {
        if deps::is_blocked(conn, rid)? {
            continue;
        }
        if let Some(t) = tasks::get_by_row_id(conn, rid)? {
            released.push(t.external_id());
        }
    }
    if !released.is_empty() {
        println!("released: {}", released.join(", "));
    }
    Ok(view)
}

fn transition(conn: &Connection, ws: &Workspace, id: &str, status: &str) -> Result<()> {
    let view = set_status(conn, &TaskId::parse(id)?, status)?;
    materialize_for(conn, ws, &view)?;
    println!("{} -> {}", view.external_id(), view.task.status);
    Ok(())
}

// --- cycles -------------------------------------------------------------------

#[derive(Args, Debug)]
pub struct CyclesArgs {
    #[command(subcommand)]
    action: CyclesCmd,
}

#[derive(Subcommand, Debug)]
enum CyclesCmd {
    /// List cycles (ordered by start_date desc).
    #[command(alias = "ls")]
    List {
        #[arg(long)]
        json: bool,
    },
    /// Add a new cycle.
    Add {
        /// Cycle key (e.g. C-15).
        key: String,
        /// Start date (YYYY-MM-DD).
        #[arg(short = 's', long)]
        start: String,
        /// End date (YYYY-MM-DD).
        #[arg(short = 'e', long)]
        end: String,
        /// Optional description.
        #[arg(short = 'd', long)]
        description: Option<String>,
        #[arg(long, value_parser = cycles::STATUSES.to_vec())]
        status: Option<String>,
        /// Optional path to the cycle's plan/review artifacts.
        #[arg(short = 'p', long)]
        path: Option<String>,
    },
    /// Edit an existing cycle.
    Edit {
        key: String,
        #[arg(short = 's', long)]
        start: Option<String>,
        #[arg(short = 'e', long)]
        end: Option<String>,
        #[arg(short = 'd', long)]
        description: Option<String>,
        #[arg(long, value_parser = cycles::STATUSES.to_vec())]
        status: Option<String>,
        #[arg(short = 'p', long)]
        path: Option<String>,
    },
    /// Show a cycle.
    Show {
        key: String,
        #[arg(long)]
        json: bool,
    },
}

pub fn cycles(ws: &Workspace, args: CyclesArgs) -> Result<()> {
    let conn = db::open(&ws.root)?;
    match args.action {
        CyclesCmd::List { json: as_json } => {
            let rows = cycles::list(&conn)?;
            emit(as_json, &rows, || cycles::render_list(&rows))
        }
        CyclesCmd::Add { key, start, end, description, status, path } => {
            let c = cycles::add(
                &conn,
                cycles::AddArgs {
                    key: &key,
                    start_date: &start,
                    end_date: &end,
                    description: description.as_deref(),
                    status: status.as_deref(),
                    path: path.as_deref(),
                },
            )?;
            println!("added cycle {} ({} → {})", c.key, c.start_date, c.end_date);
            Ok(())
        }
        CyclesCmd::Edit { key, start, end, description, status, path } => {
            let c = cycles::edit(
                &conn,
                &key,
                cycles::EditArgs {
                    start_date: start.as_deref(),
                    end_date: end.as_deref(),
                    description: description.as_deref(),
                    status: status.as_deref(),
                    path: path.as_deref(),
                },
            )?;
            println!("updated cycle {}", c.key);
            Ok(())
        }
        CyclesCmd::Show { key, json: as_json } => {
            let c = cycles::require(&conn, &key)?;
            emit(as_json, &c, || cycles::render_show(&c))
        }
    }
}

// --- labels -------------------------------------------------------------------

#[derive(Args, Debug)]
pub struct LabelsArgs {
    #[command(subcommand)]
    action: LabelsCmd,
}

#[derive(Subcommand, Debug)]
enum LabelsCmd {
    /// List labels.
    #[command(alias = "ls")]
    List {
        #[arg(long)]
        json: bool,
    },
    /// Add a new label.
    Add {
        /// Label slug (unique).
        slug: String,
        /// Display name (defaults to slug).
        #[arg(short = 'n', long)]
        name: Option<String>,
        /// Optional hex color (e.g. #ff0000).
        #[arg(short = 'c', long)]
        color: Option<String>,
    },
    /// Edit an existing label.
    Edit {
        slug: String,
        #[arg(short = 'n', long)]
        name: Option<String>,
        #[arg(short = 'c', long)]
        color: Option<String>,
    },
    /// Show a label.
    Show {
        slug: String,
        #[arg(long)]
        json: bool,
    },
}

pub fn labels(ws: &Workspace, args: LabelsArgs) -> Result<()> {
    let conn = db::open(&ws.root)?;
    match args.action {
        LabelsCmd::List { json: as_json } => {
            let rows = labels::list(&conn)?;
            emit(as_json, &rows, || labels::render_list(&rows))
        }
        LabelsCmd::Add { slug, name, color } => {
            println!("added label {}", labels::add(&conn, &slug, name.as_deref(), color.as_deref())?.slug);
            Ok(())
        }
        LabelsCmd::Edit { slug, name, color } => {
            println!("updated label {}", labels::edit(&conn, &slug, name.as_deref(), color.as_deref())?.slug);
            Ok(())
        }
        LabelsCmd::Show { slug, json: as_json } => {
            let l = labels::require(&conn, &slug)?;
            emit(as_json, &l, || labels::render_show(&l))
        }
    }
}

// --- statuses -----------------------------------------------------------------

/// Shared `--tasks | --projects` scope selector.
#[derive(Args, Debug)]
#[group(required = true, multiple = false)]
struct StatusKindArg {
    /// Operate on task statuses.
    #[arg(long)]
    tasks: bool,
    /// Operate on project statuses.
    #[arg(long)]
    projects: bool,
}

impl StatusKindArg {
    fn kind(&self) -> Kind {
        if self.tasks { Kind::Task } else { Kind::Project }
    }
}

/// Lenient boolean: `true/t/yes/y/1` and `false/f/no/n/0`, case-insensitive.
fn parse_bool_flag(s: &str) -> Result<bool, String> {
    match s.to_ascii_lowercase().as_str() {
        "true" | "t" | "yes" | "y" | "1" => Ok(true),
        "false" | "f" | "no" | "n" | "0" => Ok(false),
        other => Err(format!("expected true/false/yes/no/1/0, got '{other}'")),
    }
}

#[derive(Args, Debug)]
pub struct StatusesArgs {
    #[command(subcommand)]
    action: StatusesCmd,
}

#[derive(Subcommand, Debug)]
enum StatusesCmd {
    /// List statuses for the chosen kind.
    #[command(alias = "ls")]
    List {
        #[command(flatten)]
        kind: StatusKindArg,
        #[arg(long)]
        json: bool,
    },
    /// Add a new status.
    Add {
        /// New status slug.
        slug: String,
        #[command(flatten)]
        kind: StatusKindArg,
        /// Display name (defaults to slug).
        #[arg(short = 'n', long)]
        name: Option<String>,
        /// Free-form description shown in `statuses show`.
        #[arg(short = 'd', long)]
        description: Option<String>,
        /// Optional hex color (e.g. #ff0000).
        #[arg(short = 'c', long)]
        color: Option<String>,
        /// Mark as closed (stamps closed_at; only valid with --tasks).
        #[arg(long)]
        closed: bool,
        /// Mark as archived (hidden from default project list; only valid with --projects).
        #[arg(long)]
        archived: bool,
        /// Sort order (lower renders first). Defaults to MAX+1.
        #[arg(long)]
        order: Option<i64>,
        /// Hidden status — section renders as a count + summary command line, no table.
        #[arg(long, value_name = "BOOL", value_parser = parse_bool_flag)]
        hidden: Option<bool>,
        /// Icon name rendered in task tables (only valid with --tasks).
        #[arg(long)]
        icon: Option<String>,
    },
    /// Edit an existing status.
    Edit {
        slug: String,
        #[command(flatten)]
        kind: StatusKindArg,
        #[arg(short = 'n', long)]
        name: Option<String>,
        #[arg(short = 'd', long)]
        description: Option<String>,
        #[arg(short = 'c', long)]
        color: Option<String>,
        /// Toggle the closed flag (only valid with --tasks).
        #[arg(long, conflicts_with = "no_closed")]
        closed: bool,
        #[arg(long, conflicts_with = "closed")]
        no_closed: bool,
        /// Toggle the archived flag (only valid with --projects).
        #[arg(long, conflicts_with = "no_archived")]
        archived: bool,
        #[arg(long, conflicts_with = "archived")]
        no_archived: bool,
        /// Sort order (lower renders first).
        #[arg(long)]
        order: Option<i64>,
        /// Hidden status — section renders as a count + summary command line, no table.
        #[arg(long, value_name = "BOOL", value_parser = parse_bool_flag)]
        hidden: Option<bool>,
        /// Icon name (only valid with --tasks; pass "" to clear).
        #[arg(long)]
        icon: Option<String>,
    },
    /// Remove a status (fails if in use).
    Rm {
        slug: String,
        #[command(flatten)]
        kind: StatusKindArg,
    },
    /// Show a single status.
    Show {
        slug: String,
        #[command(flatten)]
        kind: StatusKindArg,
        #[arg(long)]
        json: bool,
    },
}

/// The kind-specific flag for `add`: `--closed` for tasks, `--archived` for projects.
fn add_flag(kind: Kind, closed: bool, archived: bool) -> Result<bool> {
    match kind {
        Kind::Task if archived => bail!("--archived applies only to project statuses (use --closed with --tasks)"),
        Kind::Task => Ok(closed),
        Kind::Project if closed => bail!("--closed applies only to task statuses (use --archived with --projects)"),
        Kind::Project => Ok(archived),
    }
}

fn edit_flag(kind: Kind, closed: bool, no_closed: bool, archived: bool, no_archived: bool) -> Result<Option<bool>> {
    let pick = |on: bool, off: bool| if on { Some(true) } else if off { Some(false) } else { None };
    match kind {
        Kind::Task if archived || no_archived => bail!("--archived/--no-archived apply only to project statuses"),
        Kind::Task => Ok(pick(closed, no_closed)),
        Kind::Project if closed || no_closed => bail!("--closed/--no-closed apply only to task statuses"),
        Kind::Project => Ok(pick(archived, no_archived)),
    }
}

pub fn statuses(ws: &Workspace, args: StatusesArgs) -> Result<()> {
    let conn = db::open(&ws.root)?;
    match args.action {
        StatusesCmd::List { kind, json: as_json } => {
            let rows = statuses::list(&conn, kind.kind())?;
            emit(as_json, &rows, || statuses::render_list(&rows, kind.kind()))
        }
        StatusesCmd::Add { slug, kind, name, description, color, closed, archived, order, hidden, icon } => {
            let flag = add_flag(kind.kind(), closed, archived)?;
            let s = statuses::add(
                &conn,
                kind.kind(),
                statuses::AddArgs {
                    slug: &slug,
                    name: name.as_deref(),
                    description: description.as_deref(),
                    color: color.as_deref(),
                    flag,
                    sort_order: order,
                    is_hidden: hidden.unwrap_or(false),
                    icon: icon.as_deref(),
                },
            )?;
            println!("added status {}", s.slug);
            Ok(())
        }
        StatusesCmd::Edit { slug, kind, name, description, color, closed, no_closed, archived, no_archived, order, hidden, icon } => {
            let flag = edit_flag(kind.kind(), closed, no_closed, archived, no_archived)?;
            let s = statuses::edit(
                &conn,
                kind.kind(),
                &slug,
                statuses::EditArgs {
                    name: name.as_deref(),
                    description: description.as_deref(),
                    color: color.as_deref(),
                    flag,
                    sort_order: order,
                    is_hidden: hidden,
                    icon: icon.as_deref(),
                },
            )?;
            println!("updated status {}", s.slug);
            Ok(())
        }
        StatusesCmd::Rm { slug, kind } => {
            statuses::remove(&conn, kind.kind(), &slug)?;
            println!("removed status {slug}");
            Ok(())
        }
        StatusesCmd::Show { slug, kind, json: as_json } => {
            let s = statuses::require(&conn, kind.kind(), &slug)?;
            emit(as_json, &s, || statuses::render_show(&s, kind.kind()))
        }
    }
}
