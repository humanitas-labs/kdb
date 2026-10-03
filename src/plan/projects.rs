//! Projects: slug, 2–6 char uppercase alias (task-id prefix), root-relative
//! path, lifecycle status from `project_statuses`, optional owning space.

use std::path::Path;

use anyhow::{Context, Result, bail};
use rusqlite::{Connection, OptionalExtension, Row, params};
use serde::Serialize;

use super::{dash, kv, table};

#[derive(Debug, Clone, Serialize)]
pub struct Project {
    pub id: i64,
    pub slug: String,
    pub alias: String,
    pub name: String,
    pub path: String,
    pub status: String,
    pub description: Option<String>,
    pub space_id: Option<i64>,
    pub space_slug: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

const SELECT: &str = "SELECT p.id, p.slug, p.alias, p.name, p.path, p.status, p.description, \
    p.space_id, s.slug, p.created_at, p.updated_at \
    FROM projects p LEFT JOIN spaces s ON s.id = p.space_id";

fn from_row(r: &Row) -> rusqlite::Result<Project> {
    Ok(Project {
        id: r.get(0)?,
        slug: r.get(1)?,
        alias: r.get(2)?,
        name: r.get(3)?,
        path: r.get(4)?,
        status: r.get(5)?,
        description: r.get(6)?,
        space_id: r.get(7)?,
        space_slug: r.get(8)?,
        created_at: r.get(9)?,
        updated_at: r.get(10)?,
    })
}

/// Projects ordered by slug; archived statuses excluded unless `include_archived`.
pub fn list(conn: &Connection, include_archived: bool, space: Option<&str>) -> Result<Vec<Project>> {
    let mut sql = SELECT.to_string();
    let mut wheres = Vec::new();
    if !include_archived {
        sql.push_str(" JOIN project_statuses ps ON ps.slug = p.status");
        wheres.push("ps.is_archived = 0");
    }
    if space.is_some() {
        wheres.push("s.slug = ?1");
    }
    if !wheres.is_empty() {
        sql = format!("{sql} WHERE {}", wheres.join(" AND "));
    }
    sql.push_str(" ORDER BY p.slug");
    let mut stmt = conn.prepare(&sql)?;
    let rows = match space {
        Some(s) => stmt.query_map([s], from_row)?,
        None => stmt.query_map([], from_row)?,
    };
    rows.collect::<rusqlite::Result<_>>().context("failed to read projects")
}

pub fn get(conn: &Connection, slug: &str) -> Result<Option<Project>> {
    conn.prepare(&format!("{SELECT} WHERE p.slug = ?"))?
        .query_row([slug], from_row)
        .optional()
        .context("failed to query project")
}

pub fn require(conn: &Connection, slug: &str) -> Result<Project> {
    get(conn, slug)?.with_context(|| format!("project not found: {slug}"))
}

pub struct AddArgs<'a> {
    pub slug: &'a str,
    pub alias: &'a str,
    pub name: Option<&'a str>,
    pub path: &'a str,
    pub description: Option<&'a str>,
    pub space_id: Option<i64>,
}

pub fn add(conn: &Connection, a: AddArgs) -> Result<Project> {
    conn.execute(
        "INSERT INTO projects (slug, alias, name, path, description, space_id) VALUES (?, ?, ?, ?, ?, ?)",
        params![a.slug, a.alias.to_ascii_uppercase(), a.name.unwrap_or(a.slug), a.path, a.description, a.space_id],
    )
    .with_context(|| format!("failed to insert project {}", a.slug))?;
    require(conn, a.slug)
}

#[derive(Default)]
pub struct EditArgs<'a> {
    pub alias: Option<&'a str>,
    pub name: Option<&'a str>,
    pub path: Option<&'a str>,
    pub status: Option<&'a str>,
    pub description: Option<&'a str>,
    /// Outer `None` leaves membership alone; `Some(None)` detaches.
    pub space_id: Option<Option<i64>>,
}

pub fn edit(conn: &Connection, slug: &str, a: EditArgs) -> Result<Project> {
    if a.alias.is_none() && a.name.is_none() && a.path.is_none() && a.status.is_none() && a.description.is_none() && a.space_id.is_none() {
        bail!("no fields to update");
    }
    require(conn, slug)?;
    conn.execute(
        "UPDATE projects SET alias = COALESCE(?, alias), name = COALESCE(?, name), path = COALESCE(?, path), \
            status = COALESCE(?, status), description = COALESCE(?, description), \
            space_id = CASE WHEN ? THEN ? ELSE space_id END, \
            updated_at = strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE slug = ?",
        params![
            a.alias.map(str::to_ascii_uppercase),
            a.name,
            a.path,
            a.status,
            a.description,
            a.space_id.is_some(),
            a.space_id.flatten(),
            slug
        ],
    )
    .with_context(|| format!("failed to update project {slug}"))?;
    require(conn, slug)
}

/// The project whose `path` is the deepest prefix of `cwd` (relative to `root`).
pub fn resolve_active(conn: &Connection, root: &Path, cwd: &Path) -> Result<Option<Project>> {
    let Ok(rel) = cwd.strip_prefix(root) else { return Ok(None) };
    let mut best: Option<Project> = None;
    for p in list(conn, true, None)? {
        let depth = Path::new(&p.path).components().count();
        let better = best.as_ref().is_none_or(|b| Path::new(&b.path).components().count() < depth);
        if rel.starts_with(&p.path) && better {
            best = Some(p);
        }
    }
    Ok(best)
}

pub fn render_list(projects: &[Project]) -> String {
    if projects.is_empty() {
        return "(no projects)\n".into();
    }
    let rows: Vec<Vec<String>> = projects
        .iter()
        .map(|p| {
            vec![
                p.slug.clone(),
                p.alias.clone(),
                p.name.clone(),
                p.status.clone(),
                dash(p.space_slug.as_deref()).into(),
                p.path.clone(),
            ]
        })
        .collect();
    table(&["slug", "alias", "name", "status", "space", "path"], &rows, &[], &[])
}

pub fn render_show(p: &Project) -> String {
    let mut out = String::new();
    kv(&mut out, 13, "slug", &p.slug);
    kv(&mut out, 13, "alias", &p.alias);
    kv(&mut out, 13, "name", &p.name);
    kv(&mut out, 13, "path", &p.path);
    kv(&mut out, 13, "status", &p.status);
    kv(&mut out, 13, "space", dash(p.space_slug.as_deref()));
    if let Some(d) = &p.description {
        kv(&mut out, 13, "description", d);
    }
    kv(&mut out, 13, "created_at", &p.created_at);
    kv(&mut out, 13, "updated_at", &p.updated_at);
    out
}
