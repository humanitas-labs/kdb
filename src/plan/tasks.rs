//! Tasks: owned by exactly one project or space, numbered per owner (`seq`) or
//! per parent (`child_seq`), ordered by fractional keys, soft-deletable.
//!
//! Every read goes through one SELECT whose CTEs compute the dotted external
//! id (`0012.3`) and the derived blocked flag (iss-0071): a task is blocked
//! when it, or any ancestor, has a `blocks` dependency on an open task.

use std::collections::HashMap;

use anyhow::{Context, Result, bail};
use rusqlite::{Connection, OptionalExtension, Row, TransactionBehavior, params};
use serde::Serialize;

use super::{TaskId, between, default_order_key, kv};

#[derive(Debug, Clone, Serialize)]
pub struct Task {
    pub id: i64,
    pub project_id: Option<i64>,
    pub space_id: Option<i64>,
    pub seq: Option<i64>,
    pub child_seq: Option<i64>,
    pub title: String,
    pub body: Option<String>,
    pub status: String,
    pub priority: i64,
    pub order: String,
    pub cycle_id: Option<i64>,
    pub parent_id: Option<i64>,
    pub created_at: String,
    pub updated_at: String,
    pub closed_at: Option<String>,
    pub deleted_at: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct TaskView {
    #[serde(flatten)]
    pub task: Task,
    /// Owner slug: the project's, or the space's for space-native tasks.
    pub project_slug: String,
    pub project_alias: String,
    pub cycle_key: Option<String>,
    pub dotted: String,
    /// Derived: an unresolved `blocks` dependency on this task or an ancestor.
    pub blocked: bool,
}

impl TaskView {
    pub fn external_id(&self) -> String {
        format!("{}-{}", self.project_alias, self.dotted)
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct ChildTask {
    pub id: String,
    pub status: String,
    pub priority: i64,
    pub title: String,
    pub order: String,
}

pub const CTE: &str = "WITH RECURSIVE crumbs(id, root_seq, suffix) AS (\
    SELECT id, seq, '' FROM tasks WHERE parent_id IS NULL \
    UNION ALL \
    SELECT t.id, c.root_seq, c.suffix || '.' || t.child_seq FROM tasks t JOIN crumbs c ON c.id = t.parent_id), \
  blocked(id) AS (\
    SELECT d.task_id FROM task_deps d \
      JOIN tasks b ON b.id = d.depends_on \
      JOIN task_statuses bs ON bs.slug = b.status \
     WHERE d.kind = 'blocks' AND bs.is_closed = 0 AND b.deleted_at IS NULL \
    UNION \
    SELECT t.id FROM tasks t JOIN blocked bl ON t.parent_id = bl.id) ";

const ORDER_EXPR: &str = "COALESCE(t.\"order\", printf('%012d', COALESCE(t.seq, t.child_seq)))";

const SELECT: &str = "SELECT t.id, t.project_id, t.space_id, t.seq, t.child_seq, t.title, t.body, t.status, t.priority, \
    COALESCE(t.\"order\", printf('%012d', COALESCE(t.seq, t.child_seq))), \
    t.cycle_id, t.parent_id, t.created_at, t.updated_at, t.closed_at, t.deleted_at, \
    COALESCE(p.slug, s.slug), COALESCE(p.alias, s.alias), c.key, printf('%04d', cr.root_seq) || cr.suffix, \
    t.id IN (SELECT id FROM blocked) \
    FROM tasks t LEFT JOIN projects p ON p.id = t.project_id LEFT JOIN spaces s ON s.id = t.space_id \
    LEFT JOIN cycles c ON c.id = t.cycle_id JOIN crumbs cr ON cr.id = t.id";

fn from_row(r: &Row) -> rusqlite::Result<TaskView> {
    Ok(TaskView {
        task: Task {
            id: r.get(0)?,
            project_id: r.get(1)?,
            space_id: r.get(2)?,
            seq: r.get(3)?,
            child_seq: r.get(4)?,
            title: r.get(5)?,
            body: r.get(6)?,
            status: r.get(7)?,
            priority: r.get(8)?,
            order: r.get(9)?,
            cycle_id: r.get(10)?,
            parent_id: r.get(11)?,
            created_at: r.get(12)?,
            updated_at: r.get(13)?,
            closed_at: r.get(14)?,
            deleted_at: r.get(15)?,
        },
        project_slug: r.get(16)?,
        project_alias: r.get(17)?,
        cycle_key: r.get(18)?,
        dotted: r.get(19)?,
        blocked: r.get(20)?,
    })
}

fn query(conn: &Connection, sql: &str, args: &[&dyn rusqlite::ToSql]) -> Result<Vec<TaskView>> {
    let mut stmt = conn.prepare(&format!("{CTE}{SELECT} {sql}"))?;
    let rows = stmt.query_map(args, from_row)?;
    rows.collect::<rusqlite::Result<_>>().context("failed to read tasks")
}

pub fn is_closed_status(conn: &Connection, slug: &str) -> Result<bool> {
    let flag: i64 = conn
        .query_row("SELECT is_closed FROM task_statuses WHERE slug = ?", [slug], |r| r.get(0))
        .with_context(|| format!("unknown task status '{slug}'"))?;
    Ok(flag != 0)
}

#[derive(Debug, Default)]
pub struct ListFilters<'a> {
    /// `None` means every status.
    pub statuses: Option<&'a [String]>,
    pub project_slug: Option<&'a str>,
    pub space_slug: Option<&'a str>,
    pub cycle_key: Option<&'a str>,
    pub priority: Option<i64>,
    pub limit: Option<i64>,
    pub top_level_only: bool,
    /// `Some(true)` only blocked tasks, `Some(false)` only unblocked.
    pub blocked: Option<bool>,
    /// Sort by priority before owner and order key (`tasks ready`).
    pub by_priority: bool,
}

pub fn list(conn: &Connection, f: ListFilters) -> Result<Vec<TaskView>> {
    let mut sql = String::from("WHERE t.deleted_at IS NULL");
    let mut args: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();
    if let Some(statuses) = f.statuses {
        if statuses.is_empty() {
            return Ok(Vec::new());
        }
        sql.push_str(&format!(" AND t.status IN ({})", vec!["?"; statuses.len()].join(",")));
        args.extend(statuses.iter().map(|s| Box::new(s.clone()) as Box<dyn rusqlite::ToSql>));
    }
    if let Some(slug) = f.project_slug {
        sql.push_str(" AND p.slug = ?");
        args.push(Box::new(slug.to_string()));
    }
    if let Some(space) = f.space_slug {
        sql.push_str(" AND (p.space_id = (SELECT id FROM spaces WHERE slug = ?) OR t.space_id = (SELECT id FROM spaces WHERE slug = ?))");
        args.push(Box::new(space.to_string()));
        args.push(Box::new(space.to_string()));
    }
    if let Some(key) = f.cycle_key {
        sql.push_str(" AND c.key = ?");
        args.push(Box::new(key.to_string()));
    }
    if let Some(pri) = f.priority {
        sql.push_str(" AND t.priority = ?");
        args.push(Box::new(pri));
    }
    if f.top_level_only {
        sql.push_str(" AND t.parent_id IS NULL");
    }
    match f.blocked {
        Some(true) => sql.push_str(" AND t.id IN (SELECT id FROM blocked)"),
        Some(false) => sql.push_str(" AND t.id NOT IN (SELECT id FROM blocked)"),
        None => {}
    }
    sql.push_str(" ORDER BY ");
    if f.by_priority {
        sql.push_str("t.priority ASC, ");
    }
    sql.push_str(&format!("COALESCE(p.slug, s.slug) ASC, {ORDER_EXPR} ASC, t.seq ASC"));
    if let Some(limit) = f.limit {
        sql.push_str(" LIMIT ?");
        args.push(Box::new(limit));
    }
    let refs: Vec<&dyn rusqlite::ToSql> = args.iter().map(|b| b.as_ref()).collect();
    query(conn, &sql, &refs)
}

/// Row id for an external id, walking the child path. Soft-deleted rows resolve too.
fn row_id(conn: &Connection, id: &TaskId) -> Result<Option<i64>> {
    let top = conn
        .query_row(
            "SELECT t.id FROM tasks t LEFT JOIN projects p ON p.id = t.project_id LEFT JOIN spaces s ON s.id = t.space_id \
             WHERE COALESCE(p.alias, s.alias) = ? AND t.seq = ?",
            params![id.alias, id.seq],
            |r| r.get::<_, i64>(0),
        )
        .optional()
        .context("failed to look up top-level task")?;
    let Some(mut rid) = top else { return Ok(None) };
    for n in &id.child_path {
        match conn
            .query_row("SELECT id FROM tasks WHERE parent_id = ? AND child_seq = ?", params![rid, n], |r| r.get(0))
            .optional()
            .context("failed to look up child task")?
        {
            Some(child) => rid = child,
            None => return Ok(None),
        }
    }
    Ok(Some(rid))
}

pub fn get(conn: &Connection, id: &TaskId) -> Result<Option<TaskView>> {
    match row_id(conn, id)? {
        Some(rid) => get_by_row_id(conn, rid),
        None => Ok(None),
    }
}

/// `get`, or the v1 "task not found" error with the normalized id.
pub fn require(conn: &Connection, id: &TaskId) -> Result<TaskView> {
    get(conn, id)?.with_context(|| format!("task not found: {}", id.render()))
}

pub fn get_by_row_id(conn: &Connection, rid: i64) -> Result<Option<TaskView>> {
    Ok(query(conn, "WHERE t.id = ?", &[&rid])?.into_iter().next())
}

pub fn children(conn: &Connection, parent: i64) -> Result<Vec<ChildTask>> {
    let rows = query(conn, &format!("WHERE t.parent_id = ? AND t.deleted_at IS NULL ORDER BY {ORDER_EXPR} ASC, t.child_seq ASC"), &[&parent])?;
    Ok(rows
        .into_iter()
        .map(|v| ChildTask {
            id: v.external_id(),
            status: v.task.status,
            priority: v.task.priority,
            title: v.task.title,
            order: v.task.order,
        })
        .collect())
}

pub struct AddArgs<'a> {
    pub project_id: Option<i64>,
    pub space_id: Option<i64>,
    pub title: &'a str,
    pub body: Option<&'a str>,
    pub priority: Option<i64>,
    pub cycle_id: Option<i64>,
    pub parent_id: Option<i64>,
    pub order: Option<&'a str>,
}

/// Insert a task, assigning `seq`/`child_seq` inside a `BEGIN IMMEDIATE`
/// transaction so parallel writers cannot pick the same number. A UNIQUE
/// conflict (another writer slipped in between) is retried once.
pub fn add(conn: &mut Connection, a: AddArgs) -> Result<TaskView> {
    if a.project_id.is_some() == a.space_id.is_some() {
        bail!("a task must have exactly one owner (a project or a space)");
    }
    let priority = a.priority.unwrap_or(3);
    if !(1..=5).contains(&priority) {
        bail!("priority must be between 1 and 5");
    }
    let mut attempt = 0;
    let rid = loop {
        match insert(conn, &a, priority) {
            Ok(rid) => break rid,
            Err(e) if attempt == 0 && is_unique_violation(&e) => attempt += 1,
            Err(e) => return Err(e),
        }
    };
    get_by_row_id(conn, rid)?.context("task missing after insert")
}

fn insert(conn: &mut Connection, a: &AddArgs, priority: i64) -> Result<i64> {
    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let (seq, child_seq) = match a.parent_id {
        None => (Some(next_seq(&tx, a.project_id, a.space_id)?), None),
        Some(pid) => (None, Some(next_child_seq(&tx, pid)?)),
    };
    let order = a.order.map(str::to_string).unwrap_or_else(|| default_order_key(seq.or(child_seq).unwrap_or(0)));
    tx.execute(
        "INSERT INTO tasks (project_id, space_id, seq, child_seq, title, body, status, priority, \"order\", cycle_id, parent_id) \
         VALUES (?, ?, ?, ?, ?, ?, 'backlog', ?, ?, ?, ?)",
        params![a.project_id, a.space_id, seq, child_seq, a.title, a.body, priority, order, a.cycle_id, a.parent_id],
    )
    .context("failed to insert task")?;
    let rid = tx.last_insert_rowid();
    tx.commit()?;
    Ok(rid)
}

fn is_unique_violation(e: &anyhow::Error) -> bool {
    matches!(
        e.downcast_ref::<rusqlite::Error>(),
        Some(rusqlite::Error::SqliteFailure(f, _)) if f.extended_code == rusqlite::ffi::SQLITE_CONSTRAINT_UNIQUE
    )
}

fn next_seq(conn: &Connection, project_id: Option<i64>, space_id: Option<i64>) -> Result<i64> {
    conn.query_row(
        "SELECT COALESCE(MAX(seq), 0) + 1 FROM tasks WHERE project_id IS ? AND space_id IS ? AND parent_id IS NULL",
        params![project_id, space_id],
        |r| r.get(0),
    )
    .context("failed to compute next seq")
}

fn next_child_seq(conn: &Connection, parent: i64) -> Result<i64> {
    conn.query_row("SELECT COALESCE(MAX(child_seq), 0) + 1 FROM tasks WHERE parent_id = ?", [parent], |r| r.get(0))
        .context("failed to compute next child_seq")
}

#[derive(Default)]
pub struct EditArgs<'a> {
    pub title: Option<&'a str>,
    pub body: Option<&'a str>,
    pub priority: Option<i64>,
    pub cycle_id: Option<Option<i64>>,
    pub parent_id: Option<Option<i64>>,
}

