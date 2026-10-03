//! Spaces table access and display.
//!
//! A **space** is a named grouping of related projects. It sits above
//! projects the way a project sits above tasks. Membership is one-to-many
//! (a project belongs to at most one space via a nullable `space_id`);
//! spaces are flat (no nesting) and organizational only — they do not
//! affect task ids (still `HRM-0120`, off the project alias) and impose
//! no filesystem structure (`path` is an optional reference, not enforced).
//!
//! A space has a unique `slug`, a display `name`, an optional `path`, and
//! a lifecycle status drawn from the shared `project_statuses` lookup.

use anyhow::{Context, Result, bail};
use rusqlite::{Connection, OptionalExtension, params};
use serde::Serialize;

// ----------------------------------------------
// projects/kdb/src/spaces.rs
//
// pub struct Space                           L40
//   fn from_row()                            L57
// const SELECT_COLS                          L73
// pub fn list()                              L80
// pub fn get_by_slug()                       L97
// pub struct AddArgs                        L105
// pub fn add()                              L118
// pub struct EditArgs                       L131
//   fn is_empty()                           L140
// pub fn edit()                             L150
// pub fn render_list()                      L181
// pub fn render_show()                      L230
// mod tests                                 L253
// fn setup()                                L260
// fn add_then_list_and_show()               L268
// fn project_count_reflects_membership()    L290
// fn detach_sets_space_null()               L326
// ----------------------------------------------

#[derive(Debug, Clone, Serialize)]
pub struct Space {
    pub id: i64,
    pub slug: String,
    pub name: String,
    /// Uppercase alias used in task ids for space-native tasks (e.g. `ICE`).
    /// `None` until the space owns tasks. Unique across projects and spaces.
    pub alias: Option<String>,
    pub path: Option<String>,
    pub status: String,
    pub description: Option<String>,
    /// Number of projects that belong to this space (joined count).
    pub project_count: i64,
    pub created_at: String,
    pub updated_at: String,
}

impl Space {
    fn from_row(row: &rusqlite::Row) -> rusqlite::Result<Self> {
        Ok(Self {
            id: row.get(0)?,
            slug: row.get(1)?,
            name: row.get(2)?,
            alias: row.get(3)?,
            path: row.get(4)?,
            status: row.get(5)?,
            description: row.get(6)?,
            project_count: row.get(7)?,
            created_at: row.get(8)?,
            updated_at: row.get(9)?,
        })
    }
}

const SELECT_COLS: &str = "s.id, s.slug, s.name, s.alias, s.path, s.status, s.description, \
     (SELECT COUNT(*) FROM projects p WHERE p.space_id = s.id), \
     s.created_at, s.updated_at";

/// List spaces, ordered by slug. Spaces whose status is marked
/// `is_archived` in `project_statuses` are excluded unless
/// `include_archived` is set.
pub fn list(conn: &Connection, include_archived: bool) -> Result<Vec<Space>> {
    let sql = if include_archived {
        format!("SELECT {SELECT_COLS} FROM spaces s ORDER BY s.slug")
    } else {
        format!(
            "SELECT {SELECT_COLS} FROM spaces s \
             JOIN project_statuses ps ON ps.slug = s.status \
             WHERE ps.is_archived = 0 ORDER BY s.slug"
        )
    };
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map([], Space::from_row)?;
    rows.collect::<rusqlite::Result<Vec<_>>>()
        .context("failed to read spaces")
}

/// Fetch a space by slug. Returns `None` if no match.
pub fn get_by_slug(conn: &Connection, slug: &str) -> Result<Option<Space>> {
    let sql = format!("SELECT {SELECT_COLS} FROM spaces s WHERE s.slug = ?");
    let mut stmt = conn.prepare(&sql)?;
    stmt.query_row([slug], Space::from_row)
        .optional()
        .context("failed to query space")
}

pub struct AddArgs<'a> {
    pub slug: &'a str,
    pub name: Option<&'a str>,
    /// Uppercase task-id alias. Required — a space carries an alias from
    /// creation so it can own tasks without a later assignment step.
    pub alias: &'a str,
    pub path: Option<&'a str>,
    pub description: Option<&'a str>,
}

/// Insert a new space. Fails if the slug or alias is already taken. The
/// alias is uppercased and must not collide with any project or space
/// alias — the DB CHECK and cross-container triggers enforce this.
pub fn add(conn: &Connection, args: AddArgs) -> Result<Space> {
    let name = args.name.unwrap_or(args.slug);
    let alias = args.alias.to_ascii_uppercase();
    conn.execute(
        "INSERT INTO spaces (slug, name, alias, path, description) VALUES (?, ?, ?, ?, ?)",
        params![args.slug, name, alias, args.path, args.description],
    )
    .with_context(|| format!("failed to insert space {}", args.slug))?;
    get_by_slug(conn, args.slug)?
        .with_context(|| format!("space {} missing after insert", args.slug))
}

#[derive(Default)]
pub struct EditArgs<'a> {
    pub name: Option<&'a str>,
    pub alias: Option<&'a str>,
    pub path: Option<&'a str>,
    pub status: Option<&'a str>,
    pub description: Option<&'a str>,
}

impl EditArgs<'_> {
    fn is_empty(&self) -> bool {
        self.name.is_none()
            && self.alias.is_none()
            && self.path.is_none()
            && self.status.is_none()
            && self.description.is_none()
    }
}

