//! SQLite access for the planning layer.
//!
//! One file, `.kdb/index.db`, shared by every process on the machine. WAL mode
//! plus a busy timeout handle parallel agents writing at once; callers that
//! assign sequence numbers must do so inside the insert transaction and retry
//! once on a unique-constraint conflict.
//!
//! Versioning uses `PRAGMA user_version`. v1 left live databases at 8. A fresh
//! database gets the baseline (0008) and then every later migration; a v1
//! database at 8 gets only the later ones. Anything below 8 must be upgraded
//! with v1 first.

use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, Result, bail};
use rusqlite::Connection;

use crate::workspace::ROOT_MARKER;

pub const DB_FILE: &str = "index.db";
pub const BUSY_TIMEOUT: Duration = Duration::from_secs(5);

/// `(user_version after applying, sql)`. The first entry is the baseline.
const MIGRATIONS: &[(i64, &str)] = &[
    (8, include_str!("migrations/0008_baseline.sql")),
    (9, include_str!("migrations/0009_v2.sql")),
];

pub fn db_path(root: &Path) -> PathBuf {
    root.join(ROOT_MARKER).join(DB_FILE)
}

/// Open (or create) the workspace database and bring it to the current schema.
pub fn open(root: &Path) -> Result<Connection> {
    open_path(&db_path(root))
}

/// Open a database at an explicit path. Tests use this.
pub fn open_path(path: &Path) -> Result<Connection> {
    let conn =
        Connection::open(path).with_context(|| format!("failed to open {}", path.display()))?;
    conn.busy_timeout(BUSY_TIMEOUT)?;
    conn.pragma_update(None, "journal_mode", "WAL")?;
    conn.pragma_update(None, "foreign_keys", "ON")?;
    migrate(&conn)?;
    Ok(conn)
}

pub fn user_version(conn: &Connection) -> Result<i64> {
    conn.query_row("PRAGMA user_version", [], |r| r.get(0))
        .context("failed to read user_version")
}

fn migrate(conn: &Connection) -> Result<()> {
    let current = user_version(conn)?;
    let baseline = MIGRATIONS[0].0;
    if current > 0 && current < baseline {
        bail!("database schema is at version {current}; upgrade it with kdb v1 first");
    }
    for (version, sql) in MIGRATIONS {
        if *version <= current {
            continue;
        }
        // The baseline only applies to an empty database.
        if *version == baseline && current != 0 {
            continue;
        }
        let tx = conn.unchecked_transaction()?;
        tx.execute_batch(sql)
            .with_context(|| format!("migration to version {version} failed"))?;
        tx.pragma_update(None, "user_version", *version)?;
        tx.commit()?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn fresh_db_gets_full_schema() {
        let tmp = TempDir::new().unwrap();
        let conn = open_path(&tmp.path().join("t.db")).unwrap();
        assert_eq!(user_version(&conn).unwrap(), 9);
        let n: i64 = conn
            .query_row("SELECT count(*) FROM task_statuses WHERE icon IS NOT NULL", [], |r| r.get(0))
            .unwrap();
        assert_eq!(n, 5);
        let parked_closed: i64 = conn
            .query_row("SELECT is_closed FROM task_statuses WHERE slug='parked'", [], |r| r.get(0))
            .unwrap();
        assert_eq!(parked_closed, 0);
        conn.execute_batch("INSERT INTO task_deps(task_id, depends_on) VALUES (1, 1)")
            .expect_err("self-dependency must be rejected");
        drop(conn);
        let conn = open_path(&tmp.path().join("t.db")).unwrap();
        assert_eq!(user_version(&conn).unwrap(), 9);
    }

    #[test]
    fn v1_db_at_8_skips_baseline() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("v1.db");
        {
            let conn = Connection::open(&path).unwrap();
            conn.execute_batch(include_str!("migrations/0008_baseline.sql")).unwrap();
            conn.execute_batch("CREATE TABLE search_fts(x); CREATE TABLE search_meta(x); CREATE TABLE collections(x);").unwrap();
            conn.pragma_update(None, "user_version", 8).unwrap();
        }
        let conn = open_path(&path).unwrap();
        assert_eq!(user_version(&conn).unwrap(), 9);
        let n: i64 = conn
            .query_row("SELECT count(*) FROM sqlite_master WHERE name LIKE 'search%' OR name = 'collections'", [], |r| r.get(0))
            .unwrap();
        assert_eq!(n, 0);
    }

    #[test]
    fn old_v1_db_is_refused() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("old.db");
        {
            let conn = Connection::open(&path).unwrap();
            conn.pragma_update(None, "user_version", 5).unwrap();
        }
        assert!(open_path(&path).is_err());
    }
}
