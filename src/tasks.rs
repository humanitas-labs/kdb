//! Tasks table access and display.
//!
//! Top-level tasks have a per-project `seq` counter and render as
//! `{PROJECT_ALIAS}-{seq:04d}` (e.g. `KDB-0030`). Subtasks (rows with
//! `parent_id IS NOT NULL`) do not consume `seq`; they have a
//! sibling-local `child_seq` and render with a dotted suffix walking
//! the parent chain (`KDB-0030.1`, `KDB-0030.1.2`). Statuses are user-
//! customizable via the `task_statuses` table; the default seeded set
//! is `backlog` | `cycle` | `in_progress` | `parked` | `done`.

use anyhow::{Context, Result, bail};
use rusqlite::{Connection, OptionalExtension, params};
use serde::Serialize;

// -------------------------------------------------------------
// projects/kdb/src/tasks.rs
//
// pub const DEFAULT_STATUSES                                L96
// pub fn is_closed_status()                                L100
// pub const SEQ_WIDTH                                      L112
// pub const ORDER_KEY_WIDTH                                L115
// pub fn format_external_id()                              L120
// pub fn dotted_top()                                      L125
// pub fn default_order_key()                               L130
// pub const CHAIN_CTE                                      L138
// const ALPHABET                                           L147
// fn rank()                                                L149
// pub fn between()                                         L160
// fn key_before()                                          L172
// fn key_after()                                           L185
// fn key_between_two()                                     L200
// pub struct Task                                          L239
// pub struct TaskView                                      L266
//   pub fn external_id()                                   L281
// pub struct TaskId                                        L292
//   pub fn parse()                                         L300
//   pub fn render()                                        L336
// const SELECT_COLS                                        L346
// const SELECT_JOINS                                       L352
// fn view_from_row()                                       L357
// pub struct ListFilters                                   L386
// pub fn list()                                            L401
// pub fn get()                                             L453
// pub fn get_by_row_id()                                   L485
// pub struct ChildTask                                     L497
// pub fn descendants()                                     L509
// pub fn children()                                        L534
// pub struct AddArgs                                       L564
// pub fn add()                                             L584
// pub struct EditArgs                                      L663
//   fn is_empty()                                          L672
// pub fn edit()                                            L692
// pub enum Side                                            L781
// pub enum MoveTarget                                      L788
// pub fn order_key_adjacent()                              L797
// pub fn move_task()                                       L825
// fn ensure_same_context()                                 L888
// enum Direction                                           L907
// fn neighbor_order()                                      L916
// pub fn set_status()                                      L957
// fn subtree_ids()                                         L978
// pub fn soft_delete()                                     L994
// pub fn restore()                                        L1013
// pub fn hard_delete()                                    L1050
// pub fn get_including_deleted()                          L1065
// pub struct PurgeFilters                                 L1100
// pub fn purge()                                          L1110
// fn status_glyph()                                       L1161
// pub fn render_list()                                    L1173
// pub fn render_show()                                    L1196
// mod tests                                               L1239
// fn setup()                                              L1246
// fn parse_task_id_uppercases_alias()                     L1266
// fn external_id_is_zero_padded()                         L1276
// fn parse_dotted_task_id_round_trips()                   L1283
// fn add_auto_seq_then_explicit_seq()                     L1302
// fn child_does_not_consume_top_level_seq()               L1359
// fn unparent_allocates_top_level_seq()                   L1375
// fn reparent_top_level_to_child_clears_seq()             L1398
// fn reparent_between_parents_renumbers_child_seq()       L1421
// fn grandchild_dotted_id()                               L1445
// fn add_with_status()                                    L1456
// fn set_status_transitions()                             L1478
// fn children_are_ordered_by_order_key()                  L1507
// fn render_show_includes_children_section()              L1575
// fn add_task()                                           L1615
// fn between_sits_strictly_between_various_pairs()        L1634
// fn move_task_before_and_after_reorders()                L1665
// fn move_task_top_and_bottom()                           L1717
// fn move_task_rejects_different_parent()                 L1758
// fn order_key_adjacent_after_sits_between_neighbors()    L1775
// -------------------------------------------------------------

/// Default seeded statuses. Users can add, rename, or remove these via
/// `kdb statuses` — treat this list as a fallback for display only.
pub const DEFAULT_STATUSES: &[&str] = &["backlog", "cycle", "in_progress", "parked", "done"];

/// Look up whether a status slug is marked `is_closed` in `task_statuses`.
/// Returns an error if the slug doesn't exist.
pub fn is_closed_status(conn: &Connection, slug: &str) -> Result<bool> {
    let flag: i64 = conn
        .query_row(
            "SELECT is_closed FROM task_statuses WHERE slug = ?",
            [slug],
            |row| row.get(0),
        )
        .with_context(|| format!("unknown task status '{slug}'"))?;
    Ok(flag != 0)
}

/// Zero-padding width for the `seq` portion of external ids.
pub const SEQ_WIDTH: usize = 4;

/// Zero-padding width for lexicographically sortable task order keys.
pub const ORDER_KEY_WIDTH: usize = 12;

/// Format an external task id: `{ALIAS}-{dotted}`. `dotted` is the
/// parent-chain rendering — `"0030"` for top-level, `"0030.1.2"` for
/// nested children. Computed by the SQL recursive CTE in [`CHAIN_CTE`].
pub fn format_external_id(alias: &str, dotted: &str) -> String {
    format!("{alias}-{dotted}")
}

/// Render a top-level seq as the dotted suffix (`"0030"`).
pub fn dotted_top(seq: i64) -> String {
    format!("{seq:0width$}", width = SEQ_WIDTH)
}

/// Default order key for newly created tasks (used when no explicit position is given).
pub fn default_order_key(seq: i64) -> String {
    format!("{seq:0width$}", width = ORDER_KEY_WIDTH)
}

/// Recursive CTE that, for every task, computes `(root_seq, suffix)`
/// — the top-level ancestor's `seq` and the dotted suffix descending
/// to the row (`""` for top-level, `".1.2"` for a grandchild, etc.).
/// Prepend to any SELECT against `tasks`.
pub const CHAIN_CTE: &str = "WITH RECURSIVE crumbs(id, root_seq, suffix) AS (\
    SELECT id, seq, '' FROM tasks WHERE parent_id IS NULL \
    UNION ALL \
    SELECT t.id, c.root_seq, c.suffix || '.' || t.child_seq \
      FROM tasks t JOIN crumbs c ON c.id = t.parent_id) ";

/// Alphabet for lexicographic order keys. Byte-order sorted (`'0'..'9'` < `'a'..'z'`).
/// The migration backfill (`printf('%012d', seq)`) is a strict subset, so generated
/// keys interleave with legacy rows under plain string comparison.
const ALPHABET: &[u8; 36] = b"0123456789abcdefghijklmnopqrstuvwxyz";

fn rank(b: u8) -> usize {
    match b {
        b'0'..=b'9' => (b - b'0') as usize,
        b'a'..=b'z' => 10 + (b - b'a') as usize,
        _ => unreachable!("order keys use alphabet 0-9a-z (got byte {b:#x})"),
    }
}

/// Generate an order key that sorts strictly between `prev` and `next` under byte
/// comparison. `None` means unbounded on that side. Panics if inputs are inconsistent
/// (`prev >= next`) or if the requested position is below an all-`0` key.
pub fn between(prev: Option<&str>, next: Option<&str>) -> String {
    match (prev, next) {
        (None, None) => String::from(ALPHABET[ALPHABET.len() / 2] as char),
        (None, Some(n)) => key_before(n),
        (Some(p), None) => key_after(p),
        (Some(p), Some(n)) => {
            debug_assert!(p < n, "between: prev must be < next ({p:?} vs {n:?})");
            key_between_two(p, n)
        }
    }
}

