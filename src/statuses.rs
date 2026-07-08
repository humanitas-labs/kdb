//! Project & task status lookup tables.
//!
//! Both `project_statuses` and `task_statuses` share the same shape
//! (slug PK + name + optional color + a boolean flag + sort_order) and
//! differ only in their behavior flag: projects have `is_archived`
//! (hidden from the default list), tasks have `is_closed` (stamps
//! `closed_at` when set).

use anyhow::{Context, Result, bail};
use rusqlite::{Connection, OptionalExtension, params};
use serde::Serialize;

use crate::color;

// ------------------------------------------------
// projects/kdb/src/statuses.rs
//
// pub enum Kind                                L48
//   pub fn table()                             L54
//   pub fn flag_column()                       L62
//   pub fn flag_label()                        L70
//   pub fn as_arg()                            L78
// pub struct Status                            L87
//   fn from_row()                              L98
// const SELECT_COLS_FMT                       L113
// pub fn list()                               L116
// pub fn get()                                L129
// pub struct AddArgs                          L141
// pub fn add()                                L152
// pub struct EditArgs                         L194
//   fn is_empty()                             L204
// pub fn edit()                               L215
// pub fn remove()                             L256
// fn validate_color()                         L282
// pub fn render_list()                        L290
// pub fn render_show()                        L329
// mod tests                                   L353
// fn setup()                                  L359
// fn default_task_statuses_are_seeded()       L367
// fn default_project_statuses_are_seeded()    L382
// fn add_edit_remove_roundtrip()              L392
// fn remove_blocks_if_in_use()                L435
// fn add_rejects_bad_color()                  L473
// ------------------------------------------------

/// Whether a status applies to the `projects` or `tasks` table.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Project,
    Task,
}

impl Kind {
    pub fn table(&self) -> &'static str {
        match self {
            Kind::Project => "project_statuses",
            Kind::Task => "task_statuses",
        }
    }

    /// Column name for the boolean behavior flag.
    pub fn flag_column(&self) -> &'static str {
        match self {
            Kind::Project => "is_archived",
            Kind::Task => "is_closed",
        }
    }

    /// Human-readable name for the flag (used in `show`/`list` output).
    pub fn flag_label(&self) -> &'static str {
        match self {
            Kind::Project => "archived",
            Kind::Task => "closed",
        }
    }

    /// Lowercase arg name for the scope flag (`--tasks` / `--projects`).
    pub fn as_arg(&self) -> &'static str {
        match self {
            Kind::Project => "--projects",
            Kind::Task => "--tasks",
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct Status {
    pub slug: String,
    pub name: String,
    pub description: Option<String>,
    pub color: Option<String>,
    pub flag: bool,
    pub sort_order: i64,
    pub is_hidden: bool,
}

impl Status {
    fn from_row(row: &rusqlite::Row) -> rusqlite::Result<Self> {
        let flag: i64 = row.get(4)?;
        let is_hidden: i64 = row.get(6)?;
        Ok(Self {
            slug: row.get(0)?,
            name: row.get(1)?,
            description: row.get(2)?,
            color: row.get(3)?,
            flag: flag != 0,
            sort_order: row.get(5)?,
            is_hidden: is_hidden != 0,
        })
    }
}

const SELECT_COLS_FMT: &str = "slug, name, description, color, {flag}, sort_order, is_hidden";

/// List statuses of the given kind, ordered by `sort_order` then `slug`.
pub fn list(conn: &Connection, kind: Kind) -> Result<Vec<Status>> {
    let cols = SELECT_COLS_FMT.replace("{flag}", kind.flag_column());
    let sql = format!(
        "SELECT {cols} FROM {table} ORDER BY sort_order ASC, slug ASC",
        table = kind.table()
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map([], Status::from_row)?;
    rows.collect::<rusqlite::Result<Vec<_>>>()
        .with_context(|| format!("failed to read {}", kind.table()))
}

/// Fetch a single status by slug. Returns `None` if no row.
pub fn get(conn: &Connection, kind: Kind, slug: &str) -> Result<Option<Status>> {
    let cols = SELECT_COLS_FMT.replace("{flag}", kind.flag_column());
    let sql = format!(
        "SELECT {cols} FROM {table} WHERE slug = ?",
        table = kind.table()
    );
    let mut stmt = conn.prepare(&sql)?;
    stmt.query_row([slug], Status::from_row)
        .optional()
        .with_context(|| format!("failed to query {}", kind.table()))
}

pub struct AddArgs<'a> {
    pub slug: &'a str,
    pub name: Option<&'a str>,
    pub description: Option<&'a str>,
    pub color: Option<&'a str>,
    pub flag: bool,
    pub sort_order: Option<i64>,
    pub is_hidden: bool,
}

/// Insert a new status. Fails if slug already exists.
pub fn add(conn: &Connection, kind: Kind, args: AddArgs) -> Result<Status> {
    let name = args.name.unwrap_or(args.slug);
    if let Some(c) = args.color {
        validate_color(c)?;
    }
    let sort_order = args.sort_order.unwrap_or_else(|| {
        conn.query_row(
            &format!(
                "SELECT COALESCE(MAX(sort_order), -1) + 1 FROM {table}",
                table = kind.table()
            ),
            [],
            |row| row.get(0),
        )
        .unwrap_or(0)
    });
    let flag = if args.flag { 1i64 } else { 0 };
    let hidden = if args.is_hidden { 1i64 } else { 0 };
    let sql = format!(
        "INSERT INTO {table} (slug, name, description, color, {flag_col}, sort_order, is_hidden) \
         VALUES (?, ?, ?, ?, ?, ?, ?)",
        table = kind.table(),
        flag_col = kind.flag_column(),
    );
    conn.execute(
        &sql,
        params![
            args.slug,
            name,
            args.description,
            args.color,
            flag,
            sort_order,
            hidden
        ],
    )
    .with_context(|| format!("failed to insert into {}", kind.table()))?;
    get(conn, kind, args.slug)?
        .with_context(|| format!("{} {} missing after insert", kind.table(), args.slug))
}

#[derive(Default)]
pub struct EditArgs<'a> {
    pub name: Option<&'a str>,
    pub description: Option<&'a str>,
    pub color: Option<&'a str>,
    pub flag: Option<bool>,
    pub sort_order: Option<i64>,
    pub is_hidden: Option<bool>,
}

