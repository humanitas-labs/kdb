//! `kdb` — a compiler and language server for markdown knowledge bases.
//!
//! Treats a directory of markdown files like a codebase: headings are exported
//! symbols, links are imports, and broken references are compile errors.
//!
//! # Modules
//!
//! - [`cmd`] — CLI command implementations (`init`, `check`, `render`, `tree`, `symbols`, `fmt`, `lsp`).
//! - [`db`] — SQLite-backed relational layer (projects, cycles, tasks, labels).
//! - [`deps`] — Code dependency extraction for `kdb deps`.
//! - [`fmt`] — Code file index header generation and maintenance.
//! - [`index`] — Markdown parser, vault indexer, and link resolver.
//! - [`lang`] — Shared code language identifiers and file-type detection.
//! - [`lsp`] — Language Server Protocol implementation.
//! - [`materialize`] — DB → markdown materialization for per-project TODO files.
//! - [`projects`] — Projects table access (slug, name, path, status).
//! - [`render`] — Transclusion resolution for `![[file#heading]]` embeds.
//! - [`resolve`] — Workspace-aware code import resolution.
//! - [`symbols`] — Multi-language code symbol extraction.
//! - [`tasks`] — Tasks table access (per-project seq, statuses, priorities).
//! - [`tree`] — Filtered tree rendering for workspace orientation.
//! - [`workspace`] — Shared workspace infrastructure (root, config, discovery, paths, ignore).

pub mod cmd;
pub mod codemap;
pub mod color;
pub mod cycles;
pub mod db;
pub mod deps;
pub mod fmt;
pub mod index;
pub mod labels;
pub mod lang;
pub mod lsp;
pub mod materialize;
pub mod projects;
pub mod render;
pub mod resolve;
pub mod search;
pub mod statuses;
pub mod symbols;
pub mod tasks;
pub mod tasks_import;
pub mod tree;
pub mod update;
pub mod workspace;

// ---------------------------
// projects/kdb/src/lib.rs
//
// pub mod cmd             L24
// pub mod codemap         L25
// pub mod color           L26
// pub mod cycles          L27
// pub mod db              L28
// pub mod deps            L29
// pub mod fmt             L30
// pub mod index           L31
// pub mod labels          L32
// pub mod lang            L33
// pub mod lsp             L34
// pub mod materialize     L35
// pub mod projects        L36
// pub mod render          L37
// pub mod resolve         L38
// pub mod search          L39
// pub mod statuses        L40
// pub mod symbols         L41
// pub mod tasks           L42
// pub mod tasks_import    L43
// pub mod tree            L44
// pub mod update          L45
// pub mod workspace       L46
// ---------------------------