pub fn edit(conn: &mut Connection, id: &TaskId, a: EditArgs) -> Result<TaskView> {
    if a.title.is_none() && a.body.is_none() && a.priority.is_none() && a.cycle_id.is_none() && a.parent_id.is_none() {
        bail!("no fields to update");
    }
    let existing = require(conn, id)?;
    if a.priority.is_some_and(|p| !(1..=5).contains(&p)) {
        bail!("priority must be between 1 and 5");
    }
    let rid = existing.task.id;
    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    if let Some(new_parent) = a.parent_id.filter(|p| *p != existing.task.parent_id) {
        match new_parent {
            Some(pid) if pid == rid => bail!("cannot make a task its own parent"),
            Some(pid) => {
                let cs = next_child_seq(&tx, pid)?;
                tx.execute(
                    "UPDATE tasks SET parent_id = ?, seq = NULL, child_seq = ?, \"order\" = ?, \
                        updated_at = strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE id = ?",
                    params![pid, cs, default_order_key(cs), rid],
                )
                .context("failed to set parent on task")?;
            }
            None => {
                let s = next_seq(&tx, existing.task.project_id, existing.task.space_id)?;
                tx.execute(
                    "UPDATE tasks SET parent_id = NULL, seq = ?, child_seq = NULL, \"order\" = ?, \
                        updated_at = strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE id = ?",
                    params![s, default_order_key(s), rid],
                )
                .context("failed to clear parent on task")?;
            }
        }
    }
    tx.execute(
        "UPDATE tasks SET title = COALESCE(?, title), body = COALESCE(?, body), priority = COALESCE(?, priority), \
            cycle_id = CASE WHEN ? THEN ? ELSE cycle_id END, \
            updated_at = strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE id = ?",
        params![a.title, a.body, a.priority, a.cycle_id.is_some(), a.cycle_id.flatten(), rid],
    )
    .context("failed to update task")?;
    tx.commit()?;
    get_by_row_id(conn, rid)?.context("task missing after update")
}