impl EditArgs<'_> {
    fn is_empty(&self) -> bool {
        self.name.is_none()
            && self.description.is_none()
            && self.color.is_none()
            && self.flag.is_none()
            && self.sort_order.is_none()
            && self.is_hidden.is_none()
    }
}

/// Update mutable fields on a status. `None` leaves the field unchanged.
pub fn edit(conn: &Connection, kind: Kind, slug: &str, args: EditArgs) -> Result<Status> {
    if args.is_empty() {
        bail!("no fields to update");
    }
    if get(conn, kind, slug)?.is_none() {
        bail!("{} not found: {slug}", kind.table());
    }
    if let Some(c) = args.color {
        validate_color(c)?;
    }
    let flag_int: Option<i64> = args.flag.map(|b| if b { 1 } else { 0 });
    let hidden_int: Option<i64> = args.is_hidden.map(|b| if b { 1 } else { 0 });
    let sql = format!(
        "UPDATE {table} SET \
            name        = COALESCE(?, name), \
            description = COALESCE(?, description), \
            color       = COALESCE(?, color), \
            {flag_col}  = COALESCE(?, {flag_col}), \
            sort_order  = COALESCE(?, sort_order), \
            is_hidden   = COALESCE(?, is_hidden) \
         WHERE slug = ?",
        table = kind.table(),
        flag_col = kind.flag_column(),
    );
    conn.execute(
        &sql,
        params![
            args.name,
            args.description,
            args.color,
            flag_int,
            args.sort_order,
            hidden_int,
            slug
        ],
    )
    .with_context(|| format!("failed to update {} {slug}", kind.table()))?;
    get(conn, kind, slug)?.with_context(|| format!("{} {slug} missing after update", kind.table()))
}

/// Delete a status. Fails if any row in the owning table still references it.
pub fn remove(conn: &Connection, kind: Kind, slug: &str) -> Result<()> {
    if get(conn, kind, slug)?.is_none() {
        bail!("{} not found: {slug}", kind.table());
    }
    let owner = match kind {
        Kind::Project => "projects",
        Kind::Task => "tasks",
    };
    let in_use: i64 = conn
        .query_row(
            &format!("SELECT COUNT(*) FROM {owner} WHERE status = ?"),
            [slug],
            |row| row.get(0),
        )
        .with_context(|| format!("failed to count {owner} using status {slug}"))?;
    if in_use > 0 {
        bail!("cannot remove status '{slug}': still used by {in_use} {owner} row(s)");
    }
    conn.execute(
        &format!("DELETE FROM {table} WHERE slug = ?", table = kind.table()),
        [slug],
    )
    .with_context(|| format!("failed to delete {} {slug}", kind.table()))?;
    Ok(())
}

fn validate_color(c: &str) -> Result<()> {
    if color::parse_hex(c).is_none() {
        bail!("invalid color '{c}' — expected #RRGGBB");
    }
    Ok(())
}

/// Render statuses as an aligned text table with colorized slugs.
pub fn render_list(statuses: &[Status], kind: Kind) -> String {
    if statuses.is_empty() {
        return String::from("(no statuses)\n");
    }
    let slug_w = statuses
        .iter()
        .map(|s| s.slug.chars().count())
        .max()
        .unwrap_or(4)
        .max(4);
    let name_w = statuses
        .iter()
        .map(|s| s.name.chars().count())
        .max()
        .unwrap_or(4)
        .max(4);
    let flag_label = kind.flag_label();
    let flag_w = flag_label.len().max(5);
    let hidden_w = "hidden".len();

    let mut out = String::new();
    out.push_str(&format!(
        "{:<slug_w$}  {:<name_w$}  {:<flag_w$}  {:<hidden_w$}  color\n",
        "slug", "name", flag_label, "hidden",
    ));
    for s in statuses {
        let slug_cell = color::pad_colored(&s.slug, s.color.as_deref(), slug_w);
        out.push_str(&format!(
            "{slug_cell}  {:<name_w$}  {:<flag_w$}  {:<hidden_w$}  {}\n",
            s.name,
            if s.flag { "yes" } else { "no" },
            if s.is_hidden { "yes" } else { "no" },
            s.color.as_deref().unwrap_or(""),
        ));
    }
    out
}

