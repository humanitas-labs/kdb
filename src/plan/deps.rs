//! Task dependencies (iss-0071). An edge `task_id → depends_on` of kind
//! `blocks` means the task cannot start until the blocker's status is closed;
//! `related` edges are informational. Blocked is derived, never stored, and
//! inherits down the parent chain.

use std::collections::{HashMap, VecDeque};

use anyhow::{Context, Result, bail};
use rusqlite::{Connection, params};

use super::tasks::{self, DepLine, TaskView};

pub const KINDS: &[&str] = &["blocks", "related"];

/// Add an edge. `blocks` edges are rejected when they would close a cycle.
pub fn add(conn: &Connection, task_id: i64, blocker_id: i64, kind: &str) -> Result<()> {
    if !KINDS.contains(&kind) {
        bail!("invalid dependency kind '{kind}' (expected {})", KINDS.join(", "));
    }
    if task_id == blocker_id {
        bail!("a task cannot depend on itself");
    }
    if kind == "blocks"
        && let Some(path) = blocks_path(conn, blocker_id, task_id)?
    {
        let mut ids = vec![external_id(conn, task_id)?];
        for rid in path {
            ids.push(external_id(conn, rid)?);
        }
        bail!("dependency cycle: {}", ids.join(" -> "));
    }
    conn.execute(
        "INSERT INTO task_deps (task_id, depends_on, kind) VALUES (?, ?, ?) \
         ON CONFLICT(task_id, depends_on) DO UPDATE SET kind = excluded.kind",
        params![task_id, blocker_id, kind],
    )
    .context("failed to add dependency")?;
    Ok(())
}

/// Remove an edge of any kind; `true` when one existed.
pub fn remove(conn: &Connection, task_id: i64, blocker_id: i64) -> Result<bool> {
    let n = conn
        .execute("DELETE FROM task_deps WHERE task_id = ? AND depends_on = ?", params![task_id, blocker_id])
        .context("failed to remove dependency")?;
    Ok(n > 0)
}

/// BFS along `blocks` edges from `from` toward `to`; the path (excluding
/// `from`'s predecessor, ending in `to`) when one exists.
fn blocks_path(conn: &Connection, from: i64, to: i64) -> Result<Option<Vec<i64>>> {
    let mut stmt = conn.prepare("SELECT depends_on FROM task_deps WHERE task_id = ? AND kind = 'blocks'")?;
    let mut prev: HashMap<i64, i64> = HashMap::new();
    let mut queue = VecDeque::from([from]);
    while let Some(cur) = queue.pop_front() {
        if cur == to {
            let mut path = vec![to];
            let mut at = to;
            while let Some(&p) = prev.get(&at) {
                path.push(p);
                at = p;
            }
            path.reverse();
            return Ok(Some(path));
        }
        let next: Vec<i64> = stmt.query_map([cur], |r| r.get(0))?.collect::<rusqlite::Result<_>>()?;
        for n in next {
            if n != from && !prev.contains_key(&n) {
                prev.insert(n, cur);
                queue.push_back(n);
            }
        }
    }
    Ok(None)
}

fn external_id(conn: &Connection, rid: i64) -> Result<String> {
    Ok(tasks::get_by_row_id(conn, rid)?.with_context(|| format!("task row {rid} not found"))?.external_id())
}

/// Unresolved blockers of a task, including those inherited from its ancestors.
pub fn blockers_of(conn: &Connection, task_id: i64) -> Result<Vec<TaskView>> {
    let mut stmt = conn.prepare(
        "WITH RECURSIVE up(id) AS (SELECT ?1 UNION ALL SELECT t.parent_id FROM tasks t JOIN up ON t.id = up.id WHERE t.parent_id IS NOT NULL) \
         SELECT DISTINCT d.depends_on FROM task_deps d JOIN up ON up.id = d.task_id \
           JOIN tasks b ON b.id = d.depends_on JOIN task_statuses s ON s.slug = b.status \
          WHERE d.kind = 'blocks' AND s.is_closed = 0 AND b.deleted_at IS NULL ORDER BY d.depends_on",
    )?;
    let ids: Vec<i64> = stmt.query_map([task_id], |r| r.get(0))?.collect::<rusqlite::Result<_>>()?;
    ids.into_iter().filter_map(|rid| tasks::get_by_row_id(conn, rid).transpose()).collect()
}

/// Whether the task has any unresolved `blocks` dependency, own or inherited.
pub fn is_blocked(conn: &Connection, task_id: i64) -> Result<bool> {
    let n: i64 = conn.query_row(
        "WITH RECURSIVE up(id) AS (SELECT ?1 UNION ALL SELECT t.parent_id FROM tasks t JOIN up ON t.id = up.id WHERE t.parent_id IS NOT NULL) \
         SELECT COUNT(*) FROM task_deps d JOIN up ON up.id = d.task_id \
           JOIN tasks b ON b.id = d.depends_on JOIN task_statuses s ON s.slug = b.status \
          WHERE d.kind = 'blocks' AND s.is_closed = 0 AND b.deleted_at IS NULL",
        [task_id],
        |r| r.get(0),
    )?;
    Ok(n > 0)
}

/// Edges where the task is the dependent (`upstream = true`: what it waits on)
/// or the dependency (`upstream = false`: what waits on it). All kinds.
pub fn edges(conn: &Connection, task_id: i64, upstream: bool) -> Result<Vec<DepLine>> {
    let (match_col, other_col) = if upstream { ("task_id", "depends_on") } else { ("depends_on", "task_id") };
    let mut stmt = conn.prepare(&format!(
        "SELECT d.{other_col}, d.kind, s.is_closed FROM task_deps d \
           JOIN tasks o ON o.id = d.{other_col} JOIN task_statuses s ON s.slug = o.status \
          WHERE d.{match_col} = ? AND o.deleted_at IS NULL ORDER BY d.kind, d.{other_col}"
    ))?;
    let rows: Vec<(i64, String, bool)> =
        stmt.query_map([task_id], |r| Ok((r.get(0)?, r.get(1)?, r.get::<_, i64>(2)? != 0)))?.collect::<rusqlite::Result<_>>()?;
    let mut out = Vec::new();
    for (rid, kind, closed) in rows {
        if let Some(v) = tasks::get_by_row_id(conn, rid)? {
            out.push(DepLine {
                id: v.external_id(),
                status: v.task.status,
                priority: v.task.priority,
                title: v.task.title,
                kind,
                resolved: closed,
            });
        }
    }
    Ok(out)
}

/// Direct `blocks` dependents of a task that are currently blocked; after the
/// task closes, those no longer blocked are the ones it released.
pub fn blocked_dependents(conn: &Connection, task_id: i64) -> Result<Vec<i64>> {
    let mut stmt = conn.prepare("SELECT task_id FROM task_deps WHERE depends_on = ? AND kind = 'blocks' ORDER BY task_id")?;
    let ids: Vec<i64> = stmt.query_map([task_id], |r| r.get(0))?.collect::<rusqlite::Result<_>>()?;
    let mut out = Vec::new();
    for id in ids {
        if is_blocked(conn, id)? {
            out.push(id);
        }
    }
    Ok(out)
}
