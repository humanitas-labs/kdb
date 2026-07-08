//! CLI command implementations.
//!
//! Each public function corresponds to a subcommand of the `kdb` binary:
//! `init`, `check`, `outline`, `tree`, `symbols`, `refs`, `deps`, `graph`, `fmt`, and `lsp`.

use anyhow::{Context, Result, bail};
use serde_json;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use crate::cycles;
use crate::db;
use crate::fmt;
use crate::index::{self, VaultIndex, WorkspaceIndex, deps as md_deps, refs};
use crate::labels;
use crate::lang::CodeLanguage;
use crate::materialize;
use crate::projects;
use crate::render;
use crate::search;
use crate::spaces;
use crate::statuses;
use crate::symbols;
use crate::tasks;
use crate::tree;
use crate::update;
use crate::workspace::{self, WorkspaceContext};

use rusqlite::Connection;

// -----------------------------------------------------------
// projects/kdb/src/cmd.rs
//
// pub struct CmdContext                                  L108
//   pub fn from_path()                                   L119
//   pub fn build_index()                                 L129
//   pub fn build_workspace_index()                       L134
//   pub fn rel_path()                                    L142
// pub fn init()                                          L169
// pub fn root()                                          L224
// pub fn check()                                         L234
// pub fn tree()                                          L251
// pub fn symbols()                                       L299
// pub fn refs()                                          L360
// pub fn deps()                                          L431
// pub fn graph()                                         L466
// pub fn render()                                        L481
// pub fn format()                                        L545
// pub fn update()                                        L586
// pub fn projects_list()                                 L593
// pub fn projects_add()                                  L612
// pub fn projects_edit()                                 L645
// pub fn projects_show()                                 L679
// fn resolve_space_id()                                  L700
// fn ensure_space_exists()                               L707
// pub fn spaces_list()                                   L712
// pub fn spaces_add()                                    L728
// pub fn spaces_edit()                                   L752
// pub fn spaces_show()                                   L778
// fn materialize_for()                                   L805
// fn resolve_project()                                   L817
// fn resolve_cycle()                                     L835
// fn resolve_parent()                                    L849
// fn parse_statuses()                                    L862
// pub fn tasks_list()                                    L900
// pub fn tasks_add()                                     L956
// fn resolve_add_position()                             L1037
// pub fn tasks_edit()                                   L1064
// pub fn tasks_view()                                   L1114
// struct TaskViewOutput                                 L1125
// pub fn tasks_move()                                   L1145
// pub fn tasks_delete()                                 L1184
// pub fn tasks_restore()                                L1203
// pub fn tasks_purge()                                  L1214
// pub fn cycles_list()                                  L1254
// pub fn cycles_add()                                   L1268
// pub fn cycles_edit()                                  L1296
// pub fn cycles_show()                                  L1321
// pub fn labels_list()                                  L1337
// pub fn labels_add()                                   L1351
// pub fn labels_edit()                                  L1366
// pub fn labels_show()                                  L1381
// pub fn tasks_label_add()                              L1398
// pub fn tasks_label_rm()                               L1417
// pub fn statuses_list()                                L1436
// pub fn statuses_add()                                 L1453
// fn resolve_add_flag()                                 L1484
// pub fn statuses_edit()                                L1504
// fn resolve_edit_flag()                                L1537
// pub fn statuses_rm()                                  L1573
// pub fn statuses_show()                                L1582
// pub fn tasks_set_status()                             L1598
// pub fn search()                                       L1609
// fn print_line_context()                               L1694
// pub fn index()                                        L1717
// pub fn collection_add()                               L1732
// pub fn collection_list()                              L1749
// mod tests                                             L1764
// fn setup()                                            L1769
// fn open_selector_resolves_to_non_closed_statuses()    L1777
// fn all_selector_means_no_filter()                     L1786
// fn unknown_status_is_rejected()                       L1792
// fn open_default_survives_seeded_status_removal()      L1798
// -----------------------------------------------------------

/// CLI command context: resolved start path + workspace state.
pub struct CmdContext {
    /// Resolved absolute start path (from CLI arg or cwd).
    pub start: PathBuf,
    /// Discovered workspace context (root, ignore patterns, ignore set).
    pub workspace: WorkspaceContext,
}

impl CmdContext {
    /// Resolve a start path and discover the workspace root.
    ///
    /// When `path` is `None`, falls back to the current working directory.
    pub fn from_path(path: Option<&Path>) -> Result<Self> {
        let start = match path {
            Some(p) => workspace::root::make_absolute(p)?,
            None => env::current_dir().context("failed to read current directory")?,
        };
        let workspace = WorkspaceContext::discover(&start)?;
        Ok(Self { start, workspace })
    }

    /// Build a [`VaultIndex`] (markdown only) using the workspace's ignore patterns.
    pub fn build_index(&self) -> Result<VaultIndex> {
        VaultIndex::build_with_ignores(&self.workspace.root, &self.workspace.ignore_patterns)
    }

    /// Build a [`WorkspaceIndex`] (vault + code) using the workspace's ignore patterns.
    pub fn build_workspace_index(&self) -> Result<WorkspaceIndex> {
        WorkspaceIndex::build_with_ignores(&self.workspace.root, &self.workspace.ignore_patterns)
    }

    /// Canonicalize an absolute path and return its root-relative form.
    ///
    /// Performs `canonicalize` → `strip_prefix(root)` → `normalize_rel_path`,
    /// producing a clean relative path suitable for index lookups.
    pub fn rel_path(&self, abs: &Path) -> Result<PathBuf> {
        let canonical = abs
            .canonicalize()
            .with_context(|| format!("failed to canonicalize {}", abs.display()))?;
        let root = &self.workspace.root;
        canonical
            .strip_prefix(root)
            .with_context(|| {
                format!(
                    "path {} is not inside kdb root {}",
                    canonical.display(),
                    root.display()
                )
            })
            .and_then(|rel| {
                workspace::paths::normalize_rel_path(rel).with_context(|| {
                    format!(
                        "path {} resolves outside kdb root {}",
                        canonical.display(),
                        root.display()
                    )
                })
            })
    }
}

/// Initialize a kdb workspace by creating `.kdb/config.toml`.
pub fn init(path: Option<PathBuf>) -> Result<()> {
    let start = match path {
        Some(path) => workspace::root::make_absolute(&path)?,
        None => env::current_dir().context("failed to read current directory")?,
    };

    if !start.exists() {
        bail!("path does not exist: {}", start.display());
    }

    if !start.is_dir() {
        bail!("init path must be a directory: {}", start.display());
    }

    let root = start
        .canonicalize()
        .with_context(|| format!("failed to canonicalize {}", start.display()))?;

    let marker_dir = root.join(workspace::root::ROOT_MARKER);
    if marker_dir.exists() {
        bail!(
            "{} already exists in {}",
            workspace::root::ROOT_MARKER,
            root.display()
        );
    }

    fs::create_dir_all(&marker_dir)
        .with_context(|| format!("failed to create {}", marker_dir.display()))?;

    let workspace_name = root
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("kdb")
        .replace('"', "\\\"");

    let config = workspace::root::config_path(&root);
    let default_config = format!("[workspace]\nname = \"{workspace_name}\"\n");
    fs::write(&config, default_config)
        .with_context(|| format!("failed to write {}", config.display()))?;

    let ignore_path = marker_dir.join("ignore");
    fs::write(&ignore_path, workspace::ignore::DEFAULT_IGNORE)
        .with_context(|| format!("failed to write {}", ignore_path.display()))?;

    db::open(&root)
        .with_context(|| format!("failed to initialize {}", db::db_path(&root).display()))?;

    println!("initialized kdb workspace at {}", root.display());

    Ok(())
}

