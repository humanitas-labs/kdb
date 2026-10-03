//! Read-only SQL over the planning schema, enough to materialize boards.
//!
//! The render layer deliberately does not depend on `plan/`: it reads the
//! tables directly so the two can be built and tested independently.

use std::collections::HashMap;

use anyhow::{Context, Result};
use rusqlite::{Connection, OptionalExtension, params};

pub struct Project {
    pub slug: String,
    pub path: String,
}

pub struct Space {
    pub name: String,
    pub alias: Option<String>,
    pub path: Option<String>,
}

pub struct Status {
    pub slug: String,
    pub name: String,
    pub description: Option<String>,
    pub is_closed: bool,
    pub is_hidden: bool,
    pub icon: Option<String>,
}

/// A task row joined with everything a table cell needs.
pub struct Task {
    pub id: i64,
    pub space_id: Option<i64>,
    /// Top-level seq; `None` for subtasks.
    pub seq: Option<i64>,
    pub title: String,
    pub body: Option<String>,
    pub status: String,
    pub priority: i64,
    pub order: String,
    /// Owner slug: the project slug, or the space slug for space-native tasks.
    pub owner_slug: String,
    /// `ALIAS-0030` or `ALIAS-0030.1.2`.
    pub external_id: String,
}

/// Which top-level tasks a board lists.
pub enum Scope<'a> {
    /// Tasks owned by one project.
    Project(&'a str),
    /// Member-project tasks plus the space's own tasks.
    Space(&'a str),
    /// Only the space's own tasks.
    SpaceNative(&'a str),
}

const CRUMBS: &str = "WITH RECURSIVE crumbs(id, root_seq, suffix) AS (\
    SELECT id, seq, '' FROM tasks WHERE parent_id IS NULL \
    UNION ALL \
    SELECT t.id, c.root_seq, c.suffix || '.' || t.child_seq \
      FROM tasks t JOIN crumbs c ON c.id = t.parent_id)";

const TASK_COLS: &str = "t.id, t.space_id, t.seq, t.title, t.body, t.status, t.priority, \
    COALESCE(t.\"order\", printf('%012d', COALESCE(t.seq, t.child_seq))), \
    COALESCE(p.slug, s.slug), \
    COALESCE(p.alias, s.alias) || '-' || printf('%04d', cr.root_seq) || cr.suffix";

const TASK_JOINS: &str = "FROM tasks t \
    LEFT JOIN projects p ON p.id = t.project_id \
    LEFT JOIN spaces s ON s.id = t.space_id \
    JOIN crumbs cr ON cr.id = t.id";

fn task_from_row(row: &rusqlite::Row) -> rusqlite::Result<Task> {
    Ok(Task {
        id: row.get(0)?,
        space_id: row.get(1)?,
        seq: row.get(2)?,
        title: row.get(3)?,
        body: row.get(4)?,
        status: row.get(5)?,
        priority: row.get(6)?,
        order: row.get(7)?,
        owner_slug: row.get(8)?,
        external_id: row.get(9)?,
    })
}

pub fn project_by_slug(conn: &Connection, slug: &str) -> Result<Option<Project>> {
    conn.query_row("SELECT slug, path FROM projects WHERE slug = ?", [slug], |r| {
        Ok(Project { slug: r.get(0)?, path: r.get(1)? })
    })
    .optional()
    .context("failed to query project")
}

/// Non-archived projects ordered by slug, optionally restricted to a space.
pub fn projects(conn: &Connection, space: Option<&str>) -> Result<Vec<Project>> {
    let mut sql = String::from(
        "SELECT p.slug, p.path FROM projects p \
         JOIN project_statuses ps ON ps.slug = p.status \
         LEFT JOIN spaces s ON s.id = p.space_id \
         WHERE ps.is_archived = 0",
    );
    let mut args: Vec<String> = Vec::new();
    if let Some(space) = space {
        sql.push_str(" AND s.slug = ?");
        args.push(space.to_string());
    }
    sql.push_str(" ORDER BY p.slug");
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(rusqlite::params_from_iter(args.iter()), |r| {
        Ok(Project { slug: r.get(0)?, path: r.get(1)? })
    })?;
    rows.collect::<rusqlite::Result<_>>().context("failed to read projects")
}

pub fn space_by_slug(conn: &Connection, slug: &str) -> Result<Option<Space>> {
    conn.query_row("SELECT name, alias, path FROM spaces WHERE slug = ?", [slug], |r| {
        Ok(Space { name: r.get(0)?, alias: r.get(1)?, path: r.get(2)? })
    })
    .optional()
    .context("failed to query space")
}

pub fn statuses(conn: &Connection) -> Result<Vec<Status>> {
    let mut stmt = conn.prepare(
        "SELECT slug, name, description, is_closed, is_hidden, icon \
         FROM task_statuses ORDER BY sort_order ASC, slug ASC",
    )?;
    let rows = stmt.query_map([], |r| {
        Ok(Status {
            slug: r.get(0)?,
            name: r.get(1)?,
            description: r.get(2)?,
            is_closed: r.get::<_, i64>(3)? != 0,
            is_hidden: r.get::<_, i64>(4)? != 0,
            icon: r.get(5)?,
        })
    })?;
    rows.collect::<rusqlite::Result<_>>().context("failed to read task_statuses")
}

/// Live top-level tasks in a scope, grouped by owner then order key (v1 order).
pub fn top_level_tasks(conn: &Connection, scope: Scope) -> Result<Vec<Task>> {
    let (filter, slug) = match scope {
        Scope::Project(s) => ("p.slug = ?1", s),
        Scope::Space(s) => (
            "(p.space_id = (SELECT id FROM spaces WHERE slug = ?1) \
             OR t.space_id = (SELECT id FROM spaces WHERE slug = ?1))",
            s,
        ),
        Scope::SpaceNative(s) => ("t.space_id = (SELECT id FROM spaces WHERE slug = ?1)", s),
    };
    let sql = format!(
        "{CRUMBS} SELECT {TASK_COLS} {TASK_JOINS} \
         WHERE t.deleted_at IS NULL AND t.parent_id IS NULL AND {filter} \
         ORDER BY COALESCE(p.slug, s.slug) ASC, \
           COALESCE(t.\"order\", printf('%012d', COALESCE(t.seq, t.child_seq))) ASC, t.seq ASC"
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(params![slug], task_from_row)?;
    rows.collect::<rusqlite::Result<_>>().context("failed to read tasks")
}

/// Every live descendant of a task, depth-first by `child_seq`.
pub fn descendants(conn: &Connection, root: i64) -> Result<Vec<Task>> {
    let sql = format!(
        "{CRUMBS}, walk(id, sortkey) AS (\
           SELECT id, printf('%012d', child_seq) FROM tasks WHERE parent_id = ?1 \
           UNION ALL \
           SELECT t.id, w.sortkey || '.' || printf('%012d', t.child_seq) \
             FROM tasks t JOIN walk w ON t.parent_id = w.id) \
         SELECT {TASK_COLS} {TASK_JOINS} JOIN walk w ON w.id = t.id \
         WHERE t.deleted_at IS NULL ORDER BY w.sortkey ASC"
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(params![root], task_from_row)?;
    rows.collect::<rusqlite::Result<_>>().context("failed to read descendants")
}

/// `meta.top_n`, defaulting to 10. Negative means unlimited.
pub fn top_n(conn: &Connection) -> Result<i64> {
    let raw: Option<String> = conn
        .query_row("SELECT value FROM meta WHERE key = 'top_n'", [], |r| r.get(0))
        .optional()
        .context("failed to read meta.top_n")?;
    raw.map(|v| v.trim().parse::<i64>().context("meta.top_n is not an integer"))
        .transpose()
        .map(|v| v.unwrap_or(10))
}

/// Unresolved `blocks` edges for the whole workspace (iss-0071).
///
/// A task is blocked when it is open and depends on a task whose status is
/// open; blockedness inherits down the parent chain.
pub struct Blockers {
    /// task id -> external ids of its own unresolved blockers, sorted.
    direct: HashMap<i64, Vec<String>>,
    /// task id -> (parent id, is_closed) for every live task.
    chain: HashMap<i64, (Option<i64>, bool)>,
}

impl Blockers {
    pub fn load(conn: &Connection) -> Result<Self> {
        let sql = format!(
            "{CRUMBS} SELECT d.task_id, \
               COALESCE(p.alias, s.alias) || '-' || printf('%04d', cr.root_seq) || cr.suffix \
             FROM task_deps d \
             JOIN tasks b ON b.id = d.depends_on \
             JOIN task_statuses st ON st.slug = b.status \
             LEFT JOIN projects p ON p.id = b.project_id \
             LEFT JOIN spaces s ON s.id = b.space_id \
             JOIN crumbs cr ON cr.id = b.id \
             WHERE d.kind = 'blocks' AND st.is_closed = 0 AND b.deleted_at IS NULL \
             ORDER BY d.task_id, 2"
        );
        let mut direct: HashMap<i64, Vec<String>> = HashMap::new();
        let mut stmt = conn.prepare(&sql)?;
        for row in stmt.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)))? {
            let (id, ext) = row?;
            direct.entry(id).or_default().push(ext);
        }
        let mut chain = HashMap::new();
        let mut stmt = conn.prepare(
            "SELECT t.id, t.parent_id, st.is_closed FROM tasks t \
             JOIN task_statuses st ON st.slug = t.status WHERE t.deleted_at IS NULL",
        )?;
        for row in stmt.query_map([], |r| {
            Ok((r.get::<_, i64>(0)?, r.get::<_, Option<i64>>(1)?, r.get::<_, i64>(2)? != 0))
        })? {
            let (id, parent, closed) = row?;
            chain.insert(id, (parent, closed));
        }
        Ok(Self { direct, chain })
    }

    /// External ids of the unresolved blockers of `id`, own edges first, then inherited.
    pub fn of(&self, id: i64) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        let mut cursor = Some(id);
        while let Some(cur) = cursor {
            let Some(&(parent, closed)) = self.chain.get(&cur) else { break };
            if closed {
                break;
            }
            if let Some(own) = self.direct.get(&cur) {
                for b in own {
                    if !out.contains(b) {
                        out.push(b.clone());
                    }
                }
            }
            cursor = parent;
        }
        out
    }
}