#[derive(Debug, Clone)]
pub enum MoveTarget<'a> {
    Before(&'a TaskId),
    After(&'a TaskId),
    Top,
    Bottom,
}

/// Order key for a new sibling immediately before (`after = false`) or after `anchor`.
pub fn order_key_adjacent(conn: &Connection, anchor: &TaskView, after: bool) -> Result<String> {
    let neighbor = neighbor_order(conn, anchor, after, -1)?;
    let pivot = anchor.task.order.as_str();
    Ok(if after { between(Some(pivot), neighbor.as_deref()) } else { between(neighbor.as_deref(), Some(pivot)) })
}

pub fn move_task(conn: &Connection, id: &TaskId, target: MoveTarget) -> Result<TaskView> {
    let task = require(conn, id)?;
    let new_order = match &target {
        MoveTarget::Top => between(None, end_order(conn, &task, true)?.as_deref()),
        MoveTarget::Bottom => between(end_order(conn, &task, false)?.as_deref(), None),
        MoveTarget::Before(sib) | MoveTarget::After(sib) => {
            let after = matches!(target, MoveTarget::After(_));
            let sib = get(conn, sib)?.with_context(|| format!("sibling not found: {}", sib.render()))?;
            if task.task.project_id != sib.task.project_id || task.task.space_id != sib.task.space_id {
                bail!("cannot move {}: target {} has a different owner", task.external_id(), sib.external_id());
            }
            if task.task.parent_id != sib.task.parent_id {
                bail!("cannot move {}: target {} has a different parent", task.external_id(), sib.external_id());
            }
            if sib.task.id == task.task.id {
                return Ok(task);
            }
            let neighbor = neighbor_order(conn, &sib, after, task.task.id)?;
            let pivot = sib.task.order.as_str();
            if after { between(Some(pivot), neighbor.as_deref()) } else { between(neighbor.as_deref(), Some(pivot)) }
        }
    };
    conn.execute(
        "UPDATE tasks SET \"order\" = ?, updated_at = strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE id = ?",
        params![new_order, task.task.id],
    )
    .context("failed to update task order")?;
    require(conn, id)
}

/// Order key of the sibling just after (`next = true`) or before `pivot`, excluding `exclude`.
fn neighbor_order(conn: &Connection, pivot: &TaskView, next: bool, exclude: i64) -> Result<Option<String>> {
    sibling_order(conn, pivot, exclude, next, Some(&pivot.task.order))
}

/// Order key of the first (`first = true`) or last sibling of `task`, excluding the task itself.
fn end_order(conn: &Connection, task: &TaskView, first: bool) -> Result<Option<String>> {
    sibling_order(conn, task, task.task.id, first, None)
}

/// Lowest (`asc`) or highest order key among `t`'s siblings, past `pivot` when given.
fn sibling_order(conn: &Connection, t: &TaskView, exclude: i64, asc: bool, pivot: Option<&str>) -> Result<Option<String>> {
    let (cmp, dir) = if asc { (">", "ASC") } else { ("<", "DESC") };
    let sql = format!(
        "SELECT \"order\" FROM tasks WHERE project_id IS ?1 AND space_id IS ?2 AND parent_id IS ?5 AND id != ?3 \
         AND (?4 IS NULL OR \"order\" {cmp} ?4) ORDER BY \"order\" {dir} LIMIT 1"
    );
    conn.prepare(&sql)?
        .query_row(params![t.task.project_id, t.task.space_id, exclude, pivot, t.task.parent_id], |r| r.get(0))
        .optional()
        .context("failed to read neighbor order")
}

pub fn set_status(conn: &Connection, id: &TaskId, status: &str) -> Result<TaskView> {
    let closed = is_closed_status(conn, status)?;
    let existing = require(conn, id)?;
    conn.execute(
        "UPDATE tasks SET status = ?, \
            closed_at = CASE WHEN ? THEN strftime('%Y-%m-%dT%H:%M:%fZ','now') ELSE NULL END, \
            updated_at = strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE id = ?",
        params![status, closed, existing.task.id],
    )?;
    require(conn, id)
}

fn subtree_ids(conn: &Connection, root: i64) -> Result<Vec<i64>> {
    let mut stmt = conn.prepare(
        "WITH RECURSIVE sub(id) AS (SELECT ?1 UNION ALL SELECT t.id FROM tasks t JOIN sub ON t.parent_id = sub.id) SELECT id FROM sub",
    )?;
    let rows = stmt.query_map([root], |r| r.get(0))?;
    rows.collect::<rusqlite::Result<_>>().context("failed to collect subtree ids")
}

/// Apply `set_sql` (an `UPDATE tasks SET ...` or `DELETE FROM tasks`) to the subtrees under `roots`.
fn apply_subtrees(conn: &mut Connection, roots: &[i64], sql: &str) -> Result<()> {
    let mut ids = Vec::new();
    for r in roots {
        ids.extend(subtree_ids(conn, *r)?);
    }
    ids.sort();
    ids.dedup();
    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    tx.pragma_update(None, "defer_foreign_keys", "ON")?;
    tx.execute(&format!("{sql} WHERE id IN ({})", vec!["?"; ids.len()].join(",")), rusqlite::params_from_iter(ids.iter()))?;
    tx.commit()?;
    Ok(())
}

pub fn soft_delete(conn: &mut Connection, id: &TaskId) -> Result<TaskView> {
    let existing = require(conn, id)?;
    apply_subtrees(
        conn,
        &[existing.task.id],
        "UPDATE tasks SET deleted_at = COALESCE(deleted_at, strftime('%Y-%m-%dT%H:%M:%fZ','now')), updated_at = strftime('%Y-%m-%dT%H:%M:%fZ','now')",
    )?;
    get_by_row_id(conn, existing.task.id)?.context("task missing after soft delete")
}

pub fn restore(conn: &mut Connection, id: &TaskId) -> Result<TaskView> {
    let existing = require(conn, id)?;
    apply_subtrees(conn, &[existing.task.id], "UPDATE tasks SET deleted_at = NULL, updated_at = strftime('%Y-%m-%dT%H:%M:%fZ','now')")?;
    get_by_row_id(conn, existing.task.id)?.context("task missing after restore")
}

/// Permanently remove a task and its subtree; returns the row as it was.
pub fn hard_delete(conn: &mut Connection, id: &TaskId) -> Result<TaskView> {
    let existing = require(conn, id)?;
    apply_subtrees(conn, &[existing.task.id], "DELETE FROM tasks")?;
    Ok(existing)
}

pub struct PurgeFilters<'a> {
    pub project_slug: Option<&'a str>,
    pub status: Option<&'a str>,
    pub deleted_only: bool,
    pub dry_run: bool,
}

