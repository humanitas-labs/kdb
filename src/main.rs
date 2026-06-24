use clap::{Args, Parser, Subcommand};
use kdb::search::FType;
use std::path::PathBuf;

// ----------------------------
// projects/kdb/src/main.rs
//
// struct Cli               L31
// enum Command             L37
// enum CodemapCmd         L226
// enum CollectionCmd      L264
// enum ProjectsCmd        L278
// enum TasksCmd           L334
// enum TaskLabelCmd       L475
// enum CyclesCmd          L493
// enum LabelsCmd          L542
// struct StatusKindArg    L579
// fn resolve_kind()       L588
// fn parse_bool_flag()    L599
// enum StatusesCmd        L610
// async fn main()         L692
// ----------------------------

#[derive(Debug, Parser)]
#[command(
    name = "kdb",
    version,
    about = "Code intelligence CLI and LSP for knowledge bases",
    after_help = "pls report bugs: https://github.com/dremnik/kdb/issues"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Initialize a kdb workspace in a directory.
    Init {
        /// Optional directory path (defaults to current directory).
        path: Option<PathBuf>,
    },
    /// Print the absolute path of the workspace root.
    Root,
    /// Report broken links and orphan files.
    Check {
        /// Print each orphan file path.
        #[arg(long)]
        orphans: bool,
        /// Optional file or directory path to scope check output to.
        path: Option<PathBuf>,
    },
    /// Print a filtered directory tree for the workspace.
    Tree {
        /// Maximum display depth (same as `tree -L`).
        #[arg(short = 'L', long = "level", alias = "depth")]
        level: Option<usize>,
        /// Include hidden entries (same as `tree -a`).
        #[arg(short = 'a', long)]
        all: bool,
        /// Show directories only (same as `tree -d`).
        #[arg(short = 'd', long = "dirs-only")]
        dirs_only: bool,
        /// Print full relative paths for each entry (same as `tree -f`).
        #[arg(short = 'f', long = "full-path")]
        full_path: bool,
        /// Exclude entries matching a wildcard pattern (same as `tree -I`).
        #[arg(short = 'I', long = "ignore")]
        ignore: Vec<String>,
        /// Include only entries matching a wildcard pattern (same as `tree -P`).
        #[arg(short = 'P', long = "pattern")]
        pattern: Vec<String>,
        /// Emit machine-readable JSON output.
        #[arg(short = 'J', long)]
        json: bool,
        /// Optional path to render (defaults to workspace root).
        path: Option<PathBuf>,
    },
    /// Print the outline (headings / symbols) for files and/or directories.
    #[command(alias = "symbols")]
    Outline {
        /// File or directory paths to inspect (accepts multiple).
        #[arg(required = true)]
        paths: Vec<PathBuf>,
        /// Select symbols by name or `Parent::name` (single file only).
        #[arg(short = 's', long = "symbol", num_args = 1..)]
        symbols: Vec<String>,
        /// Emit structured JSON output.
        #[arg(long)]
        json: bool,
        /// Only include public/exported symbols.
        #[arg(long = "public")]
        public_only: bool,
    },
    /// Find inbound references to a markdown target or code symbol.
    Refs {
        /// Symbol target expression (e.g. `notes.md#getting-started`).
        target: String,
        /// Code symbol name for code reference mode.
        #[arg(short = 's', long = "symbol")]
        symbol: Option<String>,
        /// Show N lines of context around each symbol reference (text mode only).
        #[arg(short = 'c', long = "context")]
        context: Option<usize>,
        /// Emit structured JSON output.
        #[arg(long)]
        json: bool,
        /// Print only the number of inbound references.
        #[arg(long)]
        count: bool,
        /// Print only unique file paths containing references.
        #[arg(short = 'l', long = "files")]
        files: bool,
    },
    /// Print direct dependencies for a file/symbol target.
    Deps {
        /// File or symbol target expression.
        target: String,
        /// Emit structured JSON output.
        #[arg(long)]
        json: bool,
    },
    /// Render a dependency graph for markdown and code symbols.
    Graph {
        /// Optional starting path to discover kdb root from.
        path: Option<PathBuf>,
    },
    /// Resolve markdown includes, or materialize per-project TODO files.
    Render {
        /// File path to render (resolves `![[]]` embeds to stdout).
        file: Option<PathBuf>,
        /// Materialize TODO for a single project slug.
        #[arg(short = 'P', long)]
        project: Option<String>,
        /// Materialize TODO for every non-archived project.
        #[arg(long)]
        all: bool,
        /// Cap per-task file materialization to N top-priority open tasks
        /// (in_progress tasks always included). Defaults to `meta.top_n`.
        #[arg(short = 'n', long)]
        limit: Option<i64>,
    },
    /// Generate or update code index headers in supported code files.
    Fmt {
        /// Optional file or directory path to format (defaults to workspace root).
        path: Option<PathBuf>,
        /// Force frontmatter insertion into markdown files that already have frontmatter.
        #[arg(long)]
        force: bool,
    },
    /// Run the language server over stdio.
    Lsp {
        /// Optional starting path to discover kdb root from.
        path: Option<PathBuf>,
    },
    /// Check for updates and self-update the binary.
    Update {
        /// Only check for a newer version without installing.
        #[arg(long)]
        check: bool,
    },
    /// Manage projects in the relational layer.
    Projects {
        #[command(subcommand)]
        action: ProjectsCmd,
    },
    /// Manage tasks in the relational layer.
    Tasks {
        #[command(subcommand)]
        action: Option<TasksCmd>,
    },
    /// Manage cycles in the relational layer.
    Cycles {
        #[command(subcommand)]
        action: CyclesCmd,
    },
    /// Manage labels in the relational layer.
    Labels {
        #[command(subcommand)]
        action: LabelsCmd,
    },
    /// Manage customizable task & project statuses.
    Statuses {
        #[command(subcommand)]
        action: StatusesCmd,
    },
    /// Full-text search the workspace corpus (prose by default).
    Search {
        /// Search terms (treated as keywords; punctuation is safe).
        query: String,
        /// File classes to search: docs (default), code, or all.
        #[arg(long, value_enum, default_value_t = FType::Docs)]
        ftype: FType,
        /// Constrain to a registered collection (see `kdb collection`).
        #[arg(short = 'C', long)]
        collection: Option<String>,
        /// Constrain to an ad-hoc directory (no registration needed).
        #[arg(short = 'p', long, conflicts_with = "collection")]
        path: Option<PathBuf>,
        /// Show N file lines around each match instead of the snippet.
        #[arg(short = 'c', long = "context")]
        context: Option<usize>,
        /// Maximum number of results.
        #[arg(short = 'n', long, default_value_t = 20)]
        limit: i64,
    },
    /// Refresh the full-text search index (incremental unless --rebuild).
    Index {
        /// Discard and rebuild the entire index from scratch.
        #[arg(long)]
        rebuild: bool,
    },
    /// Manage named search collections used to scope `kdb search`.
    Collection {
        #[command(subcommand)]
        action: CollectionCmd,
    },
    /// Index colocated CODEMAP.md domain maps (discovery, freshness, index).
    Codemap {
        #[command(subcommand)]
        action: CodemapCmd,
    },
}