/// Update mutable fields on a space. `None` fields are left unchanged.
pub fn edit(conn: &Connection, slug: &str, args: EditArgs) -> Result<Space> {
    if args.is_empty() {
        bail!("no fields to update");
    }
    if get_by_slug(conn, slug)?.is_none() {
        bail!("space not found: {slug}");
    }
    let alias_upper = args.alias.map(str::to_ascii_uppercase);
    conn.execute(
        "UPDATE spaces SET \
            name        = COALESCE(?, name), \
            alias       = COALESCE(?, alias), \
            path        = COALESCE(?, path), \
            status      = COALESCE(?, status), \
            description = COALESCE(?, description), \
            updated_at  = strftime('%Y-%m-%dT%H:%M:%fZ','now') \
         WHERE slug = ?",
        params![
            args.name,
            alias_upper,
            args.path,
            args.status,
            args.description,
            slug
        ],
    )
    .with_context(|| format!("failed to update space {slug}"))?;
    get_by_slug(conn, slug)?.with_context(|| format!("space {slug} missing after update"))
}

/// Render a list of spaces as an aligned text table.
pub fn render_list(spaces: &[Space]) -> String {
    if spaces.is_empty() {
        return String::from("(no spaces)\n");
    }
    let slug_w = spaces
        .iter()
        .map(|s| s.slug.len())
        .max()
        .unwrap_or(4)
        .max(4);
    let name_w = spaces
        .iter()
        .map(|s| s.name.len())
        .max()
        .unwrap_or(4)
        .max(4);
    let alias_w = spaces
        .iter()
        .map(|s| s.alias.as_deref().unwrap_or("-").len())
        .max()
        .unwrap_or(5)
        .max(5);
    let status_w = spaces
        .iter()
        .map(|s| s.status.len())
        .max()
        .unwrap_or(6)
        .max(6);

    let mut out = String::new();
    out.push_str(&format!(
        "{:<slug_w$}  {:<alias_w$}  {:<name_w$}  {:<status_w$}  {:>8}  path\n",
        "slug", "alias", "name", "status", "projects",
    ));
    for s in spaces {
        out.push_str(&format!(
            "{:<slug_w$}  {:<alias_w$}  {:<name_w$}  {:<status_w$}  {:>8}  {}\n",
            s.slug,
            s.alias.as_deref().unwrap_or("-"),
            s.name,
            s.status,
            s.project_count,
            s.path.as_deref().unwrap_or("-"),
        ));
    }
    out
}

/// Render a single space as a human-readable block.
pub fn render_show(s: &Space) -> String {
    let mut out = String::new();
    out.push_str(&format!("slug:        {}\n", s.slug));
    out.push_str(&format!("name:        {}\n", s.name));
    out.push_str(&format!(
        "alias:       {}\n",
        s.alias.as_deref().unwrap_or("-")
    ));
    out.push_str(&format!(
        "path:        {}\n",
        s.path.as_deref().unwrap_or("-")
    ));
    out.push_str(&format!("status:      {}\n", s.status));
    out.push_str(&format!("projects:    {}\n", s.project_count));
    if let Some(desc) = &s.description {
        out.push_str(&format!("description: {desc}\n"));
    }
    out.push_str(&format!("created_at:  {}\n", s.created_at));
    out.push_str(&format!("updated_at:  {}\n", s.updated_at));
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db;
    use crate::projects;
    use crate::workspace::root::ROOT_MARKER;
    use tempfile::TempDir;

    fn setup() -> (TempDir, Connection) {
        let tmp = TempDir::new().unwrap();
        std::fs::create_dir(tmp.path().join(ROOT_MARKER)).unwrap();
        let conn = db::open(tmp.path()).unwrap();
        (tmp, conn)
    }

    #[test]
    fn add_then_list_and_show() {
        let (_tmp, conn) = setup();
        let s = add(
            &conn,
            AddArgs {
                slug: "iceberg",
                name: Some("Iceberg"),
                alias: "ICE",
                path: Some("projects/iceberg"),
                description: Some("Client engagements"),
            },
        )
        .unwrap();
        assert_eq!(s.slug, "iceberg");
        assert_eq!(s.alias.as_deref(), Some("ICE"));
        assert_eq!(s.project_count, 0);

        let listed = list(&conn, false).unwrap();
        assert_eq!(listed.len(), 1);
    }

    #[test]
    fn project_count_reflects_membership() {
        let (_tmp, conn) = setup();
        let space = add(
            &conn,
            AddArgs {
                slug: "iceberg",
                name: None,
                alias: "ICE",
                path: None,
                description: None,
            },
        )
        .unwrap();
        projects::add(
            &conn,
            projects::AddArgs {
                slug: "adrata",
                alias: "ADR",
                name: None,
                path: "projects/labs/clients/adrata",
                description: None,
                space_id: Some(space.id),
            },
        )
        .unwrap();

        let got = get_by_slug(&conn, "iceberg").unwrap().unwrap();
        assert_eq!(got.project_count, 1);

        let members = projects::list(&conn, true, Some("iceberg")).unwrap();
        assert_eq!(members.len(), 1);
        assert_eq!(members[0].slug, "adrata");
        assert_eq!(members[0].space_slug.as_deref(), Some("iceberg"));
    }

    #[test]
    fn detach_sets_space_null() {
        let (_tmp, conn) = setup();
        let space = add(
            &conn,
            AddArgs {
                slug: "iceberg",
                name: None,
                alias: "ICE",
                path: None,
                description: None,
            },
        )
        .unwrap();
        projects::add(
            &conn,
            projects::AddArgs {
                slug: "adrata",
                alias: "ADR",
                name: None,
                path: "projects/labs/clients/adrata",
                description: None,
                space_id: Some(space.id),
            },
        )
        .unwrap();
        // Detach: Some(None) clears membership.
        let p = projects::edit(
            &conn,
            "adrata",
            projects::EditArgs {
                space_id: Some(None),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(p.space_id, None);
        assert_eq!(
            get_by_slug(&conn, "iceberg")
                .unwrap()
                .unwrap()
                .project_count,
            0
        );
    }
}