/// Tasks matching the selectors; deleted with their subtrees unless `dry_run`.
pub fn purge(conn: &mut Connection, f: PurgeFilters) -> Result<Vec<TaskView>> {
    if f.status.is_none() && !f.deleted_only {
        bail!("purge requires at least one selector (--status or --deleted)");
    }
    let mut sql = String::from("WHERE 1=1");
    let mut args: Vec<&dyn rusqlite::ToSql> = Vec::new();
    if let Some(slug) = &f.project_slug {
        sql.push_str(" AND p.slug = ?");
        args.push(slug);
    }
    if let Some(status) = &f.status {
        sql.push_str(" AND t.status = ?");
        args.push(status);
    }
    if f.deleted_only {
        sql.push_str(" AND t.deleted_at IS NOT NULL");
    }
    let matches = query(conn, &sql, &args)?;
    if !f.dry_run && !matches.is_empty() {
        let roots: Vec<i64> = matches.iter().map(|m| m.task.id).collect();
        apply_subtrees(conn, &roots, "DELETE FROM tasks")?;
    }
    Ok(matches)
}

pub fn status_glyph(status: &str) -> &'static str {
    match status {
        "backlog" => "[ ]",
        "cycle" => "[>]",
        "in_progress" => "[~]",
        "done" => "[x]",
        "parked" => "[=]",
        _ => "[?]",
    }
}