#[derive(Debug, Subcommand)]
enum CodemapCmd {
    /// Discover CODEMAP.md files and list their domains.
    #[command(alias = "ls")]
    List {
        /// Emit structured JSON output (the workflow's consumption format).
        #[arg(long)]
        json: bool,
        /// Optional path to scope discovery to (defaults to enclosing git repo).
        path: Option<PathBuf>,
    },
    /// Lint codemaps: dangling roots, coverage gaps, stale maps.
    Check {
        /// Only run staleness lints (default: run all).
        #[arg(long)]
        stale: bool,
        /// Only run coverage/orphan lints (default: run all).
        #[arg(long)]
        orphans: bool,
        /// Exit non-zero if any actionable finding (for CI/pre-commit).
        #[arg(long)]
        strict: bool,
        /// Minimum supported code files for an uncovered subtree to be flagged.
        #[arg(long = "min-files", default_value_t = kdb::codemap::check::DEFAULT_MIN_FILES)]
        min_files: usize,
        /// Emit structured JSON output.
        #[arg(long)]
        json: bool,
        /// Optional path to scope to (defaults to enclosing git repo).
        path: Option<PathBuf>,
    },
    /// Render the deterministic codemap index to stdout.
    Render {
        /// Optional path to scope to (defaults to enclosing git repo).
        path: Option<PathBuf>,
    },
}

