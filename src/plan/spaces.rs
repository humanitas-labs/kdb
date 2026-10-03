//! Spaces: named groupings of projects. A space carries its own uppercase
//! alias so it can own space-native tasks (`ICE-0003`).

use anyhow::{Context, Result, bail};
use rusqlite::{Connection, OptionalExtension, Row, params};
use serde::Serialize;

use super::{dash, kv, table};

#[derive(Debug, Clone, Serialize)]
pub struct Space {
    pub id: i64,
    pub slug: String,
    pub name: String,
    pub alias: Option<String>,
    pub path: Option<String>,
    pub status: String,
    pub description: Option<String>,
    pub project_count: i64,
    pub created_at: String,
    pub updated_at: String,
}

const SELECT: &str = "SELECT s.id, s.slug, s.name, s.alias, s.path, s.status, s.description, \
    (SELECT COUNT(*) FROM projects p WHERE p.space_id = s.id), s.created_at, s.updated_at FROM spaces s";

fn from_row(r: &Row) -> rusqlite::Result<Space> {
    Ok(Space {
        id: r.get(0)?,
        slug: r.get(1)?,
        name: r.get(2)?,
        alias: r.get(3)?,
        path: r.get(4)?,
        status: r.get(5)?,
        description: r.get(6)?,
        project_count: r.get(7)?,
        created_at: r.get(8)?,
        updated_at: r.get(9)?,
    })
}

pub fn list(conn: &Connection, include_archived: bool) -> Result<Vec<Space>> {
    let filter = if include_archived {
        ""
    } else {
        " JOIN project_statuses ps ON ps.slug = s.status WHERE ps.is_archived = 0"
    };
    let mut stmt = conn.prepare(&format!("{SELECT}{filter} ORDER BY s.slug"))?;
    let rows = stmt.query_map([], from_row)?;
    rows.collect::<rusqlite::Result<_>>().context("failed to read spaces")
}

pub fn get(conn: &Connection, slug: &str) -> Result<Option<Space>> {
    conn.prepare(&format!("{SELECT} WHERE s.slug = ?"))?
        .query_row([slug], from_row)
        .optional()
        .context("failed to query space")
}

pub fn require(conn: &Connection, slug: &str) -> Result<Space> {
    get(conn, slug)?.with_context(|| format!("space not found: {slug}"))
}

pub struct AddArgs<'a> {
    pub slug: &'a str,
    pub name: Option<&'a str>,
    pub alias: &'a str,
    pub path: Option<&'a str>,
    pub description: Option<&'a str>,
}

pub fn add(conn: &Connection, a: AddArgs) -> Result<Space> {
    conn.execute(
        "INSERT INTO spaces (slug, name, alias, path, description) VALUES (?, ?, ?, ?, ?)",
        params![a.slug, a.name.unwrap_or(a.slug), a.alias.to_ascii_uppercase(), a.path, a.description],
    )
    .with_context(|| format!("failed to insert space {}", a.slug))?;
    require(conn, a.slug)
}

#[derive(Default)]
pub struct EditArgs<'a> {
    pub name: Option<&'a str>,
    pub alias: Option<&'a str>,
    pub path: Option<&'a str>,
    pub status: Option<&'a str>,
    pub description: Option<&'a str>,
}

pub fn edit(conn: &Connection, slug: &str, a: EditArgs) -> Result<Space> {
    if a.name.is_none() && a.alias.is_none() && a.path.is_none() && a.status.is_none() && a.description.is_none() {
        bail!("no fields to update");
    }
    require(conn, slug)?;
    conn.execute(
        "UPDATE spaces SET name = COALESCE(?, name), alias = COALESCE(?, alias), path = COALESCE(?, path), \
            status = COALESCE(?, status), description = COALESCE(?, description), \
            updated_at = strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE slug = ?",
        params![a.name, a.alias.map(str::to_ascii_uppercase), a.path, a.status, a.description, slug],
    )
    .with_context(|| format!("failed to update space {slug}"))?;
    require(conn, slug)
}

pub fn render_list(spaces: &[Space]) -> String {
    if spaces.is_empty() {
        return "(no spaces)\n".into();
    }
    let rows: Vec<Vec<String>> = spaces
        .iter()
        .map(|s| {
            vec![
                s.slug.clone(),
                dash(s.alias.as_deref()).into(),
                s.name.clone(),
                s.status.clone(),
                s.project_count.to_string(),
                dash(s.path.as_deref()).into(),
            ]
        })
        .collect();
    table(&["slug", "alias", "name", "status", "projects", "path"], &rows, &[], &[4])
}

pub fn render_show(s: &Space) -> String {
    let mut out = String::new();
    kv(&mut out, 13, "slug", &s.slug);
    kv(&mut out, 13, "name", &s.name);
    kv(&mut out, 13, "alias", dash(s.alias.as_deref()));
    kv(&mut out, 13, "path", dash(s.path.as_deref()));
    kv(&mut out, 13, "status", &s.status);
    kv(&mut out, 13, "projects", s.project_count);
    if let Some(d) = &s.description {
        kv(&mut out, 13, "description", d);
    }
    kv(&mut out, 13, "created_at", &s.created_at);
    kv(&mut out, 13, "updated_at", &s.updated_at);
    out
}