fn key_before(next: &str) -> String {
    let mut result = Vec::new();
    for &byte in next.as_bytes() {
        let r = rank(byte);
        if r > 0 {
            result.push(ALPHABET[r / 2]);
            return String::from_utf8(result).unwrap();
        }
        result.push(byte);
    }
    panic!("cannot produce key before all-min key: {next:?}");
}

fn key_after(prev: &str) -> String {
    let mut result = Vec::new();
    for &byte in prev.as_bytes() {
        let r = rank(byte);
        if r + 1 < ALPHABET.len() {
            let high = ALPHABET.len() - 1;
            result.push(ALPHABET[(r + high + 1) / 2]);
            return String::from_utf8(result).unwrap();
        }
        result.push(byte);
    }
    result.push(ALPHABET[ALPHABET.len() / 2]);
    String::from_utf8(result).unwrap()
}

fn key_between_two(prev: &str, next: &str) -> String {
    let a = prev.as_bytes();
    let b = next.as_bytes();
    let mut result: Vec<u8> = Vec::new();
    let mut i = 0;
    while i < a.len() && i < b.len() && a[i] == b[i] {
        result.push(a[i]);
        i += 1;
    }
    let ar = a.get(i).copied().map(rank);
    let br = b.get(i).copied().map(rank);
    match (ar, br) {
        (Some(ar), Some(br)) => {
            debug_assert!(ar < br);
            if br - ar >= 2 {
                result.push(ALPHABET[(ar + br) / 2]);
                return String::from_utf8(result).unwrap();
            }
            result.push(ALPHABET[ar]);
            let rest = key_after(std::str::from_utf8(&a[i + 1..]).unwrap());
            result.extend_from_slice(rest.as_bytes());
            String::from_utf8(result).unwrap()
        }
        (None, Some(br)) => {
            if br > 0 {
                result.push(ALPHABET[br / 2]);
                return String::from_utf8(result).unwrap();
            }
            result.push(ALPHABET[0]);
            let rest = key_before(std::str::from_utf8(&b[i + 1..]).unwrap());
            result.extend_from_slice(rest.as_bytes());
            String::from_utf8(result).unwrap()
        }
        (Some(_), None) => unreachable!("key_between_two: next exhausted while prev < next"),
        (None, None) => unreachable!("key_between_two: prev == next"),
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct Task {
    pub id: i64,
    pub project_id: i64,
    /// Top-level `seq` within the project. `None` for children
    /// (subtasks); they use [`Task::child_seq`] instead.
    pub seq: Option<i64>,
    /// 1-based position among siblings sharing the same `parent_id`.
    /// `None` for top-level tasks.
    pub child_seq: Option<i64>,
    pub title: String,
    pub body: Option<String>,
    pub status: String,
    pub priority: i64,
    pub order: String,
    pub cycle_id: Option<i64>,
    pub parent_id: Option<i64>,
    pub created_at: String,
    pub updated_at: String,
    pub closed_at: Option<String>,
    /// Soft-delete timestamp. `None` for live rows; ISO-8601 UTC for
    /// rows hidden by `tasks delete`. Restored rows clear back to `None`.
    pub deleted_at: Option<String>,
}

/// Task row joined with its project slug/alias, optional cycle key,
/// and the dotted id suffix computed from the parent chain.
#[derive(Debug, Clone, Serialize)]
pub struct TaskView {
    #[serde(flatten)]
    pub task: Task,
    pub project_slug: String,
    pub project_alias: String,
    pub cycle_key: Option<String>,
    /// Dotted id suffix — `"0030"` for top-level, `"0030.1.2"` for a
    /// grandchild. Combined with `project_alias` to form the
    /// external id.
    pub dotted: String,
}

impl TaskView {
    /// External task id: `{PROJECT_ALIAS}-{dotted}` (e.g. `KDB-0030`,
    /// `KDB-0030.1.2`).
    pub fn external_id(&self) -> String {
        format_external_id(&self.project_alias, &self.dotted)
    }
}

/// Parsed external task id. `alias` is always uppercased. For
/// top-level tasks `child_path` is empty; for nested tasks it holds
/// the chain of `child_seq` values from the top-level ancestor's
/// first child onward (e.g. `KDB-0030.1.2` → `seq=30`,
/// `child_path=[1, 2]`).
#[derive(Debug, Clone)]
pub struct TaskId {
    pub alias: String,
    pub seq: i64,
    pub child_path: Vec<i64>,
}

impl TaskId {
    /// Parse `ALIAS-seq[.child[.child]...]`. Alias is uppercased.
    pub fn parse(s: &str) -> Result<Self> {
        let idx = s
            .rfind('-')
            .with_context(|| format!("invalid task id '{s}': expected ALIAS-seq"))?;
        let (prefix, rest) = s.split_at(idx);
        if prefix.is_empty() {
            bail!("invalid task id '{s}': empty alias");
        }
        let mut parts = rest[1..].split('.');
        let seq_part = parts
            .next()
            .with_context(|| format!("invalid task id '{s}': missing seq"))?;
        let seq: i64 = seq_part
            .parse()
            .with_context(|| format!("invalid task id '{s}': seq not an integer"))?;
        let mut child_path = Vec::new();
        for p in parts {
            if p.is_empty() {
                bail!("invalid task id '{s}': empty child component");
            }
            let n: i64 = p
                .parse()
                .with_context(|| format!("invalid task id '{s}': child '{p}' not an integer"))?;
            if n < 1 {
                bail!("invalid task id '{s}': child component must be >= 1");
            }
            child_path.push(n);
        }
        Ok(Self {
            alias: prefix.to_ascii_uppercase(),
            seq,
            child_path,
        })
    }

    /// Render back to the canonical external id form.
    pub fn render(&self) -> String {
        let mut s = format!("{}-{:0width$}", self.alias, self.seq, width = SEQ_WIDTH);
        for n in &self.child_path {
            s.push('.');
            s.push_str(&n.to_string());
        }
        s
    }
}

const SELECT_COLS: &str = "t.id, t.project_id, t.seq, t.child_seq, t.title, t.body, \
    t.status, t.priority, \
    COALESCE(t.\"order\", printf('%012d', COALESCE(t.seq, t.child_seq))), \
    t.cycle_id, t.parent_id, t.created_at, t.updated_at, t.closed_at, t.deleted_at, \
    p.slug, p.alias, c.key, printf('%04d', cr.root_seq) || cr.suffix";

const SELECT_JOINS: &str = "FROM tasks t \
     JOIN projects p ON p.id = t.project_id \
     LEFT JOIN cycles c ON c.id = t.cycle_id \
     JOIN crumbs cr ON cr.id = t.id";

fn view_from_row(row: &rusqlite::Row) -> rusqlite::Result<TaskView> {
    let task = Task {
        id: row.get(0)?,
        project_id: row.get(1)?,
        seq: row.get(2)?,
        child_seq: row.get(3)?,
        title: row.get(4)?,
        body: row.get(5)?,
        status: row.get(6)?,
        priority: row.get(7)?,
        order: row.get(8)?,
        cycle_id: row.get(9)?,
        parent_id: row.get(10)?,
        created_at: row.get(11)?,
        updated_at: row.get(12)?,
        closed_at: row.get(13)?,
        deleted_at: row.get(14)?,
    };
    Ok(TaskView {
        task,
        project_slug: row.get(15)?,
        project_alias: row.get(16)?,
        cycle_key: row.get(17)?,
        dotted: row.get(18)?,
    })
}

/// Filters for [`list`].
#[derive(Debug, Default)]
pub struct ListFilters<'a> {
    pub statuses: Option<&'a [&'a str]>,
    pub project_slug: Option<&'a str>,
    /// Restrict to tasks whose project belongs to this space (by slug).
    pub space_slug: Option<&'a str>,
    pub cycle_key: Option<&'a str>,
    pub priority: Option<i64>,
    pub limit: Option<i64>,
    /// When `true`, exclude rows with `parent_id IS NOT NULL`.
    /// Subtasks live inside their parent task's view; default lists
    /// don't surface them.
    pub top_level_only: bool,
}