/// Render a single status as a key/value block.
pub fn render_show(s: &Status, kind: Kind) -> String {
    let mut out = String::new();
    out.push_str(&format!("slug:        {}\n", s.slug));
    out.push_str(&format!("name:        {}\n", s.name));
    out.push_str(&format!(
        "{:<11}  {}\n",
        format!("{}:", kind.flag_label()),
        if s.flag { "yes" } else { "no" }
    ));
    out.push_str(&format!(
        "hidden:      {}\n",
        if s.is_hidden { "yes" } else { "no" }
    ));
    out.push_str(&format!("sort_order:  {}\n", s.sort_order));
    if let Some(c) = &s.color {
        out.push_str(&format!("color:       {c}\n"));
    }
    if let Some(d) = &s.description {
        out.push_str(&format!("description: {d}\n"));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db;
    use crate::workspace::root::ROOT_MARKER;
    use tempfile::TempDir;

    fn setup() -> (TempDir, Connection) {
        let tmp = TempDir::new().unwrap();
        std::fs::create_dir(tmp.path().join(ROOT_MARKER)).unwrap();
        let conn = db::open(tmp.path()).unwrap();
        (tmp, conn)
    }

    #[test]
    fn default_task_statuses_are_seeded() {
        let (_tmp, conn) = setup();
        let all = list(&conn, Kind::Task).unwrap();
        let slugs: Vec<&str> = all.iter().map(|s| s.slug.as_str()).collect();
        assert_eq!(
            slugs,
            vec!["backlog", "cycle", "in_progress", "parked", "done"]
        );
        let done = all.iter().find(|s| s.slug == "done").unwrap();
        assert!(done.flag, "done should have is_closed=true");
        let parked = all.iter().find(|s| s.slug == "parked").unwrap();
        assert!(parked.flag);
    }

    #[test]
    fn default_project_statuses_are_seeded() {
        let (_tmp, conn) = setup();
        let all = list(&conn, Kind::Project).unwrap();
        let slugs: Vec<&str> = all.iter().map(|s| s.slug.as_str()).collect();
        assert_eq!(slugs, vec!["active", "paused", "archived"]);
        let archived = all.iter().find(|s| s.slug == "archived").unwrap();
        assert!(archived.flag);
    }

    #[test]
    fn add_edit_remove_roundtrip() {
        let (_tmp, conn) = setup();
        add(
            &conn,
            Kind::Task,
            AddArgs {
                slug: "review",
                name: Some("In Review"),
                description: Some("Awaiting review."),
                color: Some("#ff00aa"),
                flag: false,
                sort_order: None,
                is_hidden: false,
            },
        )
        .unwrap();
        let got = get(&conn, Kind::Task, "review").unwrap().unwrap();
        assert_eq!(got.name, "In Review");
        assert_eq!(got.description.as_deref(), Some("Awaiting review."));
        assert_eq!(got.color.as_deref(), Some("#ff00aa"));

        edit(
            &conn,
            Kind::Task,
            "review",
            EditArgs {
                name: Some("Review"),
                description: Some("Awaiting code review."),
                flag: Some(true),
                ..Default::default()
            },
        )
        .unwrap();
        let got = get(&conn, Kind::Task, "review").unwrap().unwrap();
        assert_eq!(got.name, "Review");
        assert_eq!(got.description.as_deref(), Some("Awaiting code review."));
        assert!(got.flag);

        remove(&conn, Kind::Task, "review").unwrap();
        assert!(get(&conn, Kind::Task, "review").unwrap().is_none());
    }

    #[test]
    fn remove_blocks_if_in_use() {
        use crate::projects::{self, AddArgs as ProjAddArgs};
        let (_tmp, mut conn) = setup();
        projects::add(
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
        let p = projects::get_by_slug(&conn, "kdb").unwrap().unwrap();
        crate::tasks::add(
            &mut conn,
            crate::tasks::AddArgs {
                project_id: Some(p.id),
                space_id: None,
                title: "t",
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

        let err = remove(&conn, Kind::Task, "backlog").unwrap_err();
        assert!(err.to_string().contains("still used"));
    }

    #[test]
    fn add_rejects_bad_color() {
        let (_tmp, conn) = setup();
        let err = add(
            &conn,
            Kind::Task,
            AddArgs {
                slug: "x",
                name: None,
                description: None,
                color: Some("not-a-color"),
                flag: false,
                sort_order: None,
                is_hidden: false,
            },
        )
        .unwrap_err();
        assert!(err.to_string().contains("invalid color"));
    }
}
