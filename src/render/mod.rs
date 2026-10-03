//! DB -> markdown task boards.
//!
//! Writes `<owner path>/.tasks/index.md` plus one `T-{seq:04}.md` per open,
//! visible top-level task (capped at `top_n`), and removes task files that no
//! longer correspond to a row. `TODO.md` and other hand-written files are left
//! alone. Output matches v1 except for the icon column (iss-0070), the
//! Blocked-by column (iss-0071), and the `blocked_by:` frontmatter.

pub mod cli;
pub mod queries;
pub mod table;
pub mod task_file;

use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use rusqlite::Connection;

use queries::{Blockers, Project, Scope, Status, Task};
use table::{TableCtx, push_space_table, push_table, task_file_name, top_seq};
use task_file::{GENERATED_NOTE, render_task_file};

const TASKS_DIR: &str = ".tasks";
const INDEX_FILE: &str = "index.md";
const ICONS_DIR: &str = ".kdb/icons";

/// Status icon SVGs; `task_statuses.icon` holds the file stem.
const ICONS: &[(&str, &[u8])] = &[
    ("backlog", include_bytes!("../../docs/assets/backlog.svg")),
    ("queued", include_bytes!("../../docs/assets/queued.svg")),
    ("in_progress", include_bytes!("../../docs/assets/in_progress.svg")),
    ("in_review", include_bytes!("../../docs/assets/in_review.svg")),
    ("done", include_bytes!("../../docs/assets/done.svg")),
];

/// Transparent space added above each glyph, in px. Markdown viewers that
/// top-align inline images (Zed) leave a bare icon sitting high against the
/// text; padding the SVG pushes the glyph down. Headings get more because
/// their line box is taller.
const ROW_PAD: f64 = 1.5;
const HEADING_PAD: f64 = 3.5;

/// Ensure `.kdb/icons/` holds each icon twice: `<name>.svg` for table rows and
/// `<name>.h.svg` for section headings. Writes only on a difference.
pub fn ensure_icons(root: &Path) -> Result<()> {
    let dir = root.join(ICONS_DIR);
    fs::create_dir_all(&dir).with_context(|| format!("failed to create {}", dir.display()))?;
    for (name, bytes) in ICONS {
        let svg = std::str::from_utf8(bytes).expect("icon SVGs are UTF-8");
        for (file, pad) in [(format!("{name}.svg"), ROW_PAD), (format!("{name}.h.svg"), HEADING_PAD)] {
            let content = pad_top(svg, pad);
            let path = dir.join(file);
            if fs::read_to_string(&path).map(|cur| cur == content).unwrap_or(false) {
                continue;
            }
            fs::write(&path, content).with_context(|| format!("failed to write {}", path.display()))?;
        }
    }
    Ok(())
}

/// Grow the root `<svg>` upward by `pad`: raise its `height` and extend the
/// `viewBox` above the origin, so the drawing keeps its size and moves down.
fn pad_top(svg: &str, pad: f64) -> String {
    let attr = |name: &str| -> Option<(usize, usize)> {
        let key = format!(" {name}=\"");
        let start = svg.find(&key)? + key.len();
        let end = start + svg[start..].find('"')?;
        Some((start, end))
    };
    let (Some((hs, he)), Some((vs, ve))) = (attr("height"), attr("viewBox")) else {
        return svg.to_string();
    };
    let Ok(height) = svg[hs..he].parse::<f64>() else { return svg.to_string() };
    let vb: Vec<f64> = svg[vs..ve].split_whitespace().filter_map(|n| n.parse().ok()).collect();
    let [x, y, w, h] = vb[..] else { return svg.to_string() };
    let mut out = svg.to_string();
    // Replace the later attribute first so the earlier offsets stay valid.
    let mut edits = [(hs, he, format!("{}", height + pad)), (vs, ve, format!("{x} {} {w} {}", y - pad, h + pad))];
    edits.sort_by_key(|e| std::cmp::Reverse(e.0));
    for (s, e, v) in edits {
        out.replace_range(s..e, &v);
    }
    out
}

/// Shared per-render state.
struct Renderer<'a> {
    conn: &'a Connection,
    root: &'a Path,
    statuses: Vec<Status>,
    blockers: Blockers,
    top_n: i64,
}

impl<'a> Renderer<'a> {
    fn new(conn: &'a Connection, root: &'a Path) -> Result<Self> {
        ensure_icons(root)?;
        Ok(Self {
            conn,
            root,
            statuses: queries::statuses(conn)?,
            blockers: Blockers::load(conn)?,
            top_n: queries::top_n(conn)?,
        })
    }

