//! Task and project statuses. Both tables share a shape; `Kind` picks the table
//! and its flag column (`is_closed` for tasks, `is_archived` for projects).
//! Task statuses also carry an `icon` name (iss-0070).

use anyhow::{Context, Result, bail};
use rusqlite::{Connection, OptionalExtension, Row, params};
use serde::Serialize;

use super::{kv, parse_hex, table};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Project,
    Task,
}

impl Kind {
    pub fn table(self) -> &'static str {
        match self {
            Kind::Project => "project_statuses",
            Kind::Task => "task_statuses",
        }
    }
    pub fn flag_column(self) -> &'static str {
        match self {
            Kind::Project => "is_archived",
            Kind::Task => "is_closed",
        }
    }
    pub fn flag_label(self) -> &'static str {
        match self {
            Kind::Project => "archived",
            Kind::Task => "closed",
        }
    }
    /// Only task statuses have an icon column.
    fn icon_expr(self) -> &'static str {
        match self {
            Kind::Project => "NULL",
            Kind::Task => "icon",
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct Status {
    pub slug: String,
    pub name: String,
    pub description: Option<String>,
    pub color: Option<String>,
    /// `is_closed` (tasks) or `is_archived` (projects).
    pub flag: bool,
    pub sort_order: i64,
    pub is_hidden: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
}

fn from_row(r: &Row) -> rusqlite::Result<Status> {
    Ok(Status {
        slug: r.get(0)?,
        name: r.get(1)?,
        description: r.get(2)?,
        color: r.get(3)?,
        flag: r.get::<_, i64>(4)? != 0,
        sort_order: r.get(5)?,
        is_hidden: r.get::<_, i64>(6)? != 0,
        icon: r.get(7)?,
    })
}

fn select(kind: Kind) -> String {
    format!(
        "SELECT slug, name, description, color, {}, sort_order, is_hidden, {} FROM {}",
        kind.flag_column(),
        kind.icon_expr(),
        kind.table()
    )
}

pub fn list(conn: &Connection, kind: Kind) -> Result<Vec<Status>> {
    let mut stmt = conn.prepare(&format!("{} ORDER BY sort_order ASC, slug ASC", select(kind)))?;
    let rows = stmt.query_map([], from_row)?;
    rows.collect::<rusqlite::Result<_>>().with_context(|| format!("failed to read {}", kind.table()))
}

pub fn get(conn: &Connection, kind: Kind, slug: &str) -> Result<Option<Status>> {
    conn.prepare(&format!("{} WHERE slug = ?", select(kind)))?
        .query_row([slug], from_row)
        .optional()
        .with_context(|| format!("failed to query {}", kind.table()))
}

pub fn require(conn: &Connection, kind: Kind, slug: &str) -> Result<Status> {
    get(conn, kind, slug)?.with_context(|| format!("{} not found: {slug}", kind.table()))
}

fn check_color(c: Option<&str>) -> Result<()> {
    match c {
        Some(c) if parse_hex(c).is_none() => bail!("invalid color '{c}' — expected #RRGGBB"),
        _ => Ok(()),
    }
}

fn check_icon(kind: Kind, icon: Option<&str>) -> Result<()> {
    if icon.is_some() && kind == Kind::Project {
        bail!("--icon applies only to task statuses");
    }
    Ok(())
}

pub struct AddArgs<'a> {
    pub slug: &'a str,
    pub name: Option<&'a str>,
    pub description: Option<&'a str>,
    pub color: Option<&'a str>,
    pub flag: bool,
    pub sort_order: Option<i64>,
    pub is_hidden: bool,
    pub icon: Option<&'a str>,
}

pub fn add(conn: &Connection, kind: Kind, a: AddArgs) -> Result<Status> {
    check_color(a.color)?;
    check_icon(kind, a.icon)?;
    let sort_order = match a.sort_order {
        Some(o) => o,
        None => conn
            .query_row(&format!("SELECT COALESCE(MAX(sort_order), -1) + 1 FROM {}", kind.table()), [], |r| r.get(0))
            .unwrap_or(0),
    };
    let sql = format!(
        "INSERT INTO {} (slug, name, description, color, {}, sort_order, is_hidden) VALUES (?, ?, ?, ?, ?, ?, ?)",
        kind.table(),
        kind.flag_column()
    );
    conn.execute(
        &sql,
        params![a.slug, a.name.unwrap_or(a.slug), a.description, a.color, a.flag as i64, sort_order, a.is_hidden as i64],
    )
    .with_context(|| format!("failed to insert into {}", kind.table()))?;
    if let Some(icon) = a.icon {
        conn.execute("UPDATE task_statuses SET icon = ? WHERE slug = ?", params![icon, a.slug])?;
    }
    require(conn, kind, a.slug)
}

