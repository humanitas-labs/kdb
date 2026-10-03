//! Labels and the task ↔ label join.

use anyhow::{Context, Result, bail};
use rusqlite::{Connection, OptionalExtension, Row, params};
use serde::Serialize;

use super::{colorize, table};

#[derive(Debug, Clone, Serialize)]
pub struct Label {
    pub id: i64,
    pub slug: String,
    pub name: String,
    pub color: Option<String>,
}

const SELECT: &str = "SELECT l.id, l.slug, l.name, l.color FROM labels l";

fn from_row(r: &Row) -> rusqlite::Result<Label> {
    Ok(Label { id: r.get(0)?, slug: r.get(1)?, name: r.get(2)?, color: r.get(3)? })
}

pub fn list(conn: &Connection) -> Result<Vec<Label>> {
    let mut stmt = conn.prepare(&format!("{SELECT} ORDER BY l.slug"))?;
    let rows = stmt.query_map([], from_row)?;
    rows.collect::<rusqlite::Result<_>>().context("failed to read labels")
}

pub fn get(conn: &Connection, slug: &str) -> Result<Option<Label>> {
    conn.prepare(&format!("{SELECT} WHERE l.slug = ?"))?
        .query_row([slug], from_row)
        .optional()
        .context("failed to query label")
}

pub fn require(conn: &Connection, slug: &str) -> Result<Label> {
    get(conn, slug)?.with_context(|| format!("label not found: {slug}"))
}

pub fn for_task(conn: &Connection, task_id: i64) -> Result<Vec<Label>> {
    let mut stmt = conn.prepare(&format!(
        "{SELECT} JOIN task_labels tl ON tl.label_id = l.id WHERE tl.task_id = ? ORDER BY l.slug"
    ))?;
    let rows = stmt.query_map([task_id], from_row)?;
    rows.collect::<rusqlite::Result<_>>().context("failed to read task labels")
}

pub fn add(conn: &Connection, slug: &str, name: Option<&str>, color: Option<&str>) -> Result<Label> {
    conn.execute(
        "INSERT INTO labels (slug, name, color) VALUES (?, ?, ?)",
        params![slug, name.unwrap_or(slug), color],
    )
    .with_context(|| format!("failed to insert label {slug}"))?;
    require(conn, slug)
}

pub fn edit(conn: &Connection, slug: &str, name: Option<&str>, color: Option<&str>) -> Result<Label> {
    if name.is_none() && color.is_none() {
        bail!("no fields to update");
    }
    require(conn, slug)?;
    conn.execute(
        "UPDATE labels SET name = COALESCE(?, name), color = COALESCE(?, color) WHERE slug = ?",
        params![name, color, slug],
    )
    .with_context(|| format!("failed to update label {slug}"))?;
    require(conn, slug)
}

/// Existing label by slug, or a new one named after it.
pub fn upsert(conn: &Connection, slug: &str) -> Result<Label> {
    match get(conn, slug)? {
        Some(l) => Ok(l),
        None => add(conn, slug, None, None),
    }
}

pub fn attach(conn: &Connection, task_id: i64, label_id: i64) -> Result<()> {
    conn.execute("INSERT OR IGNORE INTO task_labels (task_id, label_id) VALUES (?, ?)", params![task_id, label_id])
        .context("failed to attach label")?;
    Ok(())
}

/// `true` when a row was removed.
pub fn detach(conn: &Connection, task_id: i64, label_id: i64) -> Result<bool> {
    let n = conn
        .execute("DELETE FROM task_labels WHERE task_id = ? AND label_id = ?", params![task_id, label_id])
        .context("failed to detach label")?;
    Ok(n > 0)
}

pub fn render_list(labels: &[Label]) -> String {
    if labels.is_empty() {
        return "(no labels)\n".into();
    }
    let rows: Vec<Vec<String>> = labels
        .iter()
        .map(|l| vec![l.slug.clone(), l.name.clone(), l.color.clone().unwrap_or_default()])
        .collect();
    let colors: Vec<Option<String>> = labels.iter().map(|l| l.color.clone()).collect();
    table(&["slug", "name", "color"], &rows, &colors, &[])
}

pub fn render_show(l: &Label) -> String {
    let mut out = format!("slug:  {}\nname:  {}\n", colorize(&l.slug, l.color.as_deref()), l.name);
    if let Some(c) = &l.color {
        out.push_str(&format!("color: {c}\n"));
    }
    out
}