/// The v1 list table. `blockers` maps a blocked task's row id to the external
/// ids of its unresolved blockers, appended to the title (iss-0071).
pub fn render_list(tasks: &[TaskView], blockers: &HashMap<i64, Vec<String>>) -> String {
    if tasks.is_empty() {
        return "(no tasks)\n".into();
    }
    let ids: Vec<String> = tasks.iter().map(TaskView::external_id).collect();
    let id_w = ids.iter().map(String::len).max().unwrap_or(4).max(4);
    let multi = tasks.iter().any(|t| t.project_slug != tasks[0].project_slug);
    let proj_w = tasks.iter().map(|t| t.project_slug.len()).max().unwrap_or(7).max(7);
    let mut out = String::new();
    if multi {
        out.push_str(&format!("{:<id_w$}  {:<proj_w$}  st    p  title\n", "id", "project"));
    } else {
        out.push_str(&format!("{:<id_w$}  st    p  title\n", "id"));
    }
    for (t, id) in tasks.iter().zip(&ids) {
        out.push_str(&format!("{id:<id_w$}  "));
        if multi {
            out.push_str(&format!("{:<proj_w$}  ", t.project_slug));
        }
        out.push_str(&format!("{}  {}  {}", status_glyph(&t.task.status), t.task.priority, t.task.title));
        if let Some(b) = blockers.get(&t.task.id) {
            out.push_str(&format!("  (blocked by {})", b.join(", ")));
        }
        out.push('\n');
    }
    out
}