#[derive(Debug, Subcommand)]
enum CollectionCmd {
    /// Register (or update) a collection by directory path.
    Add {
        /// Directory path inside the workspace.
        path: PathBuf,
        /// Name to reference the collection by.
        #[arg(long)]
        name: String,
    },
    /// List registered collections.
    List,
}

#[derive(Debug, Subcommand)]
enum ProjectsCmd {
    /// List projects.
    #[command(alias = "ls")]
    List {
        /// Include archived projects.
        #[arg(short = 'a', long)]
        all: bool,
        /// Emit structured JSON output.
        #[arg(long)]
        json: bool,
    },
    /// Add a new project.
    Add {
        /// Unique slug (e.g. "hermaeus").
        slug: String,
        /// 2–6 char uppercase alias used in task ids (e.g. "HRM").
        #[arg(short = 'a', long)]
        alias: String,
        /// Relative path from kdb root (e.g. "projects/hermaeus").
        #[arg(short = 'p', long)]
        path: String,
        /// Display name (defaults to slug).
        #[arg(short = 'n', long)]
        name: Option<String>,
        /// Optional description.
        #[arg(short = 'd', long)]
        description: Option<String>,
    },
    /// Edit an existing project.
    Edit {
        /// Project slug to edit.
        slug: String,
        /// New 2–6 char uppercase alias.
        #[arg(short = 'a', long)]
        alias: Option<String>,
        #[arg(short = 'n', long)]
        name: Option<String>,
        #[arg(short = 'p', long)]
        path: Option<String>,
        /// Status slug (must exist in project_statuses).
        #[arg(long)]
        status: Option<String>,
        #[arg(short = 'd', long)]
        description: Option<String>,
    },
    /// Show a project.
    Show {
        /// Project slug to show.
        slug: String,
        /// Emit structured JSON output.
        #[arg(long)]
        json: bool,
    },
}

