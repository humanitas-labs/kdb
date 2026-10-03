//! Workspace root discovery, config, ignore rules, and the markdown file walk.
//!
//! A workspace is a directory containing `.kdb/`. Everything else (the graph,
//! the task db, the LSP) is built on a [`Workspace`].

use std::env;
use std::fs;
use std::io::ErrorKind;
use std::path::{Component, Path, PathBuf};
use std::sync::{Arc, Mutex};

use anyhow::{Context, Result, bail};
use globset::{GlobBuilder, GlobSet, GlobSetBuilder};
use ignore::{WalkBuilder, WalkState};

/// Marker directory that identifies the root of a workspace.
pub const ROOT_MARKER: &str = ".kdb";
/// Config file inside [`ROOT_MARKER`].
pub const CONFIG_FILE: &str = "config.toml";
/// Optional ignore file inside [`ROOT_MARKER`], one pattern per line.
pub const IGNORE_FILE: &str = "ignore";

const DEFAULT_IGNORE: &str = "\
# Default ignore patterns — edit to suit your workspace.
.git
target
node_modules
dist
build
.next
.cache
vendor
__pycache__
.venv
";

/// An opened workspace: canonical root plus compiled ignore rules.
#[derive(Debug, Clone)]
pub struct Workspace {
    /// Canonical absolute path of the directory containing `.kdb/`.
    pub root: PathBuf,
    /// `[workspace] name` from config, if set.
    pub name: Option<String>,
    ignores: GlobSet,
}

impl Workspace {
    /// Walk upward from `start` (a file or directory) to the nearest `.kdb/`.
    pub fn find(start: &Path) -> Result<Self> {
        if !start.exists() {
            bail!("path does not exist: {}", start.display());
        }
        let abs = start
            .canonicalize()
            .with_context(|| format!("failed to canonicalize {}", start.display()))?;
        let mut cursor = if abs.is_file() {
            abs.parent().map(Path::to_path_buf).unwrap_or(abs)
        } else {
            abs
        };
        loop {
            if cursor.join(ROOT_MARKER).is_dir() {
                return Self::open(&cursor);
            }
            if !cursor.pop() {
                break;
            }
        }
        bail!("could not find {ROOT_MARKER} starting from {}", start.display())
    }

    /// Find the workspace containing the current directory.
    pub fn from_cwd() -> Result<Self> {
        let cwd = env::current_dir().context("failed to read current directory")?;
        Self::find(&cwd)
    }

    /// Open a workspace whose root is already known.
    pub fn open(root: &Path) -> Result<Self> {
        let root = root
            .canonicalize()
            .with_context(|| format!("failed to canonicalize {}", root.display()))?;
        if !root.join(ROOT_MARKER).is_dir() {
            bail!("{} is not a workspace root (no {ROOT_MARKER}/)", root.display());
        }
        let config = load_config(&root)?;
        let mut patterns = load_ignore_file(&root)?;
        patterns.extend(config.ignore);
        let ignores = build_globset(&patterns)?;
        Ok(Self { root, name: config.name, ignores })
    }

    pub fn config_path(&self) -> PathBuf {
        self.root.join(ROOT_MARKER).join(CONFIG_FILE)
    }

    /// Absolute path for a root-relative path.
    pub fn abs(&self, rel: &Path) -> PathBuf {
        self.root.join(rel)
    }

    /// Root-relative, normalized path for an absolute path inside the workspace.
    pub fn rel(&self, abs: &Path) -> Option<PathBuf> {
        let abs = abs.canonicalize().ok().unwrap_or_else(|| abs.to_path_buf());
        normalize_rel(abs.strip_prefix(&self.root).ok()?)
    }

    /// Whether a root-relative path is excluded by the ignore rules.
    pub fn is_ignored(&self, rel: &Path, is_dir: bool) -> bool {
        let slash = rel.to_string_lossy().replace('\\', "/");
        if slash.is_empty() {
            return false;
        }
        if self.ignores.is_match(&slash) {
            return true;
        }
        is_dir && self.ignores.is_match(format!("{slash}/"))
    }

    /// All markdown files under the root, as sorted root-relative paths.
    pub fn walk_markdown(&self) -> Result<Vec<PathBuf>> {
        self.walk_markdown_under(&self.root)
    }

    /// Markdown files under `scope` (absolute, inside the root), sorted root-relative.
    ///
    /// Honors `.gitignore` files (git repo not required), hidden directories are
    /// visited, `.kdb/` and the workspace ignore patterns are skipped.
    pub fn walk_markdown_under(&self, scope: &Path) -> Result<Vec<PathBuf>> {
        let me = self.clone();
        let mut walker = WalkBuilder::new(scope);
        walker
            .follow_links(false)
            .hidden(false)
            .ignore(true)
            .git_ignore(true)
            .git_global(false)
            .git_exclude(true)
            .parents(true)
            .require_git(false)
            .filter_entry({
                let me = me.clone();
                move |entry| {
                    let is_dir = entry.file_type().is_some_and(|t| t.is_dir());
                    match me.rel(entry.path()) {
                        Some(rel) if rel.as_os_str().is_empty() => true,
                        Some(rel) => !me.is_ignored(&rel, is_dir),
                        None => false,
                    }
                }
            });

        let collected = Arc::new(Mutex::new(Vec::new()));
        walker.build_parallel().run(|| {
            let me = me.clone();
            let collected = Arc::clone(&collected);
            Box::new(move |result| {
                let Ok(entry) = result else { return WalkState::Continue };
                if !entry.file_type().is_some_and(|t| t.is_file()) {
                    return WalkState::Continue;
                }
                if entry.path().extension().and_then(|e| e.to_str()) != Some("md") {
                    return WalkState::Continue;
                }
                if let Some(rel) = me.rel(entry.path()) {
                    collected.lock().unwrap_or_else(|p| p.into_inner()).push(rel);
                }
                WalkState::Continue
            })
        });

        let mut paths = collected.lock().unwrap_or_else(|p| p.into_inner()).clone();
        paths.sort();
        paths.dedup();
        Ok(paths)
    }
}

