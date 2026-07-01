//! SQLite-backed relational layer (projects, cycles, tasks, labels).
//!
//! The database lives at `.kdb/index.db` at the workspace root. Migrations are
//! embedded in the binary and applied in order on [`open`]. Schema version is
//! tracked via SQLite's `user_version` pragma.

use anyhow::{Context, Result};
use rusqlite::Connection;
use std::path::{Path, PathBuf};

use crate::workspace::root::ROOT_MARKER;

// -------------------------------------------------
// projects/kdb/src/db/mod.rs
//
// pub const DB_FILE                             L26
// const MIGRATIONS                              L29
// pub fn db_path()                              L55
// pub fn open()                                 L60
// fn migrate()                                  L73
// mod tests                                     L92
// fn open_creates_schema_and_is_idempotent()    L97
// -------------------------------------------------

/// Database filename inside `.kdb/`.
pub const DB_FILE: &str = "index.db";

/// Ordered list of SQL migrations. Append new entries; never rewrite.
const MIGRATIONS: &[(&str, &str)] = &[
    (
        "0001_relational",
        include_str!("migrations/0001_relational.sql"),
    ),
    (
        "0002_tasks_order",
        include_str!("migrations/0002_tasks_order.sql"),
    ),
    (
        "0003_customizable_statuses",
        include_str!("migrations/0003_customizable_statuses.sql"),
    ),
    (
        "0004_status_hidden",
        include_str!("migrations/0004_status_hidden.sql"),
    ),
    ("0005_search", include_str!("migrations/0005_search.sql")),
    (
        "0006_search_kind",
        include_str!("migrations/0006_search_kind.sql"),
    ),
    ("0007_spaces", include_str!("migrations/0007_spaces.sql")),
];

/// Return the canonical db path for a workspace root.
pub fn db_path(root: &Path) -> PathBuf {
    root.join(ROOT_MARKER).join(DB_FILE)
}

/// Open (or create) the workspace's SQLite database and apply pending migrations.
pub fn open(root: &Path) -> Result<Connection> {
    let path = db_path(root);
    let conn =
        Connection::open(&path).with_context(|| format!("failed to open {}", path.display()))?;
    conn.pragma_update(None, "foreign_keys", "ON")
        .context("failed to enable foreign_keys")?;
    conn.pragma_update(None, "journal_mode", "WAL")
        .context("failed to set journal_mode=WAL")?;
    migrate(&conn)?;
    Ok(conn)
}

/// Apply any migrations whose index is greater than `user_version`.
fn migrate(conn: &Connection) -> Result<()> {
    let current: i64 = conn
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .context("failed to read user_version")?;

    for (idx, (name, sql)) in MIGRATIONS.iter().enumerate() {
        let version = (idx + 1) as i64;
        if version <= current {
            continue;
        }
        conn.execute_batch(sql)
            .with_context(|| format!("migration {name} failed"))?;
        conn.pragma_update(None, "user_version", version)
            .with_context(|| format!("failed to bump user_version for {name}"))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn open_creates_schema_and_is_idempotent() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        std::fs::create_dir(root.join(ROOT_MARKER)).unwrap();

        let conn = open(root).unwrap();
        let version: i64 = conn
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .unwrap();
        assert_eq!(version, MIGRATIONS.len() as i64);

        let top_n: String = conn
            .query_row("SELECT value FROM meta WHERE key = 'top_n'", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(top_n, "10");

        drop(conn);
        let conn2 = open(root).unwrap();
        let version2: i64 = conn2
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .unwrap();
        assert_eq!(version2, MIGRATIONS.len() as i64);
    }
}
