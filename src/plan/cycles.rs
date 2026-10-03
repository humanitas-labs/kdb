//! Cycles: dated planning windows (`C-14`) that tasks can be scheduled into.

use anyhow::{Context, Result, bail};
use rusqlite::{Connection, OptionalExtension, Row, params};
use serde::Serialize;

use super::{kv, table};

pub const STATUSES: &[&str] = &["planned", "active", "done", "abandoned"];

#[derive(Debug, Clone, Serialize)]
pub struct Cycle {
    pub id: i64,
    pub key: String,
    pub start_date: String,
    pub end_date: String,
    pub description: Option<String>,
    pub status: String,
    pub path: Option<String>,
    pub created_at: String,
}

const SELECT: &str = "SELECT id, key, start_date, end_date, description, status, path, created_at FROM cycles";

fn from_row(r: &Row) -> rusqlite::Result<Cycle> {
    Ok(Cycle {
        id: r.get(0)?,
        key: r.get(1)?,
        start_date: r.get(2)?,
        end_date: r.get(3)?,
        description: r.get(4)?,
        status: r.get(5)?,
        path: r.get(6)?,
        created_at: r.get(7)?,
    })
}

fn check_status(s: &str) -> Result<()> {
    if !STATUSES.contains(&s) {
        bail!("invalid status '{s}' (expected {})", STATUSES.join(", "));
    }
    Ok(())
}

pub fn list(conn: &Connection) -> Result<Vec<Cycle>> {
    let mut stmt = conn.prepare(&format!("{SELECT} ORDER BY start_date DESC"))?;
    let rows = stmt.query_map([], from_row)?;
    rows.collect::<rusqlite::Result<_>>().context("failed to read cycles")
}

pub fn get(conn: &Connection, key: &str) -> Result<Option<Cycle>> {
    conn.prepare(&format!("{SELECT} WHERE key = ?"))?
        .query_row([key], from_row)
        .optional()
        .context("failed to query cycle")
}

pub fn require(conn: &Connection, key: &str) -> Result<Cycle> {
    get(conn, key)?.with_context(|| format!("cycle not found: {key}"))
}

pub struct AddArgs<'a> {
    pub key: &'a str,
    pub start_date: &'a str,
    pub end_date: &'a str,
    pub description: Option<&'a str>,
    pub status: Option<&'a str>,
    pub path: Option<&'a str>,
}

pub fn add(conn: &Connection, a: AddArgs) -> Result<Cycle> {
    let status = a.status.unwrap_or("planned");
    check_status(status)?;
    conn.execute(
        "INSERT INTO cycles (key, start_date, end_date, description, status, path) VALUES (?, ?, ?, ?, ?, ?)",
        params![a.key, a.start_date, a.end_date, a.description, status, a.path],
    )
    .with_context(|| format!("failed to insert cycle {}", a.key))?;
    require(conn, a.key)
}

#[derive(Default)]
pub struct EditArgs<'a> {
    pub start_date: Option<&'a str>,
    pub end_date: Option<&'a str>,
    pub description: Option<&'a str>,
    pub status: Option<&'a str>,
    pub path: Option<&'a str>,
}

pub fn edit(conn: &Connection, key: &str, a: EditArgs) -> Result<Cycle> {
    if a.start_date.is_none() && a.end_date.is_none() && a.description.is_none() && a.status.is_none() && a.path.is_none() {
        bail!("no fields to update");
    }
    require(conn, key)?;
    if let Some(s) = a.status {
        check_status(s)?;
    }
    conn.execute(
        "UPDATE cycles SET start_date = COALESCE(?, start_date), end_date = COALESCE(?, end_date), \
            description = COALESCE(?, description), status = COALESCE(?, status), path = COALESCE(?, path) WHERE key = ?",
        params![a.start_date, a.end_date, a.description, a.status, a.path, key],
    )
    .with_context(|| format!("failed to update cycle {key}"))?;
    require(conn, key)
}

pub fn render_list(cycles: &[Cycle]) -> String {
    if cycles.is_empty() {
        return "(no cycles)\n".into();
    }
    let rows: Vec<Vec<String>> = cycles
        .iter()
        .map(|c| {
            vec![
                c.key.clone(),
                c.status.clone(),
                c.start_date.clone(),
                c.end_date.clone(),
                c.description.clone().unwrap_or_default(),
            ]
        })
        .collect();
    // v1 pads `status` to at least 7 and the dates to 10; padded headers set those minimums.
    table(&["key", "status ", "start     ", "end       ", "description"], &rows, &[], &[])
}

pub fn render_show(c: &Cycle) -> String {
    let mut out = String::new();
    kv(&mut out, 13, "key", &c.key);
    kv(&mut out, 13, "status", &c.status);
    kv(&mut out, 13, "start_date", &c.start_date);
    kv(&mut out, 13, "end_date", &c.end_date);
    if let Some(p) = &c.path {
        kv(&mut out, 13, "path", p);
    }
    if let Some(d) = &c.description {
        kv(&mut out, 13, "description", d);
    }
    kv(&mut out, 13, "created_at", &c.created_at);
    out
}