/// Create `.kdb/` with a default config and ignore file in `dir` (cwd if `None`).
pub fn init(dir: Option<&Path>) -> Result<PathBuf> {
    let start = match dir {
        Some(p) => p.to_path_buf(),
        None => env::current_dir().context("failed to read current directory")?,
    };
    if !start.is_dir() {
        bail!("init path must be an existing directory: {}", start.display());
    }
    let root = start
        .canonicalize()
        .with_context(|| format!("failed to canonicalize {}", start.display()))?;
    let marker = root.join(ROOT_MARKER);
    if marker.exists() {
        bail!("{ROOT_MARKER} already exists in {}", root.display());
    }
    fs::create_dir(&marker).with_context(|| format!("failed to create {}", marker.display()))?;
    let name = root
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "workspace".to_string());
    fs::write(marker.join(CONFIG_FILE), format!("[workspace]\nname = \"{name}\"\n"))?;
    fs::write(marker.join(IGNORE_FILE), DEFAULT_IGNORE)?;
    Ok(root)
}

/// Collapse `.` and `..` in a relative path; `None` if it escapes or is absolute.
pub fn normalize_rel(path: &Path) -> Option<PathBuf> {
    let mut out = PathBuf::new();
    for c in path.components() {
        match c {
            Component::CurDir => {}
            Component::Normal(p) => out.push(p),
            Component::ParentDir => {
                if !out.pop() {
                    return None;
                }
            }
            Component::RootDir | Component::Prefix(_) => return None,
        }
    }
    Some(out)
}

#[derive(Default)]
struct Config {
    name: Option<String>,
    ignore: Vec<String>,
}

fn load_config(root: &Path) -> Result<Config> {
    let path = root.join(ROOT_MARKER).join(CONFIG_FILE);
    let raw = match fs::read_to_string(&path) {
        Ok(raw) => raw,
        Err(e) if e.kind() == ErrorKind::NotFound => return Ok(Config::default()),
        Err(e) => return Err(e).with_context(|| format!("failed to read {}", path.display())),
    };
    let value: toml::Value =
        toml::from_str(&raw).with_context(|| format!("failed to parse {}", path.display()))?;
    let name = value
        .get("workspace")
        .and_then(|w| w.get("name"))
        .and_then(|n| n.as_str())
        .map(str::to_owned);
    let ignore = value
        .get("index")
        .and_then(|i| i.get("ignore"))
        .and_then(|a| a.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str())
                .map(|s| expand_pattern(s.trim()))
                .collect()
        })
        .unwrap_or_default();
    Ok(Config { name, ignore })
}

fn load_ignore_file(root: &Path) -> Result<Vec<String>> {
    let path = root.join(ROOT_MARKER).join(IGNORE_FILE);
    let raw = match fs::read_to_string(&path) {
        Ok(raw) => raw,
        Err(e) if e.kind() == ErrorKind::NotFound => DEFAULT_IGNORE.to_string(),
        Err(e) => return Err(e).with_context(|| format!("failed to read {}", path.display())),
    };
    Ok(raw
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(expand_pattern)
        .collect())
}

/// A bare name matches at any depth; anything with a separator or glob is literal.
fn expand_pattern(line: &str) -> String {
    if line.contains('/') || line.contains('*') || line.contains('?') {
        line.to_string()
    } else {
        format!("**/{line}")
    }
}

fn build_globset(patterns: &[String]) -> Result<GlobSet> {
    let mut b = GlobSetBuilder::new();
    b.add(GlobBuilder::new(&format!("**/{ROOT_MARKER}")).literal_separator(true).build()?);
    for p in patterns {
        b.add(
            GlobBuilder::new(p)
                .literal_separator(true)
                .build()
                .with_context(|| format!("invalid ignore pattern `{p}`"))?,
        );
    }
    b.build().context("failed to compile ignore patterns")
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn init_find_and_walk() {
        let tmp = TempDir::new().unwrap();
        let root = init(Some(tmp.path())).unwrap();
        fs::create_dir_all(root.join("a/node_modules")).unwrap();
        fs::create_dir_all(root.join(".hidden")).unwrap();
        fs::write(root.join("a/x.md"), "# x").unwrap();
        fs::write(root.join("a/node_modules/y.md"), "# y").unwrap();
        fs::write(root.join(".hidden/z.md"), "# z").unwrap();
        fs::write(root.join("a/not.txt"), "").unwrap();

        let ws = Workspace::find(&root.join("a/x.md")).unwrap();
        assert_eq!(ws.root, root);
        assert_eq!(ws.name.as_deref(), root.file_name().and_then(|n| n.to_str()));
        let files = ws.walk_markdown().unwrap();
        assert_eq!(files, vec![PathBuf::from(".hidden/z.md"), PathBuf::from("a/x.md")]);
    }

    #[test]
    fn normalize() {
        assert_eq!(normalize_rel(Path::new("a/./b/../c.md")), Some(PathBuf::from("a/c.md")));
        assert_eq!(normalize_rel(Path::new("../x.md")), None);
    }
}