/// List tasks matching the filters, ordered by priority then updated_at desc.
pub fn list(conn: &Connection, f: ListFilters) -> Result<Vec<TaskView>> {
    let mut sql = format!(
        "{CHAIN_CTE} SELECT {SELECT_COLS} {SELECT_JOINS} WHERE t.deleted_at IS NULL"
    );
    let mut args: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();

    if let Some(statuses) = f.statuses {
        if statuses.is_empty() {
            return Ok(Vec::new());
        }
        let placeholders = vec!["?"; statuses.len()].join(",");
        sql.push_str(&format!(" AND t.status IN ({placeholders})"));
        for s in statuses {
            args.push(Box::new(s.to_string()));
        }
    }
    if let Some(slug) = f.project_slug {
        sql.push_str(" AND p.slug = ?");
        args.push(Box::new(slug.to_string()));
    }
    if let Some(space) = f.space_slug {
        sql.push_str(" AND p.space_id = (SELECT id FROM spaces WHERE slug = ?)");
        args.push(Box::new(space.to_string()));
    }
    if let Some(key) = f.cycle_key {
        sql.push_str(" AND c.key = ?");
        args.push(Box::new(key.to_string()));
    }
    if let Some(pri) = f.priority {
        sql.push_str(" AND t.priority = ?");
        args.push(Box::new(pri));
    }
    if f.top_level_only {
        sql.push_str(" AND t.parent_id IS NULL");
    }

    // Group by project first so cross-project listings (a `--space` rollup or
    // `-P all`) keep each project's tasks together instead of interleaving.
    // For a single-project list every row shares `p.slug`, so this is a no-op
    // and the within-project order key still drives the sort.
    sql.push_str(" ORDER BY p.slug ASC, COALESCE(t.\"order\", printf('%012d', COALESCE(t.seq, t.child_seq))) ASC, t.seq ASC");

    if let Some(limit) = f.limit {
        sql.push_str(" LIMIT ?");
        args.push(Box::new(limit));
    }

    let mut stmt = conn.prepare(&sql)?;
    let params_iter = rusqlite::params_from_iter(args.iter().map(|b| b.as_ref()));
    let rows = stmt.query_map(params_iter, view_from_row)?;
    rows.collect::<rusqlite::Result<Vec<_>>>()
        .context("failed to read tasks")
}

/// Resolve a parsed external id to a joined task row. Walks the
/// parent chain for nested ids (`KDB-0030.1.2`).
pub fn get(conn: &Connection, id: &TaskId) -> Result<Option<TaskView>> {
    let mut row_id: i64 = match conn
        .query_row(
            "SELECT t.id FROM tasks t JOIN projects p ON p.id = t.project_id \
             WHERE p.alias = ? AND t.seq = ?",
            params![id.alias, id.seq],
            |row| row.get(0),
        )
        .optional()
        .context("failed to look up top-level task")?
    {
        Some(rid) => rid,
        None => return Ok(None),
    };
    for n in &id.child_path {
        row_id = match conn
            .query_row(
                "SELECT id FROM tasks WHERE parent_id = ? AND child_seq = ?",
                params![row_id, n],
                |row| row.get::<_, i64>(0),
            )
            .optional()
            .context("failed to look up child task")?
        {
            Some(rid) => rid,
            None => return Ok(None),
        };
    }
    get_by_row_id(conn, row_id)
}

/// Look up a task by its primary key with full join + dotted id.
pub fn get_by_row_id(conn: &Connection, row_id: i64) -> Result<Option<TaskView>> {
    let sql = format!(
        "{CHAIN_CTE} SELECT {SELECT_COLS} {SELECT_JOINS} WHERE t.id = ?"
    );
    let mut stmt = conn.prepare(&sql)?;
    stmt.query_row(params![row_id], view_from_row)
        .optional()
        .context("failed to query task by row id")
}

/// Compact child-task payload for task view output.
#[derive(Debug, Clone, Serialize)]
pub struct ChildTask {
    pub id: String,
    pub status: String,
    pub priority: i64,
    pub title: String,
    pub order: String,
}

/// Walk the entire subtree rooted at `root_task_id` (any depth) and
/// return joined task views in depth-first sibling order. The root
/// itself is not included. Children are ordered by their `child_seq`,
/// padded so siblings 1, 2, 10 sort correctly.
pub fn descendants(conn: &Connection, root_task_id: i64) -> Result<Vec<TaskView>> {
    let sql = format!(
        "WITH RECURSIVE \
         crumbs(id, root_seq, suffix) AS (\
           SELECT id, seq, '' FROM tasks WHERE parent_id IS NULL \
           UNION ALL \
           SELECT t.id, c.root_seq, c.suffix || '.' || t.child_seq \
             FROM tasks t JOIN crumbs c ON c.id = t.parent_id), \
         walk(id, sortkey) AS (\
           SELECT id, printf('%012d', child_seq) FROM tasks WHERE parent_id = ?1 \
           UNION ALL \
           SELECT t.id, w.sortkey || '.' || printf('%012d', t.child_seq) \
             FROM tasks t JOIN walk w ON t.parent_id = w.id) \
         SELECT {SELECT_COLS} {SELECT_JOINS} \
         JOIN walk w ON w.id = t.id \
         WHERE t.deleted_at IS NULL \
         ORDER BY w.sortkey ASC"
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(params![root_task_id], view_from_row)?;
    rows.collect::<rusqlite::Result<Vec<_>>>()
        .context("failed to read descendants")
}

/// List direct child tasks for a parent, ordered by lexical `order` key.
pub fn children(conn: &Connection, parent_task_id: i64) -> Result<Vec<ChildTask>> {
    let sql = format!(
        "{CHAIN_CTE} \
         SELECT p.alias, printf('%04d', cr.root_seq) || cr.suffix AS dotted, \
                t.status, t.priority, t.title, \
                COALESCE(t.\"order\", printf('%012d', t.child_seq)) AS task_order \
         FROM tasks t \
         JOIN projects p ON p.id = t.project_id \
         JOIN crumbs cr ON cr.id = t.id \
         WHERE t.parent_id = ? AND t.deleted_at IS NULL \
         ORDER BY task_order ASC, t.child_seq ASC"
    );
    let mut stmt = conn.prepare(&sql)?;

    let rows = stmt.query_map(params![parent_task_id], |row| {
        let alias: String = row.get(0)?;
        let dotted: String = row.get(1)?;
        Ok(ChildTask {
            id: format_external_id(&alias, &dotted),
            status: row.get(2)?,
            priority: row.get(3)?,
            title: row.get(4)?,
            order: row.get(5)?,
        })
    })?;

    rows.collect::<rusqlite::Result<Vec<_>>>()
        .context("failed to query child tasks")
}

pub struct AddArgs<'a> {
    pub project_id: i64,
    pub title: &'a str,
    pub body: Option<&'a str>,
    pub priority: Option<i64>,
    pub cycle_id: Option<i64>,
    pub parent_id: Option<i64>,
    /// Explicit seq for migrations/imports. When `None`, auto-assigned
    /// as `max(seq) + 1` within the project.
    pub seq: Option<i64>,
    /// Explicit status (defaults to `open`).
    pub status: Option<&'a str>,
    /// Explicit order key. When `None`, falls back to [`default_order_key`].
    pub order: Option<&'a str>,
}

