//! Markdown task tables: v1's `| Task | Title | Priority |` plus the icon
//! column (iss-0070) and the Blocked-by column (iss-0071).

use std::collections::HashMap;

use super::queries::{Blockers, Status, Task};

/// Everything a table needs beyond its rows.
pub struct TableCtx<'a> {
    /// Relative path from the file being written to `.kdb/icons/`, with a trailing slash.
    pub icon_prefix: String,
    /// Status slug -> icon file stem.
    pub icons: HashMap<&'a str, &'a str>,
    pub blockers: &'a Blockers,
}

impl<'a> TableCtx<'a> {
    pub fn new(owner_path: &str, statuses: &'a [Status], blockers: &'a Blockers) -> Self {
        let icons = statuses
            .iter()
            .filter_map(|s| s.icon.as_deref().map(|i| (s.slug.as_str(), i)))
            .collect();
        Self { icon_prefix: icon_prefix(owner_path), icons, blockers }
    }

    /// `![](<prefix><icon>.svg)` for a status slug, or empty when the status has no icon.
    pub fn status_icon(&self, slug: &str) -> String {
        match self.icons.get(slug) {
            Some(icon) => format!("![]({}{icon}.svg)", self.icon_prefix),
            None => String::new(),
        }
    }

    fn icon_cell(&self, task: &Task) -> String {
        self.status_icon(&task.status)
    }

    fn blocked_cell(&self, task: &Task) -> String {
        self.blockers.of(task.id).join(", ")
    }
}

/// Relative path from `<owner_path>/.tasks/` up to the workspace root, then
/// into `.kdb/icons/`. `projects/kdb` gives `../../../.kdb/icons/`; the root
/// project (`.`) gives `../.kdb/icons/`.
pub fn icon_prefix(owner_path: &str) -> String {
    let depth = owner_path.split('/').filter(|c| !c.is_empty() && *c != ".").count() + 1;
    format!("{}.kdb/icons/", "../".repeat(depth))
}

/// On-disk name for a top-level task's file: `T-{seq:04}.md`.
pub fn task_file_name(seq: i64) -> String {
    format!("T-{seq:04}.md")
}

/// Top-level seq; the renderer only writes files for top-level tasks.
pub fn top_seq(t: &Task) -> i64 {
    t.seq.expect("render only writes files for top-level tasks")
}

pub fn escape_md_cell(s: &str) -> String {
    s.replace('|', "\\|").replace('\n', " ")
}

/// The Task cell: a link to the task file under `prefix`, or an unlinked code span.
fn task_cell(t: &Task, link_prefix: Option<&str>) -> String {
    match link_prefix {
        Some(p) => format!("[{}]({p}{})", t.external_id, task_file_name(top_seq(t))),
        None => format!("`{}`", t.external_id),
    }
}

/// `| | Task | Title | Blocked by | Priority |` for one owner. `link_prefix`
/// `None` renders unlinked rows (closed statuses, subtask tables).
pub fn push_table(out: &mut String, ctx: &TableCtx, tasks: &[&Task], link_prefix: Option<&str>) {
    if tasks.is_empty() {
        out.push_str("_(none)_\n\n");
        return;
    }
    out.push_str("| | Task | Title | Blocked by | Priority |\n|---|---|---|---|---|\n");
    for t in tasks {
        out.push_str(&format!(
            "| {} | {} | {} | {} | {} |\n",
            ctx.icon_cell(t),
            task_cell(t, link_prefix),
            escape_md_cell(&t.title),
            ctx.blocked_cell(t),
            t.priority,
        ));
    }
    out.push('\n');
}

/// `| | Task | Project | Title | Blocked by | Priority |` for a merged,
/// cross-owner slice of a space board. Space-native rows link beside the
/// board; member-project rows use their prefix from `member_prefix`.
pub fn push_space_table(
    out: &mut String,
    ctx: &TableCtx,
    tasks: &[&Task],
    member_prefix: &HashMap<String, String>,
    linkable: bool,
) {
    if tasks.is_empty() {
        out.push_str("_(none)_\n\n");
        return;
    }
    out.push_str("| | Task | Project | Title | Blocked by | Priority |\n|---|---|---|---|---|---|\n");
    for t in tasks {
        let prefix = if !linkable {
            None
        } else if t.space_id.is_some() {
            Some("")
        } else {
            Some(member_prefix.get(&t.owner_slug).map(String::as_str).unwrap_or(""))
        };
        out.push_str(&format!(
            "| {} | {} | {} | {} | {} | {} |\n",
            ctx.icon_cell(t),
            task_cell(t, prefix),
            t.owner_slug,
            escape_md_cell(&t.title),
            ctx.blocked_cell(t),
            t.priority,
        ));
    }
    out.push('\n');
}