#[derive(Debug, Subcommand)]
enum TasksCmd {
    /// List tasks.
    #[command(alias = "ls")]
    List {
        /// Comma-separated status slugs, or "open" (every non-closed status,
        /// resolved at runtime — the default) or "all".
        #[arg(short = 's', long, default_value = "open")]
        status: String,
        /// Filter by project slug (defaults to the active project).
        #[arg(short = 'P', long)]
        project: Option<String>,
        /// Filter by cycle key.
        #[arg(short = 'c', long)]
        cycle: Option<String>,
        /// Filter by priority (1-5).
        #[arg(short = 'p', long)]
        priority: Option<i64>,
        /// Limit to N rows.
        #[arg(short = 'n', long)]
        limit: Option<i64>,
        /// Include subtasks (rows with a parent). Off by default —
        /// subtasks surface inside their parent task's view instead.
        #[arg(long)]
        include_children: bool,
        /// Emit structured JSON output.
        #[arg(long)]
        json: bool,
    },
    /// Add a new task.
    Add {
        /// Task title.
        title: String,
        /// Project slug (defaults to the active project).
        #[arg(short = 'P', long)]
        project: Option<String>,
        /// Body text.
        #[arg(short = 'b', long)]
        body: Option<String>,
        /// Priority (1-5, default 3).
        #[arg(short = 'p', long)]
        priority: Option<i64>,
        /// Cycle key (e.g. C-14).
        #[arg(short = 'c', long)]
        cycle: Option<String>,
        /// Parent task id.
        #[arg(long)]
        parent: Option<String>,
        /// Insert immediately before this task (inherits its parent unless --parent given).
        #[arg(long, conflicts_with = "after")]
        before: Option<String>,
        /// Insert immediately after this task (inherits its parent unless --parent given).
        #[arg(long, conflicts_with = "before")]
        after: Option<String>,
    },
    /// Move a task to a new position within its sibling list.
    #[command(alias = "mv")]
    Move {
        /// Task id (e.g. KDB-4).
        id: String,
        /// Place immediately before this task.
        #[arg(long, conflicts_with_all = ["after", "top", "bottom"])]
        before: Option<String>,
        /// Place immediately after this task.
        #[arg(long, conflicts_with_all = ["before", "top", "bottom"])]
        after: Option<String>,
        /// Move to the top of the sibling list.
        #[arg(long, conflicts_with_all = ["before", "after", "bottom"])]
        top: bool,
        /// Move to the bottom of the sibling list.
        #[arg(long, conflicts_with_all = ["before", "after", "top"])]
        bottom: bool,
    },
    /// Edit an existing task.
    Edit {
        /// Task id (e.g. hermaeus-42).
        id: String,
        #[arg(short = 't', long)]
        title: Option<String>,
        #[arg(short = 'b', long)]
        body: Option<String>,
        #[arg(short = 'p', long)]
        priority: Option<i64>,
        /// Set cycle key (use empty string to clear).
        #[arg(short = 'c', long)]
        cycle: Option<String>,
        /// Set parent task id (use empty string to clear).
        #[arg(long)]
        parent: Option<String>,
        /// Set status slug (must exist in task_statuses).
        #[arg(short = 's', long)]
        status: Option<String>,
    },
    /// View a task.
    #[command(alias = "show")]
    View {
        id: String,
        #[arg(long)]
        json: bool,
    },
    /// Delete a task. Soft-delete by default (sets `deleted_at`,
    /// hides from list/render); `--hard` removes the row + subtree
    /// permanently.
    #[command(alias = "d", alias = "rm")]
    Delete {
        id: String,
        /// Permanently delete the row and its entire subtree.
        #[arg(long)]
        hard: bool,
    },
    /// Restore a soft-deleted task (and its subtree).
    Restore { id: String },
    /// Permanently delete tasks matching filters (and their subtrees).
    /// Requires at least one of `--status` or `--deleted`.
    Purge {
        /// Limit to a project slug.
        #[arg(short = 'P', long)]
        project: Option<String>,
        /// Match tasks with this status (e.g. `done`).
        #[arg(short = 's', long)]
        status: Option<String>,
        /// Match soft-deleted tasks (`deleted_at IS NOT NULL`).
        #[arg(long)]
        deleted: bool,
        /// Show what would be deleted without making changes.
        #[arg(long)]
        dry_run: bool,
    },
    /// Mark a task as done.
    Done { id: String },
    /// Mark a task as parked.
    Park { id: String },
    /// Reopen a parked or done task.
    Reopen { id: String },
    /// Manage labels on a task.
    Label {
        #[command(subcommand)]
        action: TaskLabelCmd,
    },
}

#[derive(Debug, Subcommand)]
enum TaskLabelCmd {
    /// Attach one or more labels to a task (unknown slugs are created).
    Add {
        /// Task id (e.g. HRM-0120).
        id: String,
        /// Label slugs to attach.
        #[arg(required = true)]
        labels: Vec<String>,
    },
    /// Detach one or more labels from a task.
    Rm {
        id: String,
        #[arg(required = true)]
        labels: Vec<String>,
    },
}

#[derive(Debug, Subcommand)]
enum CyclesCmd {
    /// List cycles (ordered by start_date desc).
    #[command(alias = "ls")]
    List {
        #[arg(long)]
        json: bool,
    },
    /// Add a new cycle.
    Add {
        /// Cycle key (e.g. C-15).
        key: String,
        /// Start date (YYYY-MM-DD).
        #[arg(short = 's', long)]
        start: String,
        /// End date (YYYY-MM-DD).
        #[arg(short = 'e', long)]
        end: String,
        /// Optional description.
        #[arg(short = 'd', long)]
        description: Option<String>,
        #[arg(long, value_parser = ["planned", "active", "done", "abandoned"])]
        status: Option<String>,
        /// Optional path to the cycle's plan/review artifacts.
        #[arg(short = 'p', long)]
        path: Option<String>,
    },
    /// Edit an existing cycle.
    Edit {
        key: String,
        #[arg(short = 's', long)]
        start: Option<String>,
        #[arg(short = 'e', long)]
        end: Option<String>,
        #[arg(short = 'd', long)]
        description: Option<String>,
        #[arg(long, value_parser = ["planned", "active", "done", "abandoned"])]
        status: Option<String>,
        #[arg(short = 'p', long)]
        path: Option<String>,
    },
    /// Show a cycle.
    Show {
        key: String,
        #[arg(long)]
        json: bool,
    },
}