/// Insert a new task. Top-level tasks (no `parent_id`) auto-assign
/// `seq` as `MAX(seq) + 1` within the project; children auto-assign
/// `child_seq` as `MAX(child_seq) + 1` among siblings of the same
/// parent. Both happen inside a single transaction.
pub fn add(conn: &mut Connection, args: AddArgs) -> Result<TaskView> {
    let priority = args.priority.unwrap_or(3);
    if !(1..=5).contains(&priority) {
        bail!("priority must be between 1 and 5");
    }
    let status = args.status.unwrap_or("backlog");
    let closed = is_closed_status(conn, status)?;

    let tx = conn.transaction()?;

    let (seq, child_seq, order_seed): (Option<i64>, Option<i64>, i64) = match args.parent_id {
        None => {
            let s: i64 = match args.seq {
                Some(s) => {
                    if s < 1 {
                        bail!("seq must be >= 1");
                    }
                    s
                }
                None => tx
                    .query_row(
                        "SELECT COALESCE(MAX(seq), 0) + 1 FROM tasks \
                         WHERE project_id = ? AND parent_id IS NULL",
                        [args.project_id],
                        |row| row.get(0),
                    )
                    .context("failed to compute next seq")?,
            };
            (Some(s), None, s)
        }
        Some(pid) => {
            if args.seq.is_some() {
                bail!("cannot set explicit seq on a child task");
            }
            let cs: i64 = tx
                .query_row(
                    "SELECT COALESCE(MAX(child_seq), 0) + 1 FROM tasks WHERE parent_id = ?",
                    [pid],
                    |row| row.get(0),
                )
                .context("failed to compute next child_seq")?;
            (None, Some(cs), cs)
        }
    };

    let order_key = args
        .order
        .map(str::to_string)
        .unwrap_or_else(|| default_order_key(order_seed));
    tx.execute(
        "INSERT INTO tasks \
         (project_id, seq, child_seq, title, body, status, priority, \"order\", cycle_id, parent_id, closed_at) \
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, \
                 CASE WHEN ? = 1 \
                       THEN strftime('%Y-%m-%dT%H:%M:%fZ','now') \
                       ELSE NULL END)",
        params![
            args.project_id,
            seq,
            child_seq,
            args.title,
            args.body,
            status,
            priority,
            order_key,
            args.cycle_id,
            args.parent_id,
            closed as i64,
        ],
    )
    .context("failed to insert task")?;

    let row_id = tx.last_insert_rowid();
    tx.commit()?;

    get_by_row_id(conn, row_id)?.context("task missing after insert")
}

#[derive(Default)]
pub struct EditArgs<'a> {
    pub title: Option<&'a str>,
    pub body: Option<&'a str>,
    pub priority: Option<i64>,
    pub cycle_id: Option<Option<i64>>,
    pub parent_id: Option<Option<i64>>,
}

impl EditArgs<'_> {
    fn is_empty(&self) -> bool {
        self.title.is_none()
            && self.body.is_none()
            && self.priority.is_none()
            && self.cycle_id.is_none()
            && self.parent_id.is_none()
    }
}

/// Update mutable fields on a task. `None` leaves the field unchanged;
/// for `cycle_id` / `parent_id`, `Some(None)` explicitly clears.
///
/// `parent_id` transitions are non-trivial: top-level → child clears
/// `seq` and allocates a fresh `child_seq` under the new parent;
/// child → top-level clears `child_seq` and allocates `MAX(seq)+1`
/// among project top-level rows; child → different child reallocates
/// `child_seq`. The original `seq` is *not* preserved on demotion —
/// `KDB-0030` becoming a subtask permanently loses that id. The
/// `order` key is reset to a fresh default whenever the sibling
/// context changes.
pub fn edit(conn: &mut Connection, id: &TaskId, args: EditArgs) -> Result<TaskView> {
    if args.is_empty() {
        bail!("no fields to update");
    }
    let existing = get(conn, id)?
        .with_context(|| format!("task not found: {}", id.render()))?;
    if let Some(pri) = args.priority {
        if !(1..=5).contains(&pri) {
            bail!("priority must be between 1 and 5");
        }
    }

    let row_id = existing.task.id;
    let project_id = existing.task.project_id;
    let tx = conn.transaction()?;

    // Handle parent transitions first so seq/child_seq/order are
    // consistent before the rest of the fields land.
    if let Some(new_parent) = args.parent_id {
        let old_parent = existing.task.parent_id;
        if new_parent != old_parent {
            if let Some(pid) = new_parent {
                if pid == row_id {
                    bail!("cannot make a task its own parent");
                }
                let cs: i64 = tx
                    .query_row(
                        "SELECT COALESCE(MAX(child_seq), 0) + 1 FROM tasks WHERE parent_id = ?",
                        [pid],
                        |r| r.get(0),
                    )
                    .context("failed to compute next child_seq")?;
                tx.execute(
                    "UPDATE tasks SET parent_id = ?, seq = NULL, child_seq = ?, \"order\" = ?, \
                        updated_at = strftime('%Y-%m-%dT%H:%M:%fZ','now') \
                     WHERE id = ?",
                    params![pid, cs, default_order_key(cs), row_id],
                )
                .context("failed to set parent on task")?;
            } else {
                let s: i64 = tx
                    .query_row(
                        "SELECT COALESCE(MAX(seq), 0) + 1 FROM tasks \
                         WHERE project_id = ? AND parent_id IS NULL",
                        [project_id],
                        |r| r.get(0),
                    )
                    .context("failed to compute next seq")?;
                tx.execute(
                    "UPDATE tasks SET parent_id = NULL, seq = ?, child_seq = NULL, \"order\" = ?, \
                        updated_at = strftime('%Y-%m-%dT%H:%M:%fZ','now') \
                     WHERE id = ?",
                    params![s, default_order_key(s), row_id],
                )
                .context("failed to clear parent on task")?;
            }
        }
    }

    tx.execute(
        "UPDATE tasks SET \
            title      = COALESCE(?, title), \
            priority   = COALESCE(?, priority), \
            cycle_id   = CASE WHEN ? = 1 THEN ? ELSE cycle_id END, \
            updated_at = strftime('%Y-%m-%dT%H:%M:%fZ','now') \
         WHERE id = ?",
        params![
            args.title,
            args.priority,
            args.cycle_id.is_some() as i64,
            args.cycle_id.and_then(|x| x),
            row_id,
        ],
    )
    .context("failed to update task")?;
    if let Some(body) = args.body {
        tx.execute(
            "UPDATE tasks SET body = ?, \
             updated_at = strftime('%Y-%m-%dT%H:%M:%fZ','now') \
             WHERE id = ?",
            params![body, row_id],
        )?;
    }
    tx.commit()?;
    get_by_row_id(conn, row_id)?.context("task missing after update")
}

/// Side of a sibling task when inserting/moving by position.
#[derive(Debug, Copy, Clone)]
pub enum Side {
    Before,
    After,
}