/// Print the absolute path of the workspace root discovered from the current
/// working directory. Errors if no `.kdb/` marker is found in any ancestor.
pub fn root() -> Result<()> {
    let ctx = CmdContext::from_path(None)?;
    println!("{}", ctx.workspace.root.display());
    Ok(())
}

/// Validate all links in the vault and report broken references and orphan files.
///
/// Returns `Ok(true)` if any issues were found (caller should exit with code 1),
/// or `Ok(false)` if the vault is clean.
pub fn check(path: Option<PathBuf>, list_orphans: bool) -> Result<bool> {
    let has_scope = path.is_some();
    let ctx = CmdContext::from_path(path.as_deref())?;
    let index = ctx.build_index()?;
    let mut report = index.check();

    if has_scope {
        let scope_rel = ctx.rel_path(&ctx.start)?;
        let scope_is_dir = ctx.start.is_dir();
        report = report.scoped_to(&scope_rel, scope_is_dir);
    }

    report.print(list_orphans);
    Ok(report.has_errors())
}

/// Print a filtered workspace tree for a path under the current kdb root.
pub fn tree(
    path: Option<PathBuf>,
    level: Option<usize>,
    as_json: bool,
    all: bool,
    dirs_only: bool,
    full_path: bool,
    ignore: Vec<String>,
    pattern: Vec<String>,
) -> Result<()> {
    let has_explicit_path = path.is_some();
    let ctx = CmdContext::from_path(path.as_deref())?;

    if !ctx.start.exists() {
        bail!("path does not exist: {}", ctx.start.display());
    }

    let tree_start = if has_explicit_path {
        ctx.start.clone()
    } else {
        ctx.workspace.root.clone()
    };

    let tree = tree::build_tree(
        &ctx.workspace.root,
        &tree_start,
        &ctx.workspace.ignore_patterns,
        tree::TreeOptions {
            max_depth: level,
            show_hidden: all,
            dirs_only,
            full_paths: full_path,
            ignore_patterns: ignore,
            include_patterns: pattern,
        },
    )?;
    if as_json {
        let output =
            serde_json::to_string_pretty(&tree).context("failed to serialize tree as JSON")?;
        println!("{output}");
    } else {
        println!("{}", tree::render_text(&tree));
    }

    Ok(())
}

/// Print symbols for one or more files and/or directories.
pub fn symbols(
    paths: Vec<PathBuf>,
    selectors: Vec<String>,
    as_json: bool,
    public_only: bool,
) -> Result<()> {
    assert!(!paths.is_empty(), "at least one path is required");

    let ctx = CmdContext::from_path(Some(&paths[0]))?;
    let files = symbols::query::expand_paths(&ctx.workspace, &paths)?;
    assert!(!files.is_empty(), "no supported files found in given paths");

    let multi = files.len() > 1;
    if multi && !selectors.is_empty() {
        bail!(
            "-s/--symbol requires a single definition file, got {} files",
            files.len()
        );
    }

    if selectors.is_empty() {
        let mut all_rows: Vec<(PathBuf, Vec<symbols::display::SymbolRow>)> = Vec::new();
        for (abs, rel) in &files {
            let mut rows = symbols::query::collect_rows(abs, rel)?;
            if public_only {
                rows.retain(|row| row.is_public);
            }
            all_rows.push((rel.clone(), rows));
        }

        if as_json {
            let flat: Vec<_> = all_rows.iter().flat_map(|(_, rows)| rows).collect();
            let output = serde_json::to_string_pretty(&flat)
                .context("failed to serialize symbols as JSON")?;
            println!("{output}");
        } else if multi {
            symbols::display::print_multi_text(&all_rows);
        } else {
            symbols::display::print_text(&all_rows[0].1);
        }
    } else {
        let selector_strs: Vec<&str> = selectors.iter().map(String::as_str).collect();
        let mut all_rows = Vec::new();
        for (abs, rel) in &files {
            let rows = symbols::query::collect_body_rows(abs, rel, &selector_strs, public_only)?;
            all_rows.extend(rows);
        }

        if as_json {
            let output = serde_json::to_string_pretty(&all_rows)
                .context("failed to serialize symbol bodies as JSON")?;
            println!("{output}");
        } else {
            symbols::display::print_bodies_text(&all_rows);
        }
    }

    Ok(())
}

/// Find inbound markdown references or code symbol references.
pub fn refs(
    target: String,
    symbol: Option<String>,
    context_lines: Option<usize>,
    as_json: bool,
    count_only: bool,
    files_only: bool,
) -> Result<()> {
    let ctx = CmdContext::from_path(None)?;

    if let Some(symbol_name) = symbol {
        let index = WorkspaceIndex::build_for_target(
            &ctx.workspace.root,
            &ctx.workspace.ignore_patterns,
            &target,
        )?;
        let inbound =
            refs::collect_symbol_refs(&index.code, &ctx.workspace.root, &target, &symbol_name)?;

        if count_only {
            println!("{}", inbound.len());
            return Ok(());
        }

        if files_only {
            refs::print_symbol_refs_files(&inbound);
            return Ok(());
        }

        if as_json {
            let output = serde_json::to_string_pretty(&inbound)
                .context("failed to serialize symbol refs as JSON")?;
            println!("{output}");
        } else {
            let options = refs::SymbolRefRenderOptions::new(context_lines.unwrap_or(0));
            refs::print_symbol_refs_text(&ctx.workspace.root, &inbound, options)?;
        }

        return Ok(());
    }

    if context_lines.is_some() {
        bail!("--context is currently supported only with --symbol");
    }

    let target = index::refs::parse_target(&target)?;
    let index = ctx.build_index()?;
    let inbound = refs::collect_inbound(&index, &ctx.workspace.root, target)?;

    if count_only {
        println!("{}", inbound.len());
        return Ok(());
    }

    if files_only {
        refs::print_files(&inbound);
        return Ok(());
    }

    if as_json {
        let output =
            serde_json::to_string_pretty(&inbound).context("failed to serialize refs as JSON")?;
        println!("{output}");
    } else {
        refs::print_text(&inbound);
    }

    Ok(())
}

/// List outbound dependencies for a markdown or supported code file.
pub fn deps(target: String, as_json: bool) -> Result<()> {
    let ctx = CmdContext::from_path(None)?;
    let source_file = index::resolve_file_target(&ctx.workspace.root, &target)?;
    let is_markdown = source_file
        .extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| ext.eq_ignore_ascii_case("md"));

    if !is_markdown && CodeLanguage::from_path(&source_file).is_none() {
        bail!(
            "deps is not supported for file type: {}",
            source_file.display()
        );
    }

    let pi = ctx.build_workspace_index()?;

    let outbound = if is_markdown {
        md_deps::collect_outbound(&pi.vault, &source_file)?
    } else {
        md_deps::collect_code_outbound(&pi.code, &source_file)?
    };

    if as_json {
        let output =
            serde_json::to_string_pretty(&outbound).context("failed to serialize deps as JSON")?;
        println!("{output}");
    } else {
        md_deps::print_text(&outbound);
    }

    Ok(())
}

