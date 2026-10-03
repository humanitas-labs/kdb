//! Shared integration-test helpers: a temp workspace and a `kdb2` runner.
#![allow(dead_code)]

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use tempfile::TempDir;

pub struct TestWorkspace {
    _dir: TempDir,
    pub root: PathBuf,
}

impl TestWorkspace {
    /// A fresh workspace with `.kdb/config.toml` and the default ignore file.
    pub fn new() -> Self {
        let dir = TempDir::new().expect("tempdir");
        let root = dir.path().canonicalize().unwrap();
        fs::create_dir(root.join(".kdb")).unwrap();
        fs::write(root.join(".kdb/config.toml"), "[workspace]\nname = \"test\"\n").unwrap();
        Self { _dir: dir, root }
    }

    /// Write a file (creating parents) at a root-relative path.
    pub fn write(&self, rel: &str, content: &str) -> PathBuf {
        let path = self.root.join(rel);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(&path, content).unwrap();
        path
    }

    pub fn read(&self, rel: &str) -> String {
        fs::read_to_string(self.root.join(rel)).unwrap()
    }

    pub fn path(&self, rel: &str) -> PathBuf {
        self.root.join(rel)
    }

    /// Run `kdb2 <args>` with the workspace root as cwd.
    pub fn kdb2(&self, args: &[&str]) -> Output {
        self.kdb2_in(&self.root, args)
    }

    /// Run `kdb2 <args>` from a specific cwd inside the workspace.
    pub fn kdb2_in(&self, cwd: &Path, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_kdb2"))
            .args(args)
            .current_dir(cwd)
            .output()
            .expect("run kdb2")
    }

    /// Open the workspace database (creating it with the full schema).
    pub fn db(&self) -> rusqlite::Connection {
        let path = self.root.join(".kdb/index.db");
        let conn = rusqlite::Connection::open(&path).unwrap();
        // Schema is owned by the binary; run a no-op command to create it.
        drop(conn);
        let _ = self.kdb2(&["statuses", "list"]);
        rusqlite::Connection::open(&path).unwrap()
    }
}

pub fn stdout(out: &Output) -> String {
    String::from_utf8_lossy(&out.stdout).into_owned()
}

pub fn stderr(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}