/// Relative position for [`move_task`].
#[derive(Debug, Clone)]
pub enum MoveTarget<'a> {
    Before(&'a TaskId),
    After(&'a TaskId),
    Top,
    Bottom,
}

/// Compute an order key for a new task inserted adjacent to `sibling` on the given side,
/// within `sibling`'s `(project_id, parent_id)` context.
pub fn order_key_adjacent(
    conn: &Connection,
    sibling: &TaskView,
    side: Side,
) -> Result<String> {
    let dir = match side {
        Side::Before => Direction::Prev,
        Side::After => Direction::Next,
    };
    let neighbor = neighbor_order(
        conn,
        sibling.task.project_id,
        sibling.task.parent_id,
        Some(&sibling.task.order),
        dir,
        // No task to exclude when inserting a new row.
        -1,
    )?;
    let pivot = sibling.task.order.as_str();
    let (prev, next) = match side {
        Side::Before => (neighbor.as_deref(), Some(pivot)),
        Side::After => (Some(pivot), neighbor.as_deref()),
    };
    Ok(between(prev, next))
}

/// Move a task to a new position within its `(project_id, parent_id)` context.
/// `Before`/`After` require the sibling to be in the same context; otherwise errors.
pub fn move_task(conn: &Connection, id: &TaskId, target: MoveTarget) -> Result<TaskView> {
    let task = get(conn, id)?
        .with_context(|| format!("task not found: {}", id.render()))?;
    let project_id = task.task.project_id;
    let parent_id = task.task.parent_id;

    let new_order = match target {
        MoveTarget::Top => {
            let first = neighbor_order(conn, project_id, parent_id, None, Direction::First, task.task.id)?;
            between(None, first.as_deref())
        }
        MoveTarget::Bottom => {
            let last = neighbor_order(conn, project_id, parent_id, None, Direction::Last, task.task.id)?;
            between(last.as_deref(), None)
        }
        MoveTarget::Before(sib_id) => {
            let sib = get(conn, sib_id)?.with_context(|| {
                format!("sibling not found: {}", sib_id.render())
            })?;
            ensure_same_context(&task, &sib)?;
            if sib.task.id == task.task.id {
                return Ok(task);
            }
            let prev = neighbor_order(
                conn,
                project_id,
                parent_id,
                Some(&sib.task.order),
                Direction::Prev,
                task.task.id,
            )?;
            between(prev.as_deref(), Some(&sib.task.order))
        }
        MoveTarget::After(sib_id) => {
            let sib = get(conn, sib_id)?.with_context(|| {
                format!("sibling not found: {}", sib_id.render())
            })?;
            ensure_same_context(&task, &sib)?;
            if sib.task.id == task.task.id {
                return Ok(task);
            }
            let next = neighbor_order(
                conn,
                project_id,
                parent_id,
                Some(&sib.task.order),
                Direction::Next,
                task.task.id,
            )?;
            between(Some(&sib.task.order), next.as_deref())
        }
    };

    conn.execute(
        "UPDATE tasks SET \"order\" = ?, \
            updated_at = strftime('%Y-%m-%dT%H:%M:%fZ','now') \
         WHERE id = ?",
        params![new_order, task.task.id],
    )
    .context("failed to update task order")?;
    get(conn, id)?.context("task missing after move")
}

fn ensure_same_context(task: &TaskView, sib: &TaskView) -> Result<()> {
    if task.task.project_id != sib.task.project_id {
        bail!(
            "cannot move {}: target {} is in a different project",
            task.external_id(),
            sib.external_id()
        );
    }
    if task.task.parent_id != sib.task.parent_id {
        bail!(
            "cannot move {}: target {} has a different parent",
            task.external_id(),
            sib.external_id()
        );
    }
    Ok(())
}

#[derive(Copy, Clone)]
enum Direction {
    First,
    Last,
    Prev,
    Next,
}

/// Look up a neighboring task's order key within a `(project_id, parent_id)` context.
/// `exclude_id` is always filtered out so moving a task past its own current slot works.
fn neighbor_order(
    conn: &Connection,
    project_id: i64,
    parent_id: Option<i64>,
    pivot: Option<&str>,
    dir: Direction,
    exclude_id: i64,
) -> Result<Option<String>> {
    let parent_pred = match parent_id {
        Some(_) => "parent_id = ?",
        None => "parent_id IS NULL",
    };
    let (cmp, order) = match dir {
        Direction::First => ("", "ASC"),
        Direction::Last => ("", "DESC"),
        Direction::Prev => ("AND \"order\" < ?", "DESC"),
        Direction::Next => ("AND \"order\" > ?", "ASC"),
    };
    let sql = format!(
        "SELECT \"order\" FROM tasks \
         WHERE project_id = ? AND {parent_pred} AND id != ? {cmp} \
         ORDER BY \"order\" {order} LIMIT 1"
    );
    let mut stmt = conn.prepare(&sql)?;
    let mut params_vec: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();
    params_vec.push(Box::new(project_id));
    if let Some(pid) = parent_id {
        params_vec.push(Box::new(pid));
    }
    params_vec.push(Box::new(exclude_id));
    if let Some(p) = pivot {
        params_vec.push(Box::new(p.to_string()));
    }
    let params_iter = rusqlite::params_from_iter(params_vec.iter().map(|b| b.as_ref()));
    stmt.query_row(params_iter, |row| row.get::<_, String>(0))
        .optional()
        .context("failed to read neighbor order")
}

/// Set a task's status. `done` and `parked` stamp `closed_at`; other
/// statuses clear it.
pub fn set_status(conn: &Connection, id: &TaskId, status: &str) -> Result<TaskView> {
    let closed = is_closed_status(conn, status)?;
    let existing = get(conn, id)?
        .with_context(|| format!("task not found: {}", id.render()))?;

    conn.execute(
        "UPDATE tasks SET \
            status     = ?, \
            closed_at  = CASE WHEN ? = 1 \
                              THEN strftime('%Y-%m-%dT%H:%M:%fZ','now') \
                              ELSE NULL END, \
            updated_at = strftime('%Y-%m-%dT%H:%M:%fZ','now') \
         WHERE id = ?",
        params![status, closed as i64, existing.task.id],
    )?;
    get(conn, id)?.context("task missing after status change")
}

/// Collect every row id in the subtree rooted at `root_task_id` (the
/// root included), regardless of `deleted_at`. Used for cascade
/// operations.
fn subtree_ids(conn: &Connection, root_task_id: i64) -> Result<Vec<i64>> {
    let mut stmt = conn.prepare(
        "WITH RECURSIVE sub(id) AS ( \
            SELECT ?1 \
            UNION ALL \
            SELECT t.id FROM tasks t JOIN sub s ON t.parent_id = s.id \
         ) SELECT id FROM sub",
    )?;
    let rows = stmt.query_map(params![root_task_id], |r| r.get::<_, i64>(0))?;
    rows.collect::<rusqlite::Result<Vec<_>>>()
        .context("failed to collect subtree ids")
}

/// Soft-delete a task and its entire subtree. Sets `deleted_at` to now
/// on every descendant. Idempotent — already-deleted rows keep their
/// existing timestamp.
pub fn soft_delete(conn: &mut Connection, id: &TaskId) -> Result<TaskView> {
    let existing = get(conn, id)?
        .with_context(|| format!("task not found: {}", id.render()))?;
    let ids = subtree_ids(conn, existing.task.id)?;
    let tx = conn.transaction()?;
    let placeholders = vec!["?"; ids.len()].join(",");
    let sql = format!(
        "UPDATE tasks SET deleted_at = strftime('%Y-%m-%dT%H:%M:%fZ','now'), \
            updated_at = strftime('%Y-%m-%dT%H:%M:%fZ','now') \
         WHERE id IN ({placeholders}) AND deleted_at IS NULL"
    );
    let params_iter = rusqlite::params_from_iter(ids.iter());
    tx.execute(&sql, params_iter)?;
    tx.commit()?;
    get_by_row_id(conn, existing.task.id)?.context("task missing after soft delete")
}

/// Restore a soft-deleted task and its subtree. Clears `deleted_at`
/// on every descendant; `order` keys are preserved.
pub fn restore(conn: &mut Connection, id: &TaskId) -> Result<TaskView> {
    let row_id: i64 = {
        let mut row_id: i64 = conn
            .query_row(
                "SELECT t.id FROM tasks t JOIN projects p ON p.id = t.project_id \
                 WHERE p.alias = ? AND t.seq = ?",
                params![id.alias, id.seq],
                |r| r.get(0),
            )
            .with_context(|| format!("task not found: {}", id.render()))?;
        for n in &id.child_path {
            row_id = conn
                .query_row(
                    "SELECT id FROM tasks WHERE parent_id = ? AND child_seq = ?",
                    params![row_id, n],
                    |r| r.get::<_, i64>(0),
                )
                .with_context(|| format!("task not found: {}", id.render()))?;
        }
        row_id
    };
    let ids = subtree_ids(conn, row_id)?;
    let tx = conn.transaction()?;
    let placeholders = vec!["?"; ids.len()].join(",");
    let sql = format!(
        "UPDATE tasks SET deleted_at = NULL, \
            updated_at = strftime('%Y-%m-%dT%H:%M:%fZ','now') \
         WHERE id IN ({placeholders})"
    );
    tx.execute(&sql, rusqlite::params_from_iter(ids.iter()))?;
    tx.commit()?;
    get_by_row_id(conn, row_id)?.context("task missing after restore")
}

/// Permanently remove a task and its entire subtree. The self-FK has
/// no `ON DELETE CASCADE`, so we delete bottom-up inside a transaction
/// with `defer_foreign_keys = ON`.
pub fn hard_delete(conn: &mut Connection, id: &TaskId) -> Result<()> {
    let existing = get_including_deleted(conn, id)?
        .with_context(|| format!("task not found: {}", id.render()))?;
    let ids = subtree_ids(conn, existing.task.id)?;
    let tx = conn.transaction()?;
    tx.pragma_update(None, "defer_foreign_keys", "ON")?;
    let placeholders = vec!["?"; ids.len()].join(",");
    let sql = format!("DELETE FROM tasks WHERE id IN ({placeholders})");
    tx.execute(&sql, rusqlite::params_from_iter(ids.iter()))?;
    tx.commit()?;
    Ok(())
}

/// Resolve an external id to a `TaskView` regardless of `deleted_at`.
/// Used for `restore`/`hard_delete` where the row may be soft-deleted.
pub fn get_including_deleted(conn: &Connection, id: &TaskId) -> Result<Option<TaskView>> {
    let mut row_id: i64 = match conn
        .query_row(
            "SELECT t.id FROM tasks t JOIN projects p ON p.id = t.project_id \
             WHERE p.alias = ? AND t.seq = ?",
            params![id.alias, id.seq],
            |r| r.get(0),
        )
        .optional()?
    {
        Some(rid) => rid,
        None => return Ok(None),
    };
    for n in &id.child_path {
        row_id = match conn
            .query_row(
                "SELECT id FROM tasks WHERE parent_id = ? AND child_seq = ?",
                params![row_id, n],
                |r| r.get::<_, i64>(0),
            )
            .optional()?
        {
            Some(rid) => rid,
            None => return Ok(None),
        };
    }
    let sql = format!("{CHAIN_CTE} SELECT {SELECT_COLS} {SELECT_JOINS} WHERE t.id = ?");
    let mut stmt = conn.prepare(&sql)?;
    stmt.query_row(params![row_id], view_from_row)
        .optional()
        .context("failed to query task by row id (incl deleted)")
}

/// Selectors for [`purge`]. At least one of `status`, `deleted_only`
/// must be set; callers also need `project_slug` to scope.
pub struct PurgeFilters<'a> {
    pub project_slug: Option<&'a str>,
    pub status: Option<&'a str>,
    pub deleted_only: bool,
    pub dry_run: bool,
}