#[derive(Default)]
pub struct EditArgs<'a> {
    pub name: Option<&'a str>,
    pub description: Option<&'a str>,
    pub color: Option<&'a str>,
    pub flag: Option<bool>,
    pub sort_order: Option<i64>,
    pub is_hidden: Option<bool>,
    pub icon: Option<&'a str>,
}

pub fn edit(conn: &Connection, kind: Kind, slug: &str, a: EditArgs) -> Result<Status> {
    if a.name.is_none() && a.description.is_none() && a.color.is_none() && a.flag.is_none() && a.sort_order.is_none() && a.is_hidden.is_none() && a.icon.is_none() {
        bail!("no fields to update");
    }
    require(conn, kind, slug)?;
    check_color(a.color)?;
    check_icon(kind, a.icon)?;
    let sql = format!(
        "UPDATE {t} SET name = COALESCE(?, name), description = COALESCE(?, description), color = COALESCE(?, color), \
            {f} = COALESCE(?, {f}), sort_order = COALESCE(?, sort_order), is_hidden = COALESCE(?, is_hidden) WHERE slug = ?",
        t = kind.table(),
        f = kind.flag_column()
    );
    conn.execute(
        &sql,
        params![a.name, a.description, a.color, a.flag.map(i64::from), a.sort_order, a.is_hidden.map(i64::from), slug],
    )
    .with_context(|| format!("failed to update {} {slug}", kind.table()))?;
    if let Some(icon) = a.icon {
        // Empty string clears the icon.
        let value = (!icon.is_empty()).then_some(icon);
        conn.execute("UPDATE task_statuses SET icon = ? WHERE slug = ?", params![value, slug])?;
    }
    require(conn, kind, slug)
}

/// Delete a status; refuses while any task or project still uses it.
pub fn remove(conn: &Connection, kind: Kind, slug: &str) -> Result<()> {
    require(conn, kind, slug)?;
    let owner = if kind == Kind::Task { "tasks" } else { "projects" };
    let in_use: i64 = conn
        .query_row(&format!("SELECT COUNT(*) FROM {owner} WHERE status = ?"), [slug], |r| r.get(0))
        .with_context(|| format!("failed to count {owner} using status {slug}"))?;
    if in_use > 0 {
        bail!("cannot remove status '{slug}': still used by {in_use} {owner} row(s)");
    }
    conn.execute(&format!("DELETE FROM {} WHERE slug = ?", kind.table()), [slug])
        .with_context(|| format!("failed to delete {} {slug}", kind.table()))?;
    Ok(())
}

fn yn(b: bool) -> String {
    (if b { "yes" } else { "no" }).into()
}

pub fn render_list(statuses: &[Status], kind: Kind) -> String {
    if statuses.is_empty() {
        return "(no statuses)\n".into();
    }
    let colors: Vec<Option<String>> = statuses.iter().map(|s| s.color.clone()).collect();
    let mut header = vec!["slug", "name", kind.flag_label(), "hidden"];
    let rows: Vec<Vec<String>> = statuses
        .iter()
        .map(|s| {
            let mut row = vec![s.slug.clone(), s.name.clone(), yn(s.flag), yn(s.is_hidden)];
            if kind == Kind::Task {
                row.push(s.icon.clone().unwrap_or_default());
            }
            row.push(s.color.clone().unwrap_or_default());
            row
        })
        .collect();
    if kind == Kind::Task {
        header.push("icon");
    }
    header.push("color");
    table(&header, &rows, &colors, &[])
}

pub fn render_show(s: &Status, kind: Kind) -> String {
    let mut out = String::new();
    kv(&mut out, 13, "slug", &s.slug);
    kv(&mut out, 13, "name", &s.name);
    kv(&mut out, 13, kind.flag_label(), yn(s.flag));
    kv(&mut out, 13, "hidden", yn(s.is_hidden));
    kv(&mut out, 13, "sort_order", s.sort_order);
    if let Some(c) = &s.color {
        kv(&mut out, 13, "color", c);
    }
    if let Some(i) = &s.icon {
        kv(&mut out, 13, "icon", i);
    }
    if let Some(d) = &s.description {
        kv(&mut out, 13, "description", d);
    }
    out
}
