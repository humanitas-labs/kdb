//! CLI surface of the graph domain: flags, scoping, exit codes, and output formats
//! (ported from the markdown cases of v1 `tests/cli.rs`).
mod common;
use common::*;

use serde_json::Value;

#[test]
fn check_clean_vault_exits_zero() {
    let ws = TestWorkspace::new();
    ws.write("a.md", "# A\n\n[B](b.md#target)\n");
    ws.write("b.md", "# B\n\n## Target\n\n[A](a.md#a)\n");
    let out = ws.kdb(&["check"]);
    assert!(out.status.success());
    assert_eq!(stdout(&out), "kdb check: no issues found\n");
}

#[test]
fn check_counts_orphans_unless_listed() {
    let ws = TestWorkspace::new();
    ws.write("a.md", "# A\n");
    ws.write("b.md", "# B\n");
    let out = ws.kdb(&["check"]);
    assert!(out.status.success());
    assert_eq!(stdout(&out), "2 orphan files (run `kdb check --orphans` to list)\n2 warnings\n");
    ws.write("c.md", "# C\n\n[a](a.md)\n[b](b.md)\n");
    let out = ws.kdb(&["check"]);
    assert_eq!(stdout(&out), "1 orphan file (run `kdb check --orphans` to list)\n1 warning\n");
    let out = ws.kdb(&["check", "--orphans"]);
    assert_eq!(stdout(&out), "c.md orphan file (0 inbound links)\n1 warning\n");
}

#[test]
fn check_scopes_to_path_but_validates_cross_subtree_links() {
    let ws = TestWorkspace::new();
    ws.write("crates/agent/guide.md", "# Guide\n\n[Missing Docs](../../docs/missing.md)\n");
    ws.write("docs/unrelated.md", "# Unrelated\n\n[Missing](missing.md)\n");
    let out = ws.kdb(&["check", "--orphans", "crates/agent"]);
    assert_eq!(out.status.code(), Some(1));
    assert_eq!(
        stdout(&out),
        "crates/agent/guide.md:3:1 broken link ../../docs/missing.md (target file not found: docs/missing.md)\n\
         crates/agent/guide.md orphan file (0 inbound links)\n1 error\n1 warning\n"
    );
    // A file scope keeps only that file; cwd-relative paths resolve from the cwd.
    let out = ws.kdb_in(&ws.path("docs"), &["check", "unrelated.md"]);
    assert_eq!(out.status.code(), Some(1));
    assert_eq!(
        stdout(&out),
        "docs/unrelated.md:3:1 broken link missing.md (target file not found: docs/missing.md)\n\
         1 orphan file (run `kdb check --orphans` to list)\n1 error\n1 warning\n"
    );
    let out = ws.kdb(&["check", "nope"]);
    assert_eq!(stdout(&out), "kdb check: no issues found\n");
}

#[test]
fn check_errors_without_root_marker() {
    let dir = tempfile::TempDir::new().unwrap();
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_kdb"))
        .arg("check")
        .current_dir(dir.path())
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1));
    assert!(stderr(&out).contains("could not find .kdb"));
}

#[test]
fn outline_prints_headings_with_aligned_line_numbers() {
    let ws = TestWorkspace::new();
    ws.write("docs/page.md", "# Top\n\n## Child\n\n### Leaf\n");
    let out = ws.kdb(&["outline", "docs/page.md"]);
    assert!(out.status.success(), "{}", stderr(&out));
    assert_eq!(stdout(&out), "# Top     L1\n## Child  L3\n### Leaf  L5\n");
    ws.write("docs/empty.md", "no headings\n");
    let out = ws.kdb(&["outline", "docs/empty.md"]);
    assert_eq!(stdout(&out), "(no symbols)\n");
    let out = ws.kdb(&["outline", "docs/missing.md"]);
    assert_eq!(out.status.code(), Some(1));
    assert!(stderr(&out).contains("path does not exist"));
}

#[test]
fn outline_directory_groups_by_file_and_ignores_rules() {
    let ws = TestWorkspace::new();
    ws.write(".gitignore", "private/\n");
    ws.write("docs/a.md", "# A\n");
    ws.write("docs/b/b.md", "# Longer B\n\n## Sub\n");
    ws.write("docs/private/p.md", "# P\n");
    ws.write("docs/x.txt", "# not md\n");
    let out = ws.kdb_in(&ws.path("docs"), &["outline", ".", "a.md"]);
    assert!(out.status.success(), "{}", stderr(&out));
    assert_eq!(stdout(&out), "── docs/a.md\n# A         L1\n\n── docs/b/b.md\n# Longer B  L1\n## Sub      L3\n");
    // Ignored files still outline when named explicitly.
    let out = ws.kdb(&["outline", "docs/private/p.md"]);
    assert_eq!(stdout(&out), "# P  L1\n");
}