/// One side of a dependency edge, for `view` and `deps show`.
#[derive(Debug, Clone, Serialize)]
pub struct DepLine {
    pub id: String,
    pub status: String,
    pub priority: i64,
    pub title: String,
    pub kind: String,
    /// For blockers: whether the blocker is closed.
    pub resolved: bool,
}

fn dep_lines(out: &mut String, heading: &str, lines: &[DepLine]) {
    if lines.is_empty() {
        return;
    }
    out.push_str(&format!("\n{heading}:\n"));
    for d in lines {
        let note = if d.kind == "related" { "  (related)" } else { "" };
        out.push_str(&format!("- {}  {}  p{}  {}{note}\n", d.id, status_glyph(&d.status), d.priority, d.title));
    }
}

pub fn render_show(t: &TaskView, labels: &[&str], children: &[ChildTask], blocked_by: &[DepLine], blocks: &[DepLine]) -> String {
    let mut out = String::new();
    kv(&mut out, 12, "id", t.external_id());
    kv(&mut out, 12, "title", &t.task.title);
    kv(&mut out, 12, "status", &t.task.status);
    if t.blocked {
        kv(&mut out, 12, "blocked", "yes");
    }
    kv(&mut out, 12, "priority", t.task.priority);
    kv(&mut out, 12, "order", &t.task.order);
    kv(&mut out, 12, "project", &t.project_slug);
    if let Some(c) = &t.cycle_key {
        kv(&mut out, 12, "cycle", c);
    }
    if !labels.is_empty() {
        kv(&mut out, 12, "labels", labels.join(", "));
    }
    kv(&mut out, 12, "created_at", &t.task.created_at);
    kv(&mut out, 12, "updated_at", &t.task.updated_at);
    if let Some(c) = &t.task.closed_at {
        kv(&mut out, 12, "closed_at", c);
    }
    if let Some(body) = &t.task.body {
        out.push('\n');
        out.push_str(body);
        if !body.ends_with('\n') {
            out.push('\n');
        }
    }
    if !children.is_empty() {
        out.push_str("\nchildren:\n");
        for c in children {
            out.push_str(&format!("- {}  {}  p{}  ord={}  {}\n", c.id, status_glyph(&c.status), c.priority, c.order, c.title));
        }
    }
    dep_lines(&mut out, "blocked by", blocked_by);
    dep_lines(&mut out, "blocks", blocks);
    out
}