/// Stub for `kdb graph` until graph rendering lands.
pub fn graph(path: Option<PathBuf>) -> Result<()> {
    let requested = path
        .as_ref()
        .map(|value| value.display().to_string())
        .unwrap_or_else(|| "<root>".to_string());
    bail!(
        "`kdb graph` is not implemented yet (path: {requested}). See .issues/iss-0021-graph-command.md"
    )
}

/// Resolve markdown includes, or materialize per-project task files.
///
/// With `file`: resolve `![[]]` embeds to stdout. With `--project` or
/// `--all`: write materialized `index.md` + per-task files for the
/// matching projects.
pub fn render(
    file: Option<PathBuf>,
    project: Option<String>,
    space: Option<String>,
    all: bool,
    limit: Option<i64>,
) -> Result<()> {
    if all || project.is_some() || space.is_some() {
        if file.is_some() {
            bail!("--project/--space/--all cannot be combined with a file argument");
        }
        let selectors = [all, project.is_some(), space.is_some()]
            .iter()
            .filter(|b| **b)
            .count();
        if selectors > 1 {
            bail!("--project, --space, and --all are mutually exclusive");
        }
        let ctx = CmdContext::from_path(None)?;
        let conn = db::open(&ctx.workspace.root)?;
        let written = if all {
            materialize::materialize_all(&conn, &ctx.workspace.root, limit)?
        } else if let Some(slug) = space {
            vec![materialize::materialize_space(
                &conn,
                &ctx.workspace.root,
                &slug,
            )?]
        } else {
            let slug = project.expect("project checked above");
            vec![materialize::materialize_project(
                &conn,
                &ctx.workspace.root,
                &slug,
                limit,
            )?]
        };
        for p in &written {
            println!("wrote {}", p.display());
        }
        return Ok(());
    }
    if limit.is_some() {
        bail!("--limit only applies with --project/--all");
    }

    let file = file.context(
        "missing file argument — pass a markdown file, \
         --project <slug>, or --all",
    )?;
    let ctx = CmdContext::from_path(Some(&file))?;
    let rel_path = ctx.rel_path(&ctx.start)?;

    let output = render::render_file(&ctx.workspace.root, &rel_path)
        .with_context(|| format!("failed to render {}", rel_path.display()))?;

    print!("{output}");
    Ok(())
}

/// Generate or update code index headers for supported code files.
///
/// Walks the workspace root and rewrites Rust, TypeScript/JavaScript, Python,
/// and Go files with a managed index block at the top of each file.
pub fn format(path: Option<PathBuf>, force: bool) -> Result<()> {
    let has_explicit_path = path.is_some();
    let ctx = CmdContext::from_path(path.as_deref())?;

    if !ctx.start.exists() {
        bail!("path does not exist: {}", ctx.start.display());
    }

    let fmt_target = if has_explicit_path {
        ctx.start
    } else {
        ctx.workspace.root.clone()
    };
    let report = fmt::format_path(
        &ctx.workspace.root,
        &fmt_target,
        &ctx.workspace.ignore_patterns,
        force,
    )?;
    println!(
        "kdb fmt: updated {} of {} files",
        report.updated_files, report.scanned_files
    );

    if !report.warnings.is_empty() {
        eprintln!("kdb fmt: {} warning(s)", report.warnings.len());
        for warning in &report.warnings {
            eprintln!(
                "warning: {} ({})",
                warning.message,
                warning.rel_path.display()
            );
        }
    }

    Ok(())
}

/// Check for updates and optionally self-update the binary.
///
/// When `check_only` is true, prints version info without replacing the binary.
pub fn update(check_only: bool) -> Result<()> {
    let updater = update::Updater::new();
    updater.run(check_only)
}

/// List projects. Archived projects are hidden unless `include_archived`.
/// When `space` is given, only projects in that space are shown.
pub fn projects_list(include_archived: bool, space: Option<String>, as_json: bool) -> Result<()> {
    let ctx = CmdContext::from_path(None)?;
    let conn = db::open(&ctx.workspace.root)?;
    if let Some(slug) = space.as_deref() {
        ensure_space_exists(&conn, slug)?;
    }
    let rows = projects::list(&conn, include_archived, space.as_deref())?;

    if as_json {
        let output =
            serde_json::to_string_pretty(&rows).context("failed to serialize projects as JSON")?;
        println!("{output}");
    } else {
        print!("{}", projects::render_list(&rows));
    }
    Ok(())
}

/// Insert a new project.
pub fn projects_add(
    slug: String,
    alias: String,
    path: String,
    name: Option<String>,
    description: Option<String>,
    space: Option<String>,
) -> Result<()> {
    let ctx = CmdContext::from_path(None)?;
    let conn = db::open(&ctx.workspace.root)?;
    let space_id = match space.as_deref() {
        Some(slug) if !slug.is_empty() => Some(resolve_space_id(&conn, slug)?),
        _ => None,
    };
    let created = projects::add(
        &conn,
        projects::AddArgs {
            slug: &slug,
            alias: &alias,
            name: name.as_deref(),
            path: &path,
            description: description.as_deref(),
            space_id,
        },
    )?;
    println!(
        "added project {} [{}] ({})",
        created.slug, created.alias, created.path
    );
    Ok(())
}

/// Update mutable fields on an existing project.
pub fn projects_edit(
    slug: String,
    alias: Option<String>,
    name: Option<String>,
    path: Option<String>,
    status: Option<String>,
    description: Option<String>,
    space: Option<String>,
) -> Result<()> {
    let ctx = CmdContext::from_path(None)?;
    let conn = db::open(&ctx.workspace.root)?;
    // None: leave unchanged. Some(""): detach. Some(slug): assign.
    let space_id = match space.as_deref() {
        None => None,
        Some(s) if s.is_empty() => Some(None),
        Some(s) => Some(Some(resolve_space_id(&conn, s)?)),
    };
    let updated = projects::edit(
        &conn,
        &slug,
        projects::EditArgs {
            alias: alias.as_deref(),
            name: name.as_deref(),
            path: path.as_deref(),
            status: status.as_deref(),
            description: description.as_deref(),
            space_id,
        },
    )?;
    println!("updated project {}", updated.slug);
    Ok(())
}