    fn ctx(&self, owner_path: &str) -> TableCtx<'_> {
        TableCtx::new(owner_path, &self.statuses, &self.blockers)
    }

    fn out_dir(&self, owner_path: &str) -> Result<PathBuf> {
        let dir = self.root.join(owner_path).join(TASKS_DIR);
        fs::create_dir_all(&dir).with_context(|| format!("failed to create {}", dir.display()))?;
        Ok(dir)
    }

    fn project(&self, project: &Project, limit: Option<i64>) -> Result<PathBuf> {
        let out_dir = self.out_dir(&project.path)?;
        let top_n = limit.unwrap_or(self.top_n);
        let ctx = self.ctx(&project.path);
        let all = queries::top_level_tasks(self.conn, Scope::Project(&project.slug))?;
        let by_status = group_by_status(&all);
        self.write_task_files(&ctx, &out_dir, &by_status, top_n)?;

        let mut body = format!("# {} — Tasks\n\n{GENERATED_NOTE}\n\n", project.slug);
        let empty = Vec::new();
        for status in &self.statuses {
            let tasks = by_status.get(status.slug.as_str()).unwrap_or(&empty);
            if status.is_hidden {
                push_hidden(&mut body, &ctx, status, tasks.len(), "-P", &project.slug);
                continue;
            }
            let slice = truncate(tasks, top_n);
            push_heading(&mut body, &ctx, status, slice.len(), tasks.len());
            let prefix = if status.is_closed { None } else { Some("") };
            push_table(&mut body, &ctx, slice, prefix);
        }
        body.push_str(&format!(
            "## Commands\n\n\
             - `kdb tasks list -P {slug}` — full list\n\
             - `kdb tasks add \"title\" -P {slug}` — add a task\n\
             - `kdb tasks view <id>` — view a task\n",
            slug = project.slug,
        ));
        let out_path = out_dir.join(INDEX_FILE);
        fs::write(&out_path, body).with_context(|| format!("failed to write {}", out_path.display()))?;
        Ok(out_path)
    }

    /// Status-major rollup across every member project plus the space's own tasks.
    fn space(&self, slug: &str) -> Result<PathBuf> {
        let space = queries::space_by_slug(self.conn, slug)?
            .with_context(|| format!("space not found: {slug}"))?;
        let space_path = space.path.clone().with_context(|| {
            format!("space {slug} has no path — set one with `kdb spaces edit {slug} --path <rel>`")
        })?;
        space.alias.as_deref().with_context(|| {
            format!("space {slug} has no alias — set one with `kdb spaces edit {slug} --alias <ABC>`")
        })?;
        let out_dir = self.out_dir(&space_path)?;
        let ctx = self.ctx(&space_path);

        let native = queries::top_level_tasks(self.conn, Scope::SpaceNative(slug))?;
        self.write_task_files(&ctx, &out_dir, &group_by_status(&native), self.top_n)?;

        // Refresh each member board so links resolve. A member sharing the
        // space's path would clobber the space dir; skip it.
        let mut member_prefix: HashMap<String, String> = HashMap::new();
        for member in queries::projects(self.conn, Some(slug))? {
            if member.path == space_path {
                continue;
            }
            self.project(&member, None)?;
            member_prefix.insert(member.slug.clone(), relative_tasks_prefix(&space_path, &member.path));
        }

        let all = queries::top_level_tasks(self.conn, Scope::Space(slug))?;
        let by_status = group_by_status(&all);
        let mut out = format!("# {} — Tasks\n\n{GENERATED_NOTE}\n\n", space.name);
        let empty = Vec::new();
        for status in &self.statuses {
            let tasks = by_status.get(status.slug.as_str()).unwrap_or(&empty);
            // The space board is a live-work cockpit: backlog collapses too.
            if status.is_hidden || status.slug == "backlog" {
                push_hidden(&mut out, &ctx, status, tasks.len(), "-S", slug);
                continue;
            }
            let mut merged: Vec<&Task> = tasks.clone();
            merged.sort_by(|a, b| {
                a.priority
                    .cmp(&b.priority)
                    .then_with(|| a.owner_slug.cmp(&b.owner_slug))
                    .then_with(|| a.order.cmp(&b.order))
            });
            let shown = truncate(&merged, self.top_n);
            push_heading(&mut out, &ctx, status, shown.len(), tasks.len());
            push_space_table(&mut out, &ctx, shown, &member_prefix, !status.is_closed);
        }
        let out_path = out_dir.join(INDEX_FILE);
        fs::write(&out_path, out).with_context(|| format!("failed to write {}", out_path.display()))?;
        Ok(out_path)
    }

    /// Write `T-*.md` for every open, visible status (capped at `top_n`), then clean stale files.
    fn write_task_files(
        &self,
        ctx: &TableCtx,
        out_dir: &Path,
        by_status: &HashMap<&str, Vec<&Task>>,
        top_n: i64,
    ) -> Result<()> {
        let mut expected: HashSet<String> = HashSet::new();
        for status in &self.statuses {
            if status.is_closed || status.is_hidden {
                continue;
            }
            let Some(tasks) = by_status.get(status.slug.as_str()) else { continue };
            for t in truncate(tasks, top_n) {
                let name = task_file_name(top_seq(t));
                let subtree = queries::descendants(self.conn, t.id)?;
                let path = out_dir.join(&name);
                fs::write(&path, render_task_file(ctx, t, &subtree))
                    .with_context(|| format!("failed to write {}", path.display()))?;
                expected.insert(name);
            }
        }
        clean_stale_task_files(out_dir, &expected)
    }
}