#[derive(Debug, Subcommand)]
enum LabelsCmd {
    /// List labels.
    #[command(alias = "ls")]
    List {
        #[arg(long)]
        json: bool,
    },
    /// Add a new label.
    Add {
        /// Label slug (unique).
        slug: String,
        /// Display name (defaults to slug).
        #[arg(short = 'n', long)]
        name: Option<String>,
        /// Optional hex color (e.g. #ff0000).
        #[arg(short = 'c', long)]
        color: Option<String>,
    },
    /// Edit an existing label.
    Edit {
        slug: String,
        #[arg(short = 'n', long)]
        name: Option<String>,
        #[arg(short = 'c', long)]
        color: Option<String>,
    },
    /// Show a label.
    Show {
        slug: String,
        #[arg(long)]
        json: bool,
    },
}

/// Shared `--tasks | --projects` scope selector for `kdb statuses`.
#[derive(Debug, Args)]
#[group(required = true, multiple = false)]
struct StatusKindArg {
    /// Operate on task statuses.
    #[arg(long)]
    tasks: bool,
    /// Operate on project statuses.
    #[arg(long)]
    projects: bool,
}

fn resolve_kind(arg: &StatusKindArg) -> kdb::statuses::Kind {
    if arg.tasks {
        kdb::statuses::Kind::Task
    } else {
        kdb::statuses::Kind::Project
    }
}

/// Lenient boolean parser for CLI flags. Accepts `true/t/yes/y/1` and
/// `false/f/no/n/0` (case-insensitive). Used so `--hidden 1` and
/// `--hidden false` both work.
fn parse_bool_flag(s: &str) -> Result<bool, String> {
    match s.to_ascii_lowercase().as_str() {
        "true" | "t" | "yes" | "y" | "1" => Ok(true),
        "false" | "f" | "no" | "n" | "0" => Ok(false),
        other => Err(format!(
            "expected true/false/yes/no/1/0, got '{other}'"
        )),
    }
}

#[derive(Debug, Subcommand)]
enum StatusesCmd {
    /// List statuses for the chosen kind.
    #[command(alias = "ls")]
    List {
        #[command(flatten)]
        kind: StatusKindArg,
        #[arg(long)]
        json: bool,
    },
    /// Add a new status.
    Add {
        /// New status slug.
        slug: String,
        #[command(flatten)]
        kind: StatusKindArg,
        /// Display name (defaults to slug).
        #[arg(short = 'n', long)]
        name: Option<String>,
        /// Free-form description shown in `statuses show`.
        #[arg(short = 'd', long)]
        description: Option<String>,
        /// Optional hex color (e.g. #ff0000).
        #[arg(short = 'c', long)]
        color: Option<String>,
        /// Mark as closed (stamps closed_at; only valid with --tasks).
        #[arg(long)]
        closed: bool,
        /// Mark as archived (hidden from default project list; only valid with --projects).
        #[arg(long)]
        archived: bool,
        /// Sort order (lower renders first). Defaults to MAX+1.
        #[arg(long)]
        order: Option<i64>,
        /// Hidden status — section renders as a count + summary command line, no table.
        #[arg(long, value_name = "BOOL", value_parser = parse_bool_flag)]
        hidden: Option<bool>,
    },
    /// Edit an existing status.
    Edit {
        slug: String,
        #[command(flatten)]
        kind: StatusKindArg,
        #[arg(short = 'n', long)]
        name: Option<String>,
        #[arg(short = 'd', long)]
        description: Option<String>,
        #[arg(short = 'c', long)]
        color: Option<String>,
        /// Toggle the closed flag (only valid with --tasks).
        #[arg(long, conflicts_with = "no_closed")]
        closed: bool,
        #[arg(long, conflicts_with = "closed")]
        no_closed: bool,
        /// Toggle the archived flag (only valid with --projects).
        #[arg(long, conflicts_with = "no_archived")]
        archived: bool,
        #[arg(long, conflicts_with = "archived")]
        no_archived: bool,
        /// Sort order (lower renders first).
        #[arg(long)]
        order: Option<i64>,
        /// Hidden status — section renders as a count + summary command line, no table.
        #[arg(long, value_name = "BOOL", value_parser = parse_bool_flag)]
        hidden: Option<bool>,
    },
    /// Remove a status (fails if in use).
    Rm {
        slug: String,
        #[command(flatten)]
        kind: StatusKindArg,
    },
    /// Show a single status.
    Show {
        slug: String,
        #[command(flatten)]
        kind: StatusKindArg,
        #[arg(long)]
        json: bool,
    },
}