/// Show a single project.
pub fn projects_show(slug: String, as_json: bool) -> Result<()> {
    let ctx = CmdContext::from_path(None)?;
    let conn = db::open(&ctx.workspace.root)?;
    let project = projects::get_by_slug(&conn, &slug)?
        .with_context(|| format!("project not found: {slug}"))?;

    if as_json {
        let output = serde_json::to_string_pretty(&project)
            .context("failed to serialize project as JSON")?;
        println!("{output}");
    } else {
        print!("{}", projects::render_show(&project));
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Spaces
// ---------------------------------------------------------------------------

/// Resolve a space slug to its numeric id, erroring if it doesn't exist.
fn resolve_space_id(conn: &Connection, slug: &str) -> Result<i64> {
    Ok(spaces::get_by_slug(conn, slug)?
        .with_context(|| format!("space not found: {slug}"))?
        .id)
}

/// Verify a space exists (used by filters that don't need the id).
fn ensure_space_exists(conn: &Connection, slug: &str) -> Result<()> {
    resolve_space_id(conn, slug).map(|_| ())
}

/// List spaces. Archived spaces are hidden unless `include_archived`.
pub fn spaces_list(include_archived: bool, as_json: bool) -> Result<()> {
    let ctx = CmdContext::from_path(None)?;
    let conn = db::open(&ctx.workspace.root)?;
    let rows = spaces::list(&conn, include_archived)?;

    if as_json {
        let output =
            serde_json::to_string_pretty(&rows).context("failed to serialize spaces as JSON")?;
        println!("{output}");
    } else {
        print!("{}", spaces::render_list(&rows));
    }
    Ok(())
}

/// Insert a new space.
pub fn spaces_add(
    slug: String,
    alias: String,
    name: Option<String>,
    path: Option<String>,
    description: Option<String>,
) -> Result<()> {
    let ctx = CmdContext::from_path(None)?;
    let conn = db::open(&ctx.workspace.root)?;
    let created = spaces::add(
        &conn,
        spaces::AddArgs {
            slug: &slug,
            name: name.as_deref(),
            alias: &alias,
            path: path.as_deref(),
            description: description.as_deref(),
        },
    )?;
    println!("added space {}", created.slug);
    Ok(())
}

/// Update mutable fields on an existing space.
pub fn spaces_edit(
    slug: String,
    name: Option<String>,
    alias: Option<String>,
    path: Option<String>,
    status: Option<String>,
    description: Option<String>,
) -> Result<()> {
    let ctx = CmdContext::from_path(None)?;
    let conn = db::open(&ctx.workspace.root)?;
    let updated = spaces::edit(
        &conn,
        &slug,
        spaces::EditArgs {
            name: name.as_deref(),
            alias: alias.as_deref(),
            path: path.as_deref(),
            status: status.as_deref(),
            description: description.as_deref(),
        },
    )?;
    println!("updated space {}", updated.slug);
    Ok(())
}

/// Show a single space and the projects that belong to it.
pub fn spaces_show(slug: String, as_json: bool) -> Result<()> {
    let ctx = CmdContext::from_path(None)?;
    let conn = db::open(&ctx.workspace.root)?;
    let space =
        spaces::get_by_slug(&conn, &slug)?.with_context(|| format!("space not found: {slug}"))?;
    let members = projects::list(&conn, true, Some(&slug))?;

    if as_json {
        let payload = serde_json::json!({ "space": space, "projects": members });
        let output =
            serde_json::to_string_pretty(&payload).context("failed to serialize space as JSON")?;
        println!("{output}");
    } else {
        print!("{}", spaces::render_show(&space));
        println!("\nprojects:");
        if members.is_empty() {
            println!("  (none)");
        } else {
            print!("{}", projects::render_list(&members));
        }
    }
    Ok(())
}

/// Re-materialize the board that owns `view` — the space board for a
/// space-native task, the project board otherwise. `project_slug` doubles
/// as the owner slug (space slug for space-native tasks).
fn materialize_for(conn: &Connection, root: &Path, view: &tasks::TaskView) -> Result<()> {
    if view.task.space_id.is_some() {
        materialize::materialize_space(conn, root, &view.project_slug)?;
    } else {
        materialize::materialize_project(conn, root, &view.project_slug, None)?;
    }
    Ok(())
}

/// Resolve the project to operate on. Prefers the explicit `--project`
/// flag; otherwise falls back to the project registered at the current
/// working directory.
fn resolve_project(
    conn: &Connection,
    root: &Path,
    explicit: Option<&str>,
) -> Result<projects::Project> {
    if let Some(slug) = explicit {
        return projects::get_by_slug(conn, slug)?
            .with_context(|| format!("project not found: {slug}"));
    }
    let cwd = env::current_dir().context("failed to read current directory")?;
    let cwd = cwd.canonicalize().unwrap_or(cwd);
    projects::resolve_active(conn, root, &cwd)?.context(
        "no project for current directory — pass -P/--project or \
         register one with `kdb projects add`",
    )
}

/// Resolve an optional cycle key to its numeric id.
fn resolve_cycle(conn: &Connection, key: Option<&str>) -> Result<Option<Option<i64>>> {
    match key {
        None => Ok(None),
        Some(k) if k.is_empty() => Ok(Some(None)),
        Some(k) => {
            let id: i64 = conn
                .query_row("SELECT id FROM cycles WHERE key = ?", [k], |row| row.get(0))
                .with_context(|| format!("cycle not found: {k}"))?;
            Ok(Some(Some(id)))
        }
    }
}

/// Resolve an optional parent task id (external form `slug-seq`).
fn resolve_parent(conn: &Connection, id: Option<&str>) -> Result<Option<Option<i64>>> {
    match id {
        None => Ok(None),
        Some(s) if s.is_empty() => Ok(Some(None)),
        Some(s) => {
            let parsed = tasks::TaskId::parse(s)?;
            let view = tasks::get(conn, &parsed)?
                .with_context(|| format!("parent task not found: {s}"))?;
            Ok(Some(Some(view.task.id)))
        }
    }
}

fn parse_statuses(conn: &Connection, s: &str) -> Result<Option<Vec<String>>> {
    if s == "all" {
        return Ok(None);
    }
    let known = statuses::list(conn, statuses::Kind::Task)?;
    // "open" is a runtime-resolved selector: every task status that is not
    // closed (is_closed = false). Resolving against the live status table
    // keeps the default correct after seeded statuses are renamed or
    // removed — a hardcoded slug list does not (see the `cycle` removal bug).
    if s == "open" {
        let open: Vec<String> = known
            .into_iter()
            .filter(|st| !st.flag)
            .map(|st| st.slug)
            .collect();
        return Ok(Some(open));
    }
    let parts: Vec<String> = s
        .split(',')
        .map(|p| p.trim().to_string())
        .filter(|p| !p.is_empty())
        .collect();
    for p in &parts {
        if !known.iter().any(|k| k.slug == *p) {
            bail!(
                "invalid status '{p}' (expected {} or 'all'/'open')",
                known
                    .iter()
                    .map(|k| k.slug.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            );
        }
    }
    Ok(Some(parts))
}

/// List tasks.
pub fn tasks_list(
    status: String,
    project: Option<String>,
    space: Option<String>,
    cycle: Option<String>,
    priority: Option<i64>,
    limit: Option<i64>,
    include_children: bool,
    as_json: bool,
) -> Result<()> {
    let ctx = CmdContext::from_path(None)?;
    let conn = db::open(&ctx.workspace.root)?;

    let statuses_owned = parse_statuses(&conn, &status)?;
    let statuses_refs: Option<Vec<&str>> = statuses_owned
        .as_ref()
        .map(|v| v.iter().map(|s| s.as_str()).collect());

    let space_slug = match space.as_deref() {
        Some("all") | Some("") | None => None,
        Some(s) => {
            ensure_space_exists(&conn, s)?;
            Some(s.to_string())
        }
    };

    // A space filter provides a cross-project view, so it suppresses the
    // active-project default; an explicit -P still narrows within the space.
    let project_slug = match project.as_deref() {
        Some("all") | Some("") | None => None,
        Some(s) => Some(s.to_string()),
    };

    let filters = tasks::ListFilters {
        statuses: statuses_refs.as_deref(),
        project_slug: project_slug.as_deref(),
        space_slug: space_slug.as_deref(),
        space_native_slug: None,
        cycle_key: cycle.as_deref(),
        priority,
        limit,
        top_level_only: !include_children,
    };
    let rows = tasks::list(&conn, filters)?;

    if as_json {
        let output =
            serde_json::to_string_pretty(&rows).context("failed to serialize tasks as JSON")?;
        println!("{output}");
    } else {
        print!("{}", tasks::render_list(&rows));
    }
    Ok(())
}

/// Add a new task.
pub fn tasks_add(
    title: String,
    project: Option<String>,
    space: Option<String>,
    body: Option<String>,
    priority: Option<i64>,
    cycle: Option<String>,
    parent: Option<String>,
    before: Option<String>,
    after: Option<String>,
) -> Result<()> {
    if before.is_some() && after.is_some() {
        bail!("--before and --after are mutually exclusive");
    }
    if project.is_some() && space.is_some() {
        bail!("-P/--project and -S/--space are mutually exclusive (a task has one owner)");
    }
    let ctx = CmdContext::from_path(None)?;
    let mut conn = db::open(&ctx.workspace.root)?;

    // Owner is a project or a space. -S selects a space; otherwise fall back
    // to the active project.
    let (owner_project, owner_space) = match space.as_deref() {
        Some(slug) => {
            let sp = spaces::get_by_slug(&conn, slug)?
                .with_context(|| format!("space not found: {slug}"))?;
            if sp.alias.is_none() {
                bail!(
                    "space {slug} has no alias — set one with `kdb spaces edit {slug} --alias <ABC>`"
                );
            }
            (None, Some(sp.id))
        }
        None => {
            let proj = resolve_project(&conn, &ctx.workspace.root, project.as_deref())?;
            (Some(proj.id), None)
        }
    };

    let cycle_id = resolve_cycle(&conn, cycle.as_deref())?.flatten();
    let parent_id_explicit = resolve_parent(&conn, parent.as_deref())?;

    let position = resolve_add_position(
        &conn,
        owner_project,
        owner_space,
        before.as_deref(),
        after.as_deref(),
    )?;
    // Inherit parent from the sibling we're anchoring to when --parent not given explicitly.
    let parent_id = match (parent_id_explicit, &position) {
        (Some(explicit), _) => explicit,
        (None, Some((sibling, _))) => sibling.task.parent_id,
        (None, None) => None,
    };

    let order_key = position
        .as_ref()
        .map(|(sibling, side)| tasks::order_key_adjacent(&conn, sibling, *side))
        .transpose()?;

    let view = tasks::add(
        &mut conn,
        tasks::AddArgs {
            project_id: owner_project,
            space_id: owner_space,
            title: &title,
            body: body.as_deref(),
            priority,
            cycle_id,
            parent_id,
            seq: None,
            status: None,
            order: order_key.as_deref(),
        },
    )?;
    materialize_for(&conn, &ctx.workspace.root, &view)?;
    println!("added task {}", view.external_id());
    Ok(())
}

fn resolve_add_position(
    conn: &rusqlite::Connection,
    owner_project: Option<i64>,
    owner_space: Option<i64>,
    before: Option<&str>,
    after: Option<&str>,
) -> Result<Option<(tasks::TaskView, tasks::Side)>> {
    let (raw, side) = match (before, after) {
        (Some(b), None) => (b, tasks::Side::Before),
        (None, Some(a)) => (a, tasks::Side::After),
        (None, None) => return Ok(None),
        (Some(_), Some(_)) => unreachable!("caller validates mutual exclusion"),
    };
    let parsed = tasks::TaskId::parse(raw)?;
    let view =
        tasks::get(conn, &parsed)?.with_context(|| format!("anchor task not found: {raw}"))?;
    if view.task.project_id != owner_project || view.task.space_id != owner_space {
        bail!(
            "anchor task {} belongs to {}, not the target owner",
            view.external_id(),
            view.project_slug
        );
    }
    Ok(Some((view, side)))
}

/// Edit an existing task.
pub fn tasks_edit(
    id: String,
    title: Option<String>,
    body: Option<String>,
    priority: Option<i64>,
    cycle: Option<String>,
    parent: Option<String>,
    status: Option<String>,
) -> Result<()> {
    let ctx = CmdContext::from_path(None)?;
    let mut conn = db::open(&ctx.workspace.root)?;
    let parsed = tasks::TaskId::parse(&id)?;

    let cycle_update = resolve_cycle(&conn, cycle.as_deref())?;
    let parent_update = resolve_parent(&conn, parent.as_deref())?;

    let has_core_update = title.is_some()
        || body.is_some()
        || priority.is_some()
        || cycle_update.is_some()
        || parent_update.is_some();
    if !has_core_update && status.is_none() {
        bail!("no fields to update");
    }

    let view = if has_core_update {
        tasks::edit(
            &mut conn,
            &parsed,
            tasks::EditArgs {
                title: title.as_deref(),
                body: body.as_deref(),
                priority,
                cycle_id: cycle_update,
                parent_id: parent_update,
            },
        )?
    } else {
        tasks::get(&conn, &parsed)?.with_context(|| format!("task not found: {id}"))?
    };
    let view = match status.as_deref() {
        Some(s) => tasks::set_status(&conn, &parsed, s)?,
        None => view,
    };
    materialize_for(&conn, &ctx.workspace.root, &view)?;
    println!("updated task {}", view.external_id());
    Ok(())
}

/// View a single task.
pub fn tasks_view(id: String, as_json: bool) -> Result<()> {
    let ctx = CmdContext::from_path(None)?;
    let conn = db::open(&ctx.workspace.root)?;
    let parsed = tasks::TaskId::parse(&id)?;
    let view = tasks::get(&conn, &parsed)?.with_context(|| format!("task not found: {id}"))?;
    let task_labels = labels::for_task(&conn, view.task.id)?;
    let children = tasks::children(&conn, view.task.id)?;
    let slugs: Vec<&str> = task_labels.iter().map(|l| l.slug.as_str()).collect();

    if as_json {
        #[derive(serde::Serialize)]
        struct TaskViewOutput<'a> {
            #[serde(flatten)]
            task: &'a tasks::TaskView,
            labels: &'a [labels::Label],
            children: &'a [tasks::ChildTask],
        }
        let output = serde_json::to_string_pretty(&TaskViewOutput {
            task: &view,
            labels: &task_labels,
            children: &children,
        })
        .context("failed to serialize task as JSON")?;
        println!("{output}");
    } else {
        print!("{}", tasks::render_show(&view, &slugs, &children));
    }
    Ok(())
}

/// Move a task to a new relative position within its sibling context.
pub fn tasks_move(
    id: String,
    before: Option<String>,
    after: Option<String>,
    top: bool,
    bottom: bool,
) -> Result<()> {
    let ctx = CmdContext::from_path(None)?;
    let conn = db::open(&ctx.workspace.root)?;
    let parsed = tasks::TaskId::parse(&id)?;

    let chosen = [before.is_some(), after.is_some(), top, bottom]
        .iter()
        .filter(|b| **b)
        .count();
    if chosen != 1 {
        bail!("exactly one of --before, --after, --top, --bottom must be given");
    }

    let before_id = before.as_deref().map(tasks::TaskId::parse).transpose()?;
    let after_id = after.as_deref().map(tasks::TaskId::parse).transpose()?;
    let target = if let Some(b) = before_id.as_ref() {
        tasks::MoveTarget::Before(b)
    } else if let Some(a) = after_id.as_ref() {
        tasks::MoveTarget::After(a)
    } else if top {
        tasks::MoveTarget::Top
    } else {
        tasks::MoveTarget::Bottom
    };

    let view = tasks::move_task(&conn, &parsed, target)?;
    materialize_for(&conn, &ctx.workspace.root, &view)?;
    println!("moved {} (order={})", view.external_id(), view.task.order);
    Ok(())
}

/// Delete a task. By default soft-deletes (sets `deleted_at`); pass
/// `hard = true` to permanently remove the row and its subtree.
pub fn tasks_delete(id: String, hard: bool) -> Result<()> {
    let ctx = CmdContext::from_path(None)?;
    let mut conn = db::open(&ctx.workspace.root)?;
    let parsed = tasks::TaskId::parse(&id)?;
    if hard {
        let existing = tasks::get_including_deleted(&conn, &parsed)?
            .with_context(|| format!("task not found: {}", parsed.render()))?;
        tasks::hard_delete(&mut conn, &parsed)?;
        materialize_for(&conn, &ctx.workspace.root, &existing)?;
        println!("hard-deleted {}", existing.external_id());
    } else {
        let view = tasks::soft_delete(&mut conn, &parsed)?;
        materialize_for(&conn, &ctx.workspace.root, &view)?;
        println!("deleted {}", view.external_id());
    }
    Ok(())
}

/// Restore a soft-deleted task and its subtree.
pub fn tasks_restore(id: String) -> Result<()> {
    let ctx = CmdContext::from_path(None)?;
    let mut conn = db::open(&ctx.workspace.root)?;
    let parsed = tasks::TaskId::parse(&id)?;
    let view = tasks::restore(&mut conn, &parsed)?;
    materialize_for(&conn, &ctx.workspace.root, &view)?;
    println!("restored {}", view.external_id());
    Ok(())
}

/// Permanently purge tasks matching filters.
pub fn tasks_purge(
    project: Option<String>,
    status: Option<String>,
    deleted: bool,
    dry_run: bool,
) -> Result<()> {
    let ctx = CmdContext::from_path(None)?;
    let mut conn = db::open(&ctx.workspace.root)?;
    let matches = tasks::purge(
        &mut conn,
        tasks::PurgeFilters {
            project_slug: project.as_deref(),
            status: status.as_deref(),
            deleted_only: deleted,
            dry_run,
        },
    )?;
    if matches.is_empty() {
        println!("(no tasks matched)");
        return Ok(());
    }
    let action = if dry_run { "would purge" } else { "purged" };
    println!("{action} {} tasks:", matches.len());
    for m in &matches {
        println!("  {}  {}", m.external_id(), m.task.title);
    }
    if !dry_run {
        // Re-materialize each distinct owner board once (project or space).
        let mut seen: std::collections::HashSet<(bool, &str)> = std::collections::HashSet::new();
        for m in &matches {
            let is_space = m.task.space_id.is_some();
            if seen.insert((is_space, m.project_slug.as_str())) {
                materialize_for(&conn, &ctx.workspace.root, m)?;
            }
        }
    }
    Ok(())
}

/// List cycles.
pub fn cycles_list(as_json: bool) -> Result<()> {
    let ctx = CmdContext::from_path(None)?;
    let conn = db::open(&ctx.workspace.root)?;
    let rows = cycles::list(&conn)?;
    if as_json {
        let output =
            serde_json::to_string_pretty(&rows).context("failed to serialize cycles as JSON")?;
        println!("{output}");
    } else {
        print!("{}", cycles::render_list(&rows));
    }
    Ok(())
}

pub fn cycles_add(
    key: String,
    start: String,
    end: String,
    description: Option<String>,
    status: Option<String>,
    path: Option<String>,
) -> Result<()> {
    let ctx = CmdContext::from_path(None)?;
    let conn = db::open(&ctx.workspace.root)?;
    let created = cycles::add(
        &conn,
        cycles::AddArgs {
            key: &key,
            start_date: &start,
            end_date: &end,
            description: description.as_deref(),
            status: status.as_deref(),
            path: path.as_deref(),
        },
    )?;
    println!(
        "added cycle {} ({} → {})",
        created.key, created.start_date, created.end_date
    );
    Ok(())
}

pub fn cycles_edit(
    key: String,
    start: Option<String>,
    end: Option<String>,
    description: Option<String>,
    status: Option<String>,
    path: Option<String>,
) -> Result<()> {
    let ctx = CmdContext::from_path(None)?;
    let conn = db::open(&ctx.workspace.root)?;
    let updated = cycles::edit(
        &conn,
        &key,
        cycles::EditArgs {
            start_date: start.as_deref(),
            end_date: end.as_deref(),
            description: description.as_deref(),
            status: status.as_deref(),
            path: path.as_deref(),
        },
    )?;
    println!("updated cycle {}", updated.key);
    Ok(())
}

pub fn cycles_show(key: String, as_json: bool) -> Result<()> {
    let ctx = CmdContext::from_path(None)?;
    let conn = db::open(&ctx.workspace.root)?;
    let cycle =
        cycles::get_by_key(&conn, &key)?.with_context(|| format!("cycle not found: {key}"))?;
    if as_json {
        let output =
            serde_json::to_string_pretty(&cycle).context("failed to serialize cycle as JSON")?;
        println!("{output}");
    } else {
        print!("{}", cycles::render_show(&cycle));
    }
    Ok(())
}

/// List labels.
pub fn labels_list(as_json: bool) -> Result<()> {
    let ctx = CmdContext::from_path(None)?;
    let conn = db::open(&ctx.workspace.root)?;
    let rows = labels::list(&conn)?;
    if as_json {
        let output =
            serde_json::to_string_pretty(&rows).context("failed to serialize labels as JSON")?;
        println!("{output}");
    } else {
        print!("{}", labels::render_list(&rows));
    }
    Ok(())
}

pub fn labels_add(slug: String, name: Option<String>, color: Option<String>) -> Result<()> {
    let ctx = CmdContext::from_path(None)?;
    let conn = db::open(&ctx.workspace.root)?;
    let created = labels::add(
        &conn,
        labels::AddArgs {
            slug: &slug,
            name: name.as_deref(),
            color: color.as_deref(),
        },
    )?;
    println!("added label {}", created.slug);
    Ok(())
}

pub fn labels_edit(slug: String, name: Option<String>, color: Option<String>) -> Result<()> {
    let ctx = CmdContext::from_path(None)?;
    let conn = db::open(&ctx.workspace.root)?;
    let updated = labels::edit(
        &conn,
        &slug,
        labels::EditArgs {
            name: name.as_deref(),
            color: color.as_deref(),
        },
    )?;
    println!("updated label {}", updated.slug);
    Ok(())
}

pub fn labels_show(slug: String, as_json: bool) -> Result<()> {
    let ctx = CmdContext::from_path(None)?;
    let conn = db::open(&ctx.workspace.root)?;
    let label =
        labels::get_by_slug(&conn, &slug)?.with_context(|| format!("label not found: {slug}"))?;
    if as_json {
        let output =
            serde_json::to_string_pretty(&label).context("failed to serialize label as JSON")?;
        println!("{output}");
    } else {
        print!("{}", labels::render_show(&label));
    }
    Ok(())
}

/// Attach one or more labels to a task. Unknown label slugs are
/// created on the fly.
pub fn tasks_label_add(id: String, label_slugs: Vec<String>) -> Result<()> {
    let ctx = CmdContext::from_path(None)?;
    let conn = db::open(&ctx.workspace.root)?;
    let parsed = tasks::TaskId::parse(&id)?;
    let view = tasks::get(&conn, &parsed)?.with_context(|| format!("task not found: {id}"))?;
    for slug in &label_slugs {
        let label = labels::upsert_by_slug(&conn, slug)?;
        labels::attach(&conn, view.task.id, label.id)?;
    }
    materialize_for(&conn, &ctx.workspace.root, &view)?;
    println!(
        "attached {} label(s) to {}",
        label_slugs.len(),
        view.external_id()
    );
    Ok(())
}

/// Detach one or more labels from a task.
pub fn tasks_label_rm(id: String, label_slugs: Vec<String>) -> Result<()> {
    let ctx = CmdContext::from_path(None)?;
    let conn = db::open(&ctx.workspace.root)?;
    let parsed = tasks::TaskId::parse(&id)?;
    let view = tasks::get(&conn, &parsed)?.with_context(|| format!("task not found: {id}"))?;
    let mut removed = 0usize;
    for slug in &label_slugs {
        if let Some(label) = labels::get_by_slug(&conn, slug)? {
            if labels::detach(&conn, view.task.id, label.id)? {
                removed += 1;
            }
        }
    }
    materialize_for(&conn, &ctx.workspace.root, &view)?;
    println!("detached {removed} label(s) from {}", view.external_id());
    Ok(())
}

/// List statuses for `kind`.
pub fn statuses_list(kind: statuses::Kind, as_json: bool) -> Result<()> {
    let ctx = CmdContext::from_path(None)?;
    let conn = db::open(&ctx.workspace.root)?;
    let rows = statuses::list(&conn, kind)?;
    if as_json {
        let output =
            serde_json::to_string_pretty(&rows).context("failed to serialize statuses as JSON")?;
        println!("{output}");
    } else {
        print!("{}", statuses::render_list(&rows, kind));
    }
    Ok(())
}

/// Add a new status. `closed` applies only when `kind == Task`; `archived`
/// only when `kind == Project`; passing the wrong one errors.
#[allow(clippy::too_many_arguments)]
pub fn statuses_add(
    slug: String,
    kind: statuses::Kind,
    name: Option<String>,
    description: Option<String>,
    color: Option<String>,
    closed: bool,
    archived: bool,
    order: Option<i64>,
    hidden: Option<bool>,
) -> Result<()> {
    let flag = resolve_add_flag(kind, closed, archived)?;
    let ctx = CmdContext::from_path(None)?;
    let conn = db::open(&ctx.workspace.root)?;
    let created = statuses::add(
        &conn,
        kind,
        statuses::AddArgs {
            slug: &slug,
            name: name.as_deref(),
            description: description.as_deref(),
            color: color.as_deref(),
            flag,
            sort_order: order,
            is_hidden: hidden.unwrap_or(false),
        },
    )?;
    println!("added status {}", created.slug);
    Ok(())
}

fn resolve_add_flag(kind: statuses::Kind, closed: bool, archived: bool) -> Result<bool> {
    match kind {
        statuses::Kind::Task => {
            if archived {
                bail!("--archived applies only to project statuses (use --closed with --tasks)");
            }
            Ok(closed)
        }
        statuses::Kind::Project => {
            if closed {
                bail!("--closed applies only to task statuses (use --archived with --projects)");
            }
            Ok(archived)
        }
    }
}

/// Edit an existing status. `closed`/`no_closed` apply only to task statuses;
/// `archived`/`no_archived` only to project statuses.
#[allow(clippy::too_many_arguments)]
pub fn statuses_edit(
    slug: String,
    kind: statuses::Kind,
    name: Option<String>,
    description: Option<String>,
    color: Option<String>,
    closed: bool,
    no_closed: bool,
    archived: bool,
    no_archived: bool,
    order: Option<i64>,
    hidden: Option<bool>,
) -> Result<()> {
    let flag = resolve_edit_flag(kind, closed, no_closed, archived, no_archived)?;
    let ctx = CmdContext::from_path(None)?;
    let conn = db::open(&ctx.workspace.root)?;
    let updated = statuses::edit(
        &conn,
        kind,
        &slug,
        statuses::EditArgs {
            name: name.as_deref(),
            description: description.as_deref(),
            color: color.as_deref(),
            flag,
            sort_order: order,
            is_hidden: hidden,
        },
    )?;
    println!("updated status {}", updated.slug);
    Ok(())
}

fn resolve_edit_flag(
    kind: statuses::Kind,
    closed: bool,
    no_closed: bool,
    archived: bool,
    no_archived: bool,
) -> Result<Option<bool>> {
    match kind {
        statuses::Kind::Task => {
            if archived || no_archived {
                bail!("--archived/--no-archived apply only to project statuses");
            }
            if closed {
                Ok(Some(true))
            } else if no_closed {
                Ok(Some(false))
            } else {
                Ok(None)
            }
        }
        statuses::Kind::Project => {
            if closed || no_closed {
                bail!("--closed/--no-closed apply only to task statuses");
            }
            if archived {
                Ok(Some(true))
            } else if no_archived {
                Ok(Some(false))
            } else {
                Ok(None)
            }
        }
    }
}

/// Remove a status. Fails if any row still references it.
pub fn statuses_rm(slug: String, kind: statuses::Kind) -> Result<()> {
    let ctx = CmdContext::from_path(None)?;
    let conn = db::open(&ctx.workspace.root)?;
    statuses::remove(&conn, kind, &slug)?;
    println!("removed status {slug}");
    Ok(())
}

/// Show a single status.
pub fn statuses_show(slug: String, kind: statuses::Kind, as_json: bool) -> Result<()> {
    let ctx = CmdContext::from_path(None)?;
    let conn = db::open(&ctx.workspace.root)?;
    let status = statuses::get(&conn, kind, &slug)?
        .with_context(|| format!("{} not found: {slug}", kind.table()))?;
    if as_json {
        let output =
            serde_json::to_string_pretty(&status).context("failed to serialize status as JSON")?;
        println!("{output}");
    } else {
        print!("{}", statuses::render_show(&status, kind));
    }
    Ok(())
}

/// Transition a task to a new status.
pub fn tasks_set_status(id: String, status: &str) -> Result<()> {
    let ctx = CmdContext::from_path(None)?;
    let conn = db::open(&ctx.workspace.root)?;
    let parsed = tasks::TaskId::parse(&id)?;
    let view = tasks::set_status(&conn, &parsed, status)?;
    materialize_for(&conn, &ctx.workspace.root, &view)?;
    println!("{} -> {}", view.external_id(), view.task.status);
    Ok(())
}

/// Full-text search the workspace. Syncs the index incrementally first.
pub fn search(
    query: String,
    ftype: search::FType,
    collection: Option<String>,
    path: Option<PathBuf>,
    context: Option<usize>,
    limit: i64,
) -> Result<()> {
    let ctx = CmdContext::from_path(None)?;
    let conn = db::open(&ctx.workspace.root)?;
    search::sync(&conn, &ctx.workspace.root, &ctx.workspace.ignore_set)?;

    // Scope by a registered collection OR an ad-hoc directory (clap keeps
    // them mutually exclusive). Both reduce to a workspace-relative path
    // prefix; normalize to a directory boundary so `foo` doesn't match
    // `foobar/`.
    let (prefix, scope_label) = match (collection.as_deref(), path.as_deref()) {
        (Some(name), _) => (
            Some(search::resolve_collection(&conn, name)?),
            format!("  ·  collection: {name}"),
        ),
        (_, Some(dir)) => {
            let abs = workspace::root::make_absolute(dir)?;
            let rel = abs.strip_prefix(&ctx.workspace.root).map_err(|_| {
                anyhow::anyhow!(
                    "--path must be inside the workspace ({})",
                    ctx.workspace.root.display()
                )
            })?;
            let rel = rel.to_string_lossy().to_string();
            (Some(rel.clone()), format!("  ·  path: {rel}"))
        }
        (None, None) => (None, String::new()),
    };
    let prefix = prefix.map(|p| {
        if p.is_empty() || p.ends_with('/') {
            p
        } else {
            format!("{p}/")
        }
    });

    let hits = search::query(&conn, &query, ftype, prefix.as_deref(), limit)?;

    let scope = match ftype {
        search::FType::Docs => "docs",
        search::FType::Code => "code",
        search::FType::All => "all",
    };
    if hits.is_empty() {
        println!("No matches for \"{query}\" in {scope}{scope_label}.");
        if ftype == search::FType::Docs {
            println!("Tip: add --ftype all to include code & config.");
        }
        return Ok(());
    }

    let plural = if hits.len() == 1 { "result" } else { "results" };
    let scoped = scope_label;
    println!(
        "{} {plural} for \"{query}\"  ·  ftype: {scope}{scoped}\n",
        hits.len()
    );
    let needles = search::terms(&query);
    for (i, hit) in hits.iter().enumerate() {
        let tag = if hit.kind == "code" { " [code]" } else { "" };
        println!("{:>3}. {}{tag}   rel {:.1}", i + 1, hit.path, hit.score);
        // With --context N, show N file lines around the first matching
        // line; otherwise the compact FTS snippet. Fall back to the
        // snippet if the file can't be read or no line matches literally
        // (e.g. the hit came from a stemmed term).
        let printed = match context {
            Some(n) => print_line_context(&ctx.workspace.root, &hit.path, &needles, n),
            None => false,
        };
        if !printed {
            let snippet = hit.snippet.replace('\n', " ");
            println!("     {}\n", snippet.trim());
        }
    }
    Ok(())
}

/// Print up to `n` lines of file context around the first line containing
/// any of `needles`. Returns false if nothing was printed.
fn print_line_context(root: &Path, rel: &str, needles: &[String], n: usize) -> bool {
    let Ok(text) = std::fs::read_to_string(root.join(rel)) else {
        return false;
    };
    let lines: Vec<&str> = text.lines().collect();
    let Some(hit_idx) = lines.iter().position(|l| {
        let lc = l.to_lowercase();
        needles.iter().any(|t| lc.contains(t.as_str()))
    }) else {
        return false;
    };
    let start = hit_idx.saturating_sub(n);
    let end = (hit_idx + n + 1).min(lines.len());
    let width = (end).to_string().len();
    for (idx, line) in lines.iter().enumerate().take(end).skip(start) {
        let marker = if idx == hit_idx { '>' } else { ' ' };
        println!("   {marker} {:>width$} | {}", idx + 1, line, width = width);
    }
    println!();
    true
}

/// Refresh the search index. Incremental unless `rebuild` is set.
pub fn index(rebuild: bool) -> Result<()> {
    let ctx = CmdContext::from_path(None)?;
    let conn = db::open(&ctx.workspace.root)?;
    if rebuild {
        search::rebuild(&conn)?;
    }
    let s = search::sync(&conn, &ctx.workspace.root, &ctx.workspace.ignore_set)?;
    println!(
        "index synced: {} added, {} updated, {} removed, {} unchanged",
        s.added, s.updated, s.removed, s.unchanged
    );
    Ok(())
}

/// Register (or update) a search collection by directory path.
pub fn collection_add(name: String, path: PathBuf) -> Result<()> {
    let ctx = CmdContext::from_path(None)?;
    let conn = db::open(&ctx.workspace.root)?;
    let abs = workspace::root::make_absolute(&path)?;
    let rel = abs.strip_prefix(&ctx.workspace.root).map_err(|_| {
        anyhow::anyhow!(
            "collection path must be inside the workspace ({})",
            ctx.workspace.root.display()
        )
    })?;
    let rel = rel.to_string_lossy();
    search::collection_add(&conn, &name, &rel)?;
    println!("collection {name:?} -> {rel}");
    Ok(())
}

/// List registered search collections.
pub fn collection_list() -> Result<()> {
    let ctx = CmdContext::from_path(None)?;
    let conn = db::open(&ctx.workspace.root)?;
    let cols = search::collection_list(&conn)?;
    if cols.is_empty() {
        println!("(no collections — add one with `kdb collection add <path> --name <name>`)");
        return Ok(());
    }
    for (name, path) in cols {
        println!("{name:<16} {path}");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workspace::root::ROOT_MARKER;
    use tempfile::TempDir;

    fn setup() -> (TempDir, Connection) {
        let tmp = TempDir::new().unwrap();
        std::fs::create_dir(tmp.path().join(ROOT_MARKER)).unwrap();
        let conn = db::open(tmp.path()).unwrap();
        (tmp, conn)
    }

    #[test]
    fn open_selector_resolves_to_non_closed_statuses() {
        let (_tmp, conn) = setup();
        let got = parse_statuses(&conn, "open").unwrap().unwrap();
        // Seeded set is backlog,cycle,in_progress,parked,done; parked+done
        // are is_closed=true, so "open" must exclude exactly those two.
        assert_eq!(got, vec!["backlog", "cycle", "in_progress"]);
    }

    #[test]
    fn all_selector_means_no_filter() {
        let (_tmp, conn) = setup();
        assert!(parse_statuses(&conn, "all").unwrap().is_none());
    }

    #[test]
    fn unknown_status_is_rejected() {
        let (_tmp, conn) = setup();
        assert!(parse_statuses(&conn, "nonexistent").is_err());
    }

    #[test]
    fn open_default_survives_seeded_status_removal() {
        // Regression: the old hardcoded default "backlog,cycle,in_progress"
        // broke `kdb tasks list` once a user removed the seeded `cycle`
        // status. The runtime-resolved "open" selector must not.
        let (_tmp, conn) = setup();
        statuses::remove(&conn, statuses::Kind::Task, "cycle").unwrap();
        let got = parse_statuses(&conn, "open").unwrap().unwrap();
        assert_eq!(got, vec!["backlog", "in_progress"]);
        assert!(!got.iter().any(|s| s == "cycle"));
    }
}