/// Permanently delete tasks matching the filters (and their subtrees).
/// Returns the rows that matched (or would have matched, on dry-run).
/// Refuses to run with no selector set.
pub fn purge(conn: &mut Connection, f: PurgeFilters) -> Result<Vec<TaskView>> {
    if f.status.is_none() && !f.deleted_only {
        bail!("purge requires at least one selector (--status or --deleted)");
    }

    // Build the match query. Always at top level of the subtree —
    // a matching subtask drags only its own subtree.
    let mut sql = format!(
        "{CHAIN_CTE} SELECT {SELECT_COLS} {SELECT_JOINS} WHERE 1=1"
    );
    let mut args: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();
    if let Some(slug) = f.project_slug {
        sql.push_str(" AND p.slug = ?");
        args.push(Box::new(slug.to_string()));
    }
    if let Some(status) = f.status {
        sql.push_str(" AND t.status = ?");
        args.push(Box::new(status.to_string()));
    }
    if f.deleted_only {
        sql.push_str(" AND t.deleted_at IS NOT NULL");
    }

    let matches: Vec<TaskView> = {
        let mut stmt = conn.prepare(&sql)?;
        let params_iter = rusqlite::params_from_iter(args.iter().map(|b| b.as_ref()));
        stmt.query_map(params_iter, view_from_row)?
            .collect::<rusqlite::Result<Vec<_>>>()?
    };

    if f.dry_run || matches.is_empty() {
        return Ok(matches);
    }

    let mut all_ids: Vec<i64> = Vec::new();
    for m in &matches {
        all_ids.extend(subtree_ids(conn, m.task.id)?);
    }
    all_ids.sort();
    all_ids.dedup();

    let tx = conn.transaction()?;
    tx.pragma_update(None, "defer_foreign_keys", "ON")?;
    let placeholders = vec!["?"; all_ids.len()].join(",");
    let sql = format!("DELETE FROM tasks WHERE id IN ({placeholders})");
    tx.execute(&sql, rusqlite::params_from_iter(all_ids.iter()))?;
    tx.commit()?;

    Ok(matches)
}

fn status_glyph(status: &str) -> &'static str {
    match status {
        "backlog" => "[ ]",
        "cycle" => "[>]",
        "in_progress" => "[~]",
        "done" => "[x]",
        "parked" => "[=]",
        _ => "[?]",
    }
}

/// Render a list of tasks as an aligned text table.
pub fn render_list(tasks: &[TaskView]) -> String {
    if tasks.is_empty() {
        return String::from("(no tasks)\n");
    }
    let id_strs: Vec<String> = tasks.iter().map(|t| t.external_id()).collect();
    let id_w = id_strs.iter().map(|s| s.len()).max().unwrap_or(4).max(4);

    // Show a project column only when the rows span more than one project
    // (e.g. a `--space` rollup or `-P all`); single-project lists stay compact.
    let multi_project = tasks
        .iter()
        .map(|t| t.project_slug.as_str())
        .collect::<std::collections::HashSet<_>>()
        .len()
        > 1;

    let mut out = String::new();
    if multi_project {
        let proj_w = tasks
            .iter()
            .map(|t| t.project_slug.len())
            .max()
            .unwrap_or(7)
            .max(7);
        out.push_str(&format!("{:<id_w$}  {:<proj_w$}  st    p  title\n", "id", "project"));
        for (t, id) in tasks.iter().zip(id_strs.iter()) {
            out.push_str(&format!(
                "{:<id_w$}  {:<proj_w$}  {}  {}  {}\n",
                id,
                t.project_slug,
                status_glyph(&t.task.status),
                t.task.priority,
                t.task.title,
            ));
        }
    } else {
        out.push_str(&format!("{:<id_w$}  st    p  title\n", "id"));
        for (t, id) in tasks.iter().zip(id_strs.iter()) {
            out.push_str(&format!(
                "{:<id_w$}  {}  {}  {}\n",
                id,
                status_glyph(&t.task.status),
                t.task.priority,
                t.task.title,
            ));
        }
    }
    out
}