#[test]
fn outline_selector_prints_section_bodies() {
    let ws = TestWorkspace::new();
    ws.write(
        "docs/page.md",
        "# Top\n\n## SOP-3 Refactor Cleanup\n\nCleanup details.\n\n### Nested Step\n\n- remove dead code\n\n## SOP-4 Bugfix\n\nBugfix details.\n",
    );
    let out = ws.kdb(&["outline", "docs/page.md", "-s", "#SOP-3-REFACTOR-CLEANUP"]);
    assert!(out.status.success(), "{}", stderr(&out));
    assert_eq!(
        stdout(&out),
        " 3 | ## SOP-3 Refactor Cleanup\n 4 | \n 5 | Cleanup details.\n 6 | \n 7 | ### Nested Step\n 8 | \n 9 | - remove dead code\n10 | \n"
    );
    let out = ws.kdb(&["outline", "docs/page.md", "-s", "sop-3-refactor-cleanup", "sop-4-bugfix", "--json"]);
    let rows: Vec<Value> = serde_json::from_str(&stdout(&out)).unwrap();
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0]["file"], "docs/page.md");
    assert_eq!(rows[0]["kind"], "heading");
    assert_eq!(rows[0]["display_kind"], "##");
    assert_eq!(rows[0]["name"], "SOP-3 Refactor Cleanup");
    assert_eq!(rows[0]["public"], true);
    assert_eq!(rows[0]["line"], 3);
    assert_eq!(rows[0]["end_line"], 10);
    assert_eq!(rows[1]["end_line"], 13);
    assert!(rows[0]["body"].as_str().unwrap().contains("### Nested Step"));
    let out = ws.kdb(&["outline", "docs/page.md", "-s", "SOP-3 Refactor Cleanup"]);
    assert_eq!(out.status.code(), Some(1));
    assert!(stderr(&out).contains("symbol not found: SOP-3 Refactor Cleanup in docs/page.md"));
    ws.write("docs/other.md", "# O\n");
    let out = ws.kdb(&["outline", "docs", "-s", "top"]);
    assert!(stderr(&out).contains("-s/--symbol requires a single definition file, got 2 files"));
}

#[test]
fn refs_lists_counts_and_files() {
    let ws = TestWorkspace::new();
    ws.write("docs/hooks.md", "# Hooks\n\n## useEffect\n");
    ws.write("tutorial.md", "# Tutorial\n\n[React Hooks](docs/hooks.md) and [again](docs/hooks.md#useEffect)\n");
    ws.write("index.md", "# Index\n\n[[docs/hooks]]\n");
    ws.write("patterns.md", "# Patterns\n\n[[docs/hooks#useEffect]]\n");
    let out = ws.kdb(&["refs", "docs/hooks.md"]);
    assert!(out.status.success(), "{}", stderr(&out));
    assert_eq!(
        stdout(&out),
        "index.md:3:1  [[docs/hooks]]\npatterns.md:3:1  [[docs/hooks#useEffect]]\ntutorial.md:3:1  docs/hooks.md\ntutorial.md:3:34  docs/hooks.md#useEffect\n"
    );
    let out = ws.kdb(&["refs", "docs/hooks.md#useEffect"]);
    assert_eq!(stdout(&out), "patterns.md:3:1  [[docs/hooks#useEffect]]\ntutorial.md:3:34  docs/hooks.md#useEffect\n");
    let out = ws.kdb(&["refs", "docs/hooks.md", "--count"]);
    assert_eq!(stdout(&out), "4\n");
    let out = ws.kdb(&["refs", "docs/hooks.md", "-l"]);
    assert_eq!(stdout(&out), "index.md\npatterns.md\ntutorial.md\n");
    let out = ws.kdb(&["refs", "tutorial.md"]);
    assert_eq!(stdout(&out), "(no references)\n");
    let abs = ws.path("docs/hooks.md");
    let out = ws.kdb(&["refs", abs.to_str().unwrap(), "--count"]);
    assert_eq!(stdout(&out), "4\n");
}

#[test]
fn refs_errors_match_v1() {
    let ws = TestWorkspace::new();
    ws.write("docs/hooks.md", "# Hooks\n");
    let out = ws.kdb(&["refs", "docs/hooks.md#nope"]);
    assert_eq!(out.status.code(), Some(1));
    assert!(stderr(&out).contains("target heading not found: docs/hooks.md#nope"));
    let out = ws.kdb(&["refs", "nothere.md"]);
    assert!(stderr(&out).contains("target file is not an indexed markdown file: nothere.md"));
    let out = ws.kdb(&["refs", "nothere.txt"]);
    assert!(stderr(&out).contains("invalid refs target `nothere.txt` (expected <file.md> or <file.md>#<heading>)"));
}