pub fn materialize_project(conn: &Connection, root: &Path, slug: &str, limit: Option<i64>) -> Result<PathBuf> {
    let project =
        queries::project_by_slug(conn, slug)?.with_context(|| format!("project not found: {slug}"))?;
    Renderer::new(conn, root)?.project(&project, limit)
}

pub fn materialize_all(conn: &Connection, root: &Path, limit: Option<i64>) -> Result<Vec<PathBuf>> {
    let r = Renderer::new(conn, root)?;
    queries::projects(conn, None)?.iter().map(|p| r.project(p, limit)).collect()
}

pub fn materialize_space(conn: &Connection, root: &Path, slug: &str) -> Result<PathBuf> {
    Renderer::new(conn, root)?.space(slug)
}

fn group_by_status(all: &[Task]) -> HashMap<&str, Vec<&Task>> {
    let mut by_status: HashMap<&str, Vec<&Task>> = HashMap::new();
    for t in all {
        by_status.entry(t.status.as_str()).or_default().push(t);
    }
    by_status
}

/// `top_n < 0` means unlimited.
fn truncate<'t>(tasks: &'t [&'t Task], top_n: i64) -> &'t [&'t Task] {
    if top_n < 0 { tasks } else { &tasks[..(top_n as usize).min(tasks.len())] }
}

/// `## <icon> Name` — the status icon leads the heading when the status has one.
fn heading_name(ctx: &TableCtx, status: &Status) -> String {
    let icon = ctx.heading_icon(&status.slug);
    if icon.is_empty() { status.name.clone() } else { format!("{icon} {}", status.name) }
}

fn push_heading(out: &mut String, ctx: &TableCtx, status: &Status, shown: usize, total: usize) {
    let name = heading_name(ctx, status);
    if total > shown {
        out.push_str(&format!("## {name} (top {shown} of {total})\n\n"));
    } else {
        out.push_str(&format!("## {name} ({total})\n\n"));
    }
    push_description(out, status);
}

/// Collapsed section: heading, count, and the list command instead of a table.
fn push_hidden(out: &mut String, ctx: &TableCtx, status: &Status, total: usize, flag: &str, owner: &str) {
    out.push_str(&format!("## {} ({total})\n\n", heading_name(ctx, status)));
    push_description(out, status);
    out.push_str(&format!("_`kdb tasks list {flag} {owner} -s {}`_\n\n", status.slug));
}

fn push_description(out: &mut String, status: &Status) {
    if let Some(desc) = status.description.as_deref().map(str::trim)
        && !desc.is_empty()
    {
        out.push_str(&format!("_{desc}_\n\n"));
    }
}

/// Relative prefix from `<space>/.tasks/` to `<project>/.tasks/`, with a
/// trailing slash; empty when they coincide.
fn relative_tasks_prefix(space_path: &str, project_path: &str) -> String {
    let from: Vec<&str> = space_path.split('/').chain([TASKS_DIR]).filter(|s| !s.is_empty()).collect();
    let to: Vec<&str> = project_path.split('/').chain([TASKS_DIR]).filter(|s| !s.is_empty()).collect();
    let common = from.iter().zip(&to).take_while(|(a, b)| a == b).count();
    let mut parts: Vec<&str> = vec![".."; from.len() - common];
    parts.extend_from_slice(&to[common..]);
    if parts.is_empty() { String::new() } else { format!("{}/", parts.join("/")) }
}

/// Delete `T-NNNN.md` files not in `expected`; anything else is left alone.
fn clean_stale_task_files(dir: &Path, expected: &HashSet<String>) -> Result<()> {
    let Ok(entries) = fs::read_dir(dir) else { return Ok(()) };
    for entry in entries {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().into_owned();
        let Some(stem) = name.strip_prefix("T-").and_then(|s| s.strip_suffix(".md")) else { continue };
        if stem.is_empty() || !stem.bytes().all(|b| b.is_ascii_digit()) || expected.contains(&name) {
            continue;
        }
        fs::remove_file(entry.path())
            .with_context(|| format!("failed to remove stale task file {}", entry.path().display()))?;
    }
    Ok(())
}
