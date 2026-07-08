//! Labels table access and display.
//!
//! A label is a free-form tag (identified by a unique `slug`) that can
//! be attached to many tasks via `task_labels`.

use anyhow::{Context, Result, bail};
use rusqlite::{Connection, OptionalExtension, params};
use serde::Serialize;

use crate::color;

// ----------------------------------------
// projects/kdb/src/labels.rs
//
// pub struct Label                     L38
//   fn from_row()                      L46
// const SELECT_COLS                    L56
// pub fn list()                        L59
// pub fn get_by_slug()                 L68
// pub fn for_task()                    L77
// pub struct AddArgs                   L89
// pub fn add()                         L96
// pub struct EditArgs                 L108
//   fn is_empty()                     L114
// pub fn edit()                       L119
// pub fn attach()                     L138
// pub fn detach()                     L148
// pub fn upsert_by_slug()             L160
// pub fn render_list()                L174
// pub fn render_show()                L207
// mod tests                           L219
// fn setup()                          L227
// fn attach_detach_label_to_task()    L235
// fn add_edit()                       L279
// ----------------------------------------

#[derive(Debug, Clone, Serialize)]
pub struct Label {
    pub id: i64,
    pub slug: String,
    pub name: String,
    pub color: Option<String>,
}

impl Label {
    fn from_row(row: &rusqlite::Row) -> rusqlite::Result<Self> {
        Ok(Self {
            id: row.get(0)?,
            slug: row.get(1)?,
            name: row.get(2)?,
            color: row.get(3)?,
        })
    }
}

const SELECT_COLS: &str = "id, slug, name, color";

/// List all labels, ordered by slug.
pub fn list(conn: &Connection) -> Result<Vec<Label>> {
    let sql = format!("SELECT {SELECT_COLS} FROM labels ORDER BY slug");
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map([], Label::from_row)?;
    rows.collect::<rusqlite::Result<Vec<_>>>()
        .context("failed to read labels")
}

/// Fetch a label by slug. Returns `None` if no match.
pub fn get_by_slug(conn: &Connection, slug: &str) -> Result<Option<Label>> {
    let sql = format!("SELECT {SELECT_COLS} FROM labels WHERE slug = ?");
    let mut stmt = conn.prepare(&sql)?;
    stmt.query_row([slug], Label::from_row)
        .optional()
        .context("failed to query label")
}

/// Return the labels attached to the given task, ordered by slug.
pub fn for_task(conn: &Connection, task_id: i64) -> Result<Vec<Label>> {
    let sql = format!(
        "SELECT {SELECT_COLS} FROM labels l \
         JOIN task_labels tl ON tl.label_id = l.id \
         WHERE tl.task_id = ? ORDER BY l.slug"
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map([task_id], Label::from_row)?;
    rows.collect::<rusqlite::Result<Vec<_>>>()
        .context("failed to read task labels")
}

pub struct AddArgs<'a> {
    pub slug: &'a str,
    pub name: Option<&'a str>,
    pub color: Option<&'a str>,
}

/// Insert a new label. Fails if `slug` already exists.
pub fn add(conn: &Connection, args: AddArgs) -> Result<Label> {
    let name = args.name.unwrap_or(args.slug);
    conn.execute(
        "INSERT INTO labels (slug, name, color) VALUES (?, ?, ?)",
        params![args.slug, name, args.color],
    )
    .with_context(|| format!("failed to insert label {}", args.slug))?;
    get_by_slug(conn, args.slug)?
        .with_context(|| format!("label {} missing after insert", args.slug))
}

#[derive(Default)]
pub struct EditArgs<'a> {
    pub name: Option<&'a str>,
    pub color: Option<&'a str>,
}

impl EditArgs<'_> {
    fn is_empty(&self) -> bool {
        self.name.is_none() && self.color.is_none()
    }
}

pub fn edit(conn: &Connection, slug: &str, args: EditArgs) -> Result<Label> {
    if args.is_empty() {
        bail!("no fields to update");
    }
    if get_by_slug(conn, slug)?.is_none() {
        bail!("label not found: {slug}");
    }
    conn.execute(
        "UPDATE labels SET \
            name  = COALESCE(?, name), \
            color = COALESCE(?, color) \
         WHERE slug = ?",
        params![args.name, args.color, slug],
    )
    .with_context(|| format!("failed to update label {slug}"))?;
    get_by_slug(conn, slug)?.with_context(|| format!("label {slug} missing after update"))
}