#[tokio::main]
async fn main() {
    let cli = Cli::parse();

    let result = match cli.command {
        Command::Init { path } => kdb::cmd::init(path),
        Command::Root => kdb::cmd::root(),
        Command::Check { path, orphans } => match kdb::cmd::check(path, orphans) {
            Ok(has_issues) => {
                if has_issues {
                    std::process::exit(1);
                }
                Ok(())
            }
            Err(error) => Err(error),
        },
        Command::Tree {
            level,
            all,
            dirs_only,
            full_path,
            ignore,
            pattern,
            json,
            path,
        } => kdb::cmd::tree(
            path, level, json, all, dirs_only, full_path, ignore, pattern,
        ),
        Command::Outline {
            paths,
            symbols,
            json,
            public_only,
        } => kdb::cmd::symbols(paths, symbols, json, public_only),
        Command::Refs {
            target,
            symbol,
            context,
            json,
            count,
            files,
        } => kdb::cmd::refs(target, symbol, context, json, count, files),
        Command::Deps { target, json } => kdb::cmd::deps(target, json),
        Command::Graph { path } => kdb::cmd::graph(path),
        Command::Render {
            file,
            project,
            all,
            limit,
        } => kdb::cmd::render(file, project, all, limit),
        Command::Fmt { path, force } => kdb::cmd::format(path, force),
        Command::Lsp { path } => kdb::lsp::serve(path).await,
        Command::Update { check } => kdb::cmd::update(check),
        Command::Projects { action } => match action {
            ProjectsCmd::List { all, json } => kdb::cmd::projects_list(all, json),
            ProjectsCmd::Add {
                slug,
                alias,
                path,
                name,
                description,
            } => kdb::cmd::projects_add(slug, alias, path, name, description),
            ProjectsCmd::Edit {
                slug,
                alias,
                name,
                path,
                status,
                description,
            } => kdb::cmd::projects_edit(slug, alias, name, path, status, description),
            ProjectsCmd::Show { slug, json } => kdb::cmd::projects_show(slug, json),
        },
        Command::Tasks { action } => match action.unwrap_or(TasksCmd::List {
            status: "open".to_string(),
            project: None,
            cycle: None,
            priority: None,
            limit: None,
            include_children: false,
            json: false,
        }) {
            TasksCmd::List {
                status,
                project,
                cycle,
                priority,
                limit,
                include_children,
                json,
            } => kdb::cmd::tasks_list(status, project, cycle, priority, limit, include_children, json),
            TasksCmd::Add {
                title,
                project,
                body,
                priority,
                cycle,
                parent,
                before,
                after,
            } => kdb::cmd::tasks_add(title, project, body, priority, cycle, parent, before, after),
            TasksCmd::Move {
                id,
                before,
                after,
                top,
                bottom,
            } => kdb::cmd::tasks_move(id, before, after, top, bottom),
            TasksCmd::Edit {
                id,
                title,
                body,
                priority,
                cycle,
                parent,
                status,
            } => kdb::cmd::tasks_edit(id, title, body, priority, cycle, parent, status),
            TasksCmd::View { id, json } => kdb::cmd::tasks_view(id, json),
            TasksCmd::Delete { id, hard } => kdb::cmd::tasks_delete(id, hard),
            TasksCmd::Restore { id } => kdb::cmd::tasks_restore(id),
            TasksCmd::Purge {
                project,
                status,
                deleted,
                dry_run,
            } => kdb::cmd::tasks_purge(project, status, deleted, dry_run),
            TasksCmd::Done { id } => kdb::cmd::tasks_set_status(id, "done"),
            TasksCmd::Park { id } => kdb::cmd::tasks_set_status(id, "parked"),
            TasksCmd::Reopen { id } => kdb::cmd::tasks_set_status(id, "backlog"),
            TasksCmd::Label { action } => match action {
                TaskLabelCmd::Add { id, labels } => kdb::cmd::tasks_label_add(id, labels),
                TaskLabelCmd::Rm { id, labels } => kdb::cmd::tasks_label_rm(id, labels),
            },
        },
        Command::Cycles { action } => match action {
            CyclesCmd::List { json } => kdb::cmd::cycles_list(json),
            CyclesCmd::Add {
                key,
                start,
                end,
                description,
                status,
                path,
            } => kdb::cmd::cycles_add(key, start, end, description, status, path),
            CyclesCmd::Edit {
                key,
                start,
                end,
                description,
                status,
                path,
            } => kdb::cmd::cycles_edit(key, start, end, description, status, path),
            CyclesCmd::Show { key, json } => kdb::cmd::cycles_show(key, json),
        },
        Command::Labels { action } => match action {
            LabelsCmd::List { json } => kdb::cmd::labels_list(json),
            LabelsCmd::Add { slug, name, color } => kdb::cmd::labels_add(slug, name, color),
            LabelsCmd::Edit { slug, name, color } => kdb::cmd::labels_edit(slug, name, color),
            LabelsCmd::Show { slug, json } => kdb::cmd::labels_show(slug, json),
        },
        Command::Statuses { action } => match action {
            StatusesCmd::List { kind, json } => {
                kdb::cmd::statuses_list(resolve_kind(&kind), json)
            }
            StatusesCmd::Add {
                slug,
                kind,
                name,
                description,
                color,
                closed,
                archived,
                order,
                hidden,
            } => kdb::cmd::statuses_add(
                slug,
                resolve_kind(&kind),
                name,
                description,
                color,
                closed,
                archived,
                order,
                hidden,
            ),
            StatusesCmd::Edit {
                slug,
                kind,
                name,
                description,
                color,
                closed,
                no_closed,
                archived,
                no_archived,
                order,
                hidden,
            } => kdb::cmd::statuses_edit(
                slug,
                resolve_kind(&kind),
                name,
                description,
                color,
                closed,
                no_closed,
                archived,
                no_archived,
                order,
                hidden,
            ),
            StatusesCmd::Rm { slug, kind } => kdb::cmd::statuses_rm(slug, resolve_kind(&kind)),
            StatusesCmd::Show { slug, kind, json } => {
                kdb::cmd::statuses_show(slug, resolve_kind(&kind), json)
            }
        },
        Command::Search {
            query,
            ftype,
            collection,
            path,
            context,
            limit,
        } => kdb::cmd::search(query, ftype, collection, path, context, limit),
        Command::Index { rebuild } => kdb::cmd::index(rebuild),
        Command::Collection { action } => match action {
            CollectionCmd::Add { path, name } => kdb::cmd::collection_add(name, path),
            CollectionCmd::List => kdb::cmd::collection_list(),
        },
        Command::Codemap { action } => match action {
            CodemapCmd::List { json, path } => kdb::codemap::ls(path, json),
            CodemapCmd::Check {
                stale,
                orphans,
                strict,
                min_files,
                json,
                path,
            } => kdb::codemap::check::check(
                path,
                kdb::codemap::check::Lints { stale, orphans },
                strict,
                min_files,
                json,
            ),
            CodemapCmd::Render { path } => kdb::codemap::render::render(path),
        },
    };

    if let Err(error) = result {
        eprintln!("{error:#}");
        std::process::exit(1);
    }
}