/// Render a single task as a human-readable block. `label_slugs` are
/// joined with `, ` and shown in the metadata section when non-empty.
pub fn render_show(t: &TaskView, label_slugs: &[&str], children: &[ChildTask]) -> String {
    let mut out = String::new();
    out.push_str(&format!("id:         {}\n", t.external_id()));
    out.push_str(&format!("title:      {}\n", t.task.title));
    out.push_str(&format!("status:     {}\n", t.task.status));
    out.push_str(&format!("priority:   {}\n", t.task.priority));
    out.push_str(&format!("order:      {}\n", t.task.order));
    out.push_str(&format!("project:    {}\n", t.project_slug));
    if let Some(c) = &t.cycle_key {
        out.push_str(&format!("cycle:      {c}\n"));
    }
    if !label_slugs.is_empty() {
        out.push_str(&format!("labels:     {}\n", label_slugs.join(", ")));
    }
    out.push_str(&format!("created_at: {}\n", t.task.created_at));
    out.push_str(&format!("updated_at: {}\n", t.task.updated_at));
    if let Some(closed) = &t.task.closed_at {
        out.push_str(&format!("closed_at:  {closed}\n"));
    }
    if let Some(body) = &t.task.body {
        out.push_str("\n");
        out.push_str(body);
        if !body.ends_with('\n') {
            out.push('\n');
        }
    }
    if !children.is_empty() {
        out.push_str("\nchildren:\n");
        for child in children {
            out.push_str(&format!(
                "- {}  {}  p{}  ord={}  {}\n",
                child.id,
                status_glyph(&child.status),
                child.priority,
                child.order,
                child.title,
            ));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db;
    use crate::projects::{self, AddArgs as ProjAddArgs};
    use crate::workspace::root::ROOT_MARKER;
    use tempfile::TempDir;

    fn setup() -> (TempDir, Connection, i64) {
        let tmp = TempDir::new().unwrap();
        std::fs::create_dir(tmp.path().join(ROOT_MARKER)).unwrap();
        let conn = db::open(tmp.path()).unwrap();
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
        (tmp, conn, p.id)
    }

    #[test]
    fn parse_task_id_uppercases_alias() {
        let id = TaskId::parse("hrm-0120").unwrap();
        assert_eq!(id.alias, "HRM");
        assert_eq!(id.seq, 120);

        assert!(TaskId::parse("nodash").is_err());
        assert!(TaskId::parse("slug-abc").is_err());
    }

    #[test]
    fn external_id_is_zero_padded() {
        assert_eq!(format_external_id("HRM", &dotted_top(1)), "HRM-0001");
        assert_eq!(format_external_id("KDB", &dotted_top(120)), "KDB-0120");
        assert_eq!(format_external_id("SWF", &dotted_top(99999)), "SWF-99999");
    }

    #[test]
    fn parse_dotted_task_id_round_trips() {
        let id = TaskId::parse("KDB-0030.1.2").unwrap();
        assert_eq!(id.alias, "KDB");
        assert_eq!(id.seq, 30);
        assert_eq!(id.child_path, vec![1, 2]);
        assert_eq!(id.render(), "KDB-0030.1.2");

        let id = TaskId::parse("kdb-0001").unwrap();
        assert_eq!(id.alias, "KDB");
        assert_eq!(id.child_path, Vec::<i64>::new());
        assert_eq!(id.render(), "KDB-0001");

        assert!(TaskId::parse("KDB-0030.").is_err());
        assert!(TaskId::parse("KDB-0030..1").is_err());
        assert!(TaskId::parse("KDB-0030.0").is_err());
        assert!(TaskId::parse("KDB-0030.abc").is_err());
    }

    #[test]
    fn add_auto_seq_then_explicit_seq() {
        let (_tmp, mut conn, pid) = setup();
        let a = add(
            &mut conn,
            AddArgs {
                project_id: pid,
                title: "first",
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
        assert_eq!(a.task.seq, Some(1));
        assert_eq!(a.external_id(), "KDB-0001");

        let b = add(
            &mut conn,
            AddArgs {
                project_id: pid,
                title: "explicit",
                body: Some("body text"),
                priority: Some(2),
                cycle_id: None,
                parent_id: None,
                seq: Some(120),
                status: None,
                order: None,
            },
        )
        .unwrap();
        assert_eq!(b.task.seq, Some(120));
        assert_eq!(b.external_id(), "KDB-0120");

        let c = add(
            &mut conn,
            AddArgs {
                project_id: pid,
                title: "after explicit",
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
        assert_eq!(c.task.seq, Some(121));
    }

    #[test]
    fn child_does_not_consume_top_level_seq() {
        let (_tmp, mut conn, pid) = setup();
        let parent = add_task(&mut conn, pid, "parent", None);
        let child = add_task(&mut conn, pid, "child", Some(parent.task.id));
        let next_top = add_task(&mut conn, pid, "next", None);

        assert_eq!(parent.task.seq, Some(1));
        assert_eq!(parent.task.child_seq, None);
        assert_eq!(child.task.seq, None);
        assert_eq!(child.task.child_seq, Some(1));
        assert_eq!(child.external_id(), "KDB-0001.1");
        assert_eq!(next_top.task.seq, Some(2));
        assert_eq!(next_top.external_id(), "KDB-0002");
    }

    #[test]
    fn unparent_allocates_top_level_seq() {
        let (_tmp, mut conn, pid) = setup();
        let parent = add_task(&mut conn, pid, "parent", None);
        let child = add_task(&mut conn, pid, "child", Some(parent.task.id));
        assert_eq!(child.task.seq, None);

        let id = TaskId::parse(&child.external_id()).unwrap();
        let updated = edit(
            &mut conn,
            &id,
            EditArgs {
                parent_id: Some(None),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(updated.task.parent_id, None);
        assert_eq!(updated.task.child_seq, None);
        assert_eq!(updated.task.seq, Some(2));
        assert_eq!(updated.external_id(), "KDB-0002");
    }

    #[test]
    fn reparent_top_level_to_child_clears_seq() {
        let (_tmp, mut conn, pid) = setup();
        let a = add_task(&mut conn, pid, "a", None);
        let b = add_task(&mut conn, pid, "b", None);
        assert_eq!(b.task.seq, Some(2));

        let b_id = TaskId::parse(&b.external_id()).unwrap();
        let updated = edit(
            &mut conn,
            &b_id,
            EditArgs {
                parent_id: Some(Some(a.task.id)),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(updated.task.parent_id, Some(a.task.id));
        assert_eq!(updated.task.seq, None);
        assert_eq!(updated.task.child_seq, Some(1));
        assert_eq!(updated.external_id(), "KDB-0001.1");
    }

    #[test]
    fn reparent_between_parents_renumbers_child_seq() {
        let (_tmp, mut conn, pid) = setup();
        let a = add_task(&mut conn, pid, "a", None);
        let b = add_task(&mut conn, pid, "b", None);
        let _ax = add_task(&mut conn, pid, "ax", Some(a.task.id));
        let bx = add_task(&mut conn, pid, "bx", Some(b.task.id));
        assert_eq!(bx.task.child_seq, Some(1));

        let bx_id = TaskId::parse(&bx.external_id()).unwrap();
        let updated = edit(
            &mut conn,
            &bx_id,
            EditArgs {
                parent_id: Some(Some(a.task.id)),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(updated.task.parent_id, Some(a.task.id));
        assert_eq!(updated.task.child_seq, Some(2));
        assert_eq!(updated.external_id(), "KDB-0001.2");
    }

    #[test]
    fn grandchild_dotted_id() {
        let (_tmp, mut conn, pid) = setup();
        let parent = add_task(&mut conn, pid, "parent", None);
        let child = add_task(&mut conn, pid, "child", Some(parent.task.id));
        let grand = add_task(&mut conn, pid, "grand", Some(child.task.id));
        assert_eq!(child.external_id(), "KDB-0001.1");
        assert_eq!(grand.external_id(), "KDB-0001.1.1");
        assert_eq!(grand.task.child_seq, Some(1));
    }

    #[test]
    fn add_with_status() {
        let (_tmp, mut conn, pid) = setup();
        let t = add(
            &mut conn,
            AddArgs {
                project_id: pid,
                title: "done one",
                body: None,
                priority: None,
                cycle_id: None,
                parent_id: None,
                seq: None,
                status: Some("done"),
                order: None,
            },
        )
        .unwrap();
        assert_eq!(t.task.status, "done");
        assert!(t.task.closed_at.is_some());
    }

    #[test]
    fn set_status_transitions() {
        let (_tmp, mut conn, pid) = setup();
        let t = add(
            &mut conn,
            AddArgs {
                project_id: pid,
                title: "first",
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
        let id = TaskId::parse(&t.external_id()).unwrap();

        let done = set_status(&conn, &id, "done").unwrap();
        assert_eq!(done.task.status, "done");
        assert!(done.task.closed_at.is_some());

        let reopened = set_status(&conn, &id, "backlog").unwrap();
        assert_eq!(reopened.task.status, "backlog");
        assert!(reopened.task.closed_at.is_none());
    }

    #[test]
    fn children_are_ordered_by_order_key() {
        let (_tmp, mut conn, pid) = setup();
        let parent = add(
            &mut conn,
            AddArgs {
                project_id: pid,
                title: "parent",
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
        let child_a = add(
            &mut conn,
            AddArgs {
                project_id: pid,
                title: "child a",
                body: None,
                priority: Some(2),
                cycle_id: None,
                parent_id: Some(parent.task.id),
                seq: None,
                status: None,
                order: None,
            },
        )
        .unwrap();
        let child_b = add(
            &mut conn,
            AddArgs {
                project_id: pid,
                title: "child b",
                body: None,
                priority: Some(1),
                cycle_id: None,
                parent_id: Some(parent.task.id),
                seq: None,
                status: None,
                order: None,
            },
        )
        .unwrap();

        conn.execute(
            "UPDATE tasks SET \"order\" = ? WHERE id = ?",
            params!["b", child_a.task.id],
        )
        .unwrap();
        conn.execute(
            "UPDATE tasks SET \"order\" = ? WHERE id = ?",
            params!["a", child_b.task.id],
        )
        .unwrap();

        let kids = children(&conn, parent.task.id).unwrap();
        assert_eq!(kids.len(), 2);
        assert_eq!(kids[0].id, child_b.external_id());
        assert_eq!(kids[0].order, "a");
        assert_eq!(kids[1].id, child_a.external_id());
        assert_eq!(kids[1].order, "b");
    }

    #[test]
    fn render_show_includes_children_section() {
        let (_tmp, mut conn, pid) = setup();
        let parent = add(
            &mut conn,
            AddArgs {
                project_id: pid,
                title: "parent",
                body: Some("details"),
                priority: Some(2),
                cycle_id: None,
                parent_id: None,
                seq: None,
                status: None,
                order: None,
            },
        )
        .unwrap();
        let _child = add(
            &mut conn,
            AddArgs {
                project_id: pid,
                title: "child",
                body: None,
                priority: Some(1),
                cycle_id: None,
                parent_id: Some(parent.task.id),
                seq: None,
                status: None,
                order: None,
            },
        )
        .unwrap();

        let kids = children(&conn, parent.task.id).unwrap();
        let rendered = render_show(&parent, &[], &kids);
        assert!(rendered.contains("children:"));
        assert!(rendered.contains(&kids[0].id));
        assert!(rendered.contains("ord="));
    }

    fn add_task(conn: &mut Connection, pid: i64, title: &str, parent: Option<i64>) -> TaskView {
        add(
            conn,
            AddArgs {
                project_id: pid,
                title,
                body: None,
                priority: None,
                cycle_id: None,
                parent_id: parent,
                seq: None,
                status: None,
                order: None,
            },
        )
        .unwrap()
    }

    #[test]
    fn between_sits_strictly_between_various_pairs() {
        let cases: &[(Option<&str>, Option<&str>)] = &[
            (None, None),
            (Some("000000000001"), Some("000000000002")),
            (Some("a"), Some("b")),
            (Some("a"), Some("c")),
            (Some("a"), Some("az")),
            (None, Some("m")),
            (Some("m"), None),
            (Some("abc"), Some("abd")),
            (Some("0"), Some("z")),
            (Some("zzz"), None),
        ];
        for &(prev, next) in cases {
            let mid = between(prev, next);
            if let Some(p) = prev {
                assert!(
                    p < mid.as_str(),
                    "between({prev:?}, {next:?}) = {mid:?} is not > {p:?}",
                );
            }
            if let Some(n) = next {
                assert!(
                    mid.as_str() < n,
                    "between({prev:?}, {next:?}) = {mid:?} is not < {n:?}",
                );
            }
        }
    }

    #[test]
    fn move_task_before_and_after_reorders() {
        let (_tmp, mut conn, pid) = setup();
        let a = add_task(&mut conn, pid, "a", None);
        let b = add_task(&mut conn, pid, "b", None);
        let c = add_task(&mut conn, pid, "c", None);

        let a_id = TaskId::parse(&a.external_id()).unwrap();
        let b_id = TaskId::parse(&b.external_id()).unwrap();
        let c_id = TaskId::parse(&c.external_id()).unwrap();

        // Move c before a → order: c, a, b
        let _ = move_task(&conn, &c_id, MoveTarget::Before(&a_id)).unwrap();
        let listed = list(
            &conn,
            ListFilters {
                statuses: None,
                project_slug: None,
                space_slug: None,
                cycle_key: None,
                priority: None,
                limit: None,
                top_level_only: false,
            },
        )
        .unwrap();
        assert_eq!(
            listed.iter().map(|t| t.task.title.as_str()).collect::<Vec<_>>(),
            vec!["c", "a", "b"],
        );

        // Move a after b → order: c, b, a
        let _ = move_task(&conn, &a_id, MoveTarget::After(&b_id)).unwrap();
        let listed = list(
            &conn,
            ListFilters {
                statuses: None,
                project_slug: None,
                space_slug: None,
                cycle_key: None,
                priority: None,
                limit: None,
                top_level_only: false,
            },
        )
        .unwrap();
        assert_eq!(
            listed.iter().map(|t| t.task.title.as_str()).collect::<Vec<_>>(),
            vec!["c", "b", "a"],
        );
    }

    #[test]
    fn move_task_top_and_bottom() {
        let (_tmp, mut conn, pid) = setup();
        let a = add_task(&mut conn, pid, "a", None);
        let _b = add_task(&mut conn, pid, "b", None);
        let _c = add_task(&mut conn, pid, "c", None);
        let a_id = TaskId::parse(&a.external_id()).unwrap();

        let _ = move_task(&conn, &a_id, MoveTarget::Bottom).unwrap();
        let listed = list(
            &conn,
            ListFilters {
                statuses: None,
                project_slug: None,
                space_slug: None,
                cycle_key: None,
                priority: None,
                limit: None,
                top_level_only: false,
            },
        )
        .unwrap();
        assert_eq!(listed.last().unwrap().task.title, "a");

        let _ = move_task(&conn, &a_id, MoveTarget::Top).unwrap();
        let listed = list(
            &conn,
            ListFilters {
                statuses: None,
                project_slug: None,
                space_slug: None,
                cycle_key: None,
                priority: None,
                limit: None,
                top_level_only: false,
            },
        )
        .unwrap();
        assert_eq!(listed.first().unwrap().task.title, "a");
    }

    #[test]
    fn move_task_rejects_different_parent() {
        let (_tmp, mut conn, pid) = setup();
        let parent = add_task(&mut conn, pid, "parent", None);
        let child = add_task(&mut conn, pid, "child", Some(parent.task.id));
        let root = add_task(&mut conn, pid, "root", None);

        let child_id = TaskId::parse(&child.external_id()).unwrap();
        let root_id = TaskId::parse(&root.external_id()).unwrap();

        let err = move_task(&conn, &child_id, MoveTarget::Before(&root_id)).unwrap_err();
        assert!(
            err.to_string().contains("different parent"),
            "unexpected error: {err}",
        );
    }

    #[test]
    fn order_key_adjacent_after_sits_between_neighbors() {
        let (_tmp, mut conn, pid) = setup();
        let a = add_task(&mut conn, pid, "a", None);
        let b = add_task(&mut conn, pid, "b", None);

        let key_after_a = order_key_adjacent(&conn, &a, Side::After).unwrap();
        assert!(key_after_a > a.task.order, "{key_after_a} > {}", a.task.order);
        assert!(key_after_a < b.task.order, "{key_after_a} < {}", b.task.order);

        let key_before_a = order_key_adjacent(&conn, &a, Side::Before).unwrap();
        assert!(key_before_a < a.task.order, "{key_before_a} < {}", a.task.order);
    }
}