/// Attach a label to a task. Idempotent.
pub fn attach(conn: &Connection, task_id: i64, label_id: i64) -> Result<()> {
    conn.execute(
        "INSERT OR IGNORE INTO task_labels (task_id, label_id) VALUES (?, ?)",
        params![task_id, label_id],
    )
    .context("failed to attach label")?;
    Ok(())
}

/// Detach a label from a task. Returns true if a row was removed.
pub fn detach(conn: &Connection, task_id: i64, label_id: i64) -> Result<bool> {
    let rows = conn
        .execute(
            "DELETE FROM task_labels WHERE task_id = ? AND label_id = ?",
            params![task_id, label_id],
        )
        .context("failed to detach label")?;
    Ok(rows > 0)
}

/// Resolve a label slug to its id, creating the row if it doesn't exist
/// yet (useful for importers that want to accept free-form tags).
pub fn upsert_by_slug(conn: &Connection, slug: &str) -> Result<Label> {
    if let Some(existing) = get_by_slug(conn, slug)? {
        return Ok(existing);
    }
    add(
        conn,
        AddArgs {
            slug,
            name: None,
            color: None,
        },
    )
}

pub fn render_list(labels: &[Label]) -> String {
    if labels.is_empty() {
        return String::from("(no labels)\n");
    }
    let slug_w = labels
        .iter()
        .map(|l| l.slug.len())
        .max()
        .unwrap_or(4)
        .max(4);
    let name_w = labels
        .iter()
        .map(|l| l.name.len())
        .max()
        .unwrap_or(4)
        .max(4);

    let mut out = String::new();
    out.push_str(&format!(
        "{:<slug_w$}  {:<name_w$}  color\n",
        "slug", "name",
    ));
    for l in labels {
        let slug_cell = color::pad_colored(&l.slug, l.color.as_deref(), slug_w);
        out.push_str(&format!(
            "{slug_cell}  {:<name_w$}  {}\n",
            l.name,
            l.color.as_deref().unwrap_or(""),
        ));
    }
    out
}

pub fn render_show(l: &Label) -> String {
    let mut out = String::new();
    let colored_slug = color::colorize_for_stdout(&l.slug, l.color.as_deref());
    out.push_str(&format!("slug:  {colored_slug}\n"));
    out.push_str(&format!("name:  {}\n", l.name));
    if let Some(c) = &l.color {
        out.push_str(&format!("color: {c}\n"));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db;
    use crate::projects::{self, AddArgs as ProjAddArgs};
    use crate::tasks::{self, AddArgs as TaskAddArgs};
    use crate::workspace::root::ROOT_MARKER;
    use tempfile::TempDir;

    fn setup() -> (TempDir, Connection) {
        let tmp = TempDir::new().unwrap();
        std::fs::create_dir(tmp.path().join(ROOT_MARKER)).unwrap();
        let conn = db::open(tmp.path()).unwrap();
        (tmp, conn)
    }

    #[test]
    fn attach_detach_label_to_task() {
        let (_tmp, mut conn) = setup();
        let p = projects::add(
            &conn,
            ProjAddArgs {
                slug: "kdb",
                alias: "KDB",
                name: None,
                path: "projects/kdb",
                description: None,
                space_id: None,
            },
        )
        .unwrap();
        let t = tasks::add(
            &mut conn,
            TaskAddArgs {
                project_id: Some(p.id),
                space_id: None,
                title: "x",
                body: None,
                priority: None,
                cycle_id: None,
                parent_id: None,
                seq: None,
                status: None,
                order: None,
            },
        )
        .unwrap();
        let l = upsert_by_slug(&conn, "forge").unwrap();

        attach(&conn, t.task.id, l.id).unwrap();
        attach(&conn, t.task.id, l.id).unwrap(); // idempotent

        let attached = for_task(&conn, t.task.id).unwrap();
        assert_eq!(attached.len(), 1);
        assert_eq!(attached[0].slug, "forge");

        assert!(detach(&conn, t.task.id, l.id).unwrap());
        assert!(for_task(&conn, t.task.id).unwrap().is_empty());
    }

    #[test]
    fn add_edit() {
        let (_tmp, conn) = setup();
        add(
            &conn,
            AddArgs {
                slug: "bug",
                name: Some("Bug"),
                color: Some("#ff0000"),
            },
        )
        .unwrap();
        edit(
            &conn,
            "bug",
            EditArgs {
                color: Some("#aa0000"),
                ..Default::default()
            },
        )
        .unwrap();
        let l = get_by_slug(&conn, "bug").unwrap().unwrap();
        assert_eq!(l.color.as_deref(), Some("#aa0000"));
        assert_eq!(l.name, "Bug");
    }
}
