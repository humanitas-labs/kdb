use serde_json::Value;
use std::fs;
use std::path::Path;
use std::process::{Command, Output};
use tempfile::tempdir;

// ----------------------------------------------------------------------------------
// projects/kdb/tests/codemap.rs
//
// Integration tests for `kdb codemap` (ls / check / render).
// ----------------------------------------------------------------------------------

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_kdb")
}

fn run(root: &Path, args: &[&str]) -> Output {
    Command::new(bin())
        .current_dir(root)
        .args(args)
        .output()
        .expect("run kdb command")
}

fn write_file(root: &Path, rel_path: &str, content: &str) {
    let path = root.join(rel_path);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).expect("create parent dirs");
    }
    fs::write(path, content).expect("write fixture file");
}

fn write_marker(root: &Path) {
    write_file(root, ".kdb/config.toml", "[workspace]\nname = \"fixture\"\n");
}

fn map(domain: &str, root: &str, commit: Option<&str>) -> String {
    let mut fm = format!("---\ndomain: {domain}\nroot: {root}\n");
    if let Some(c) = commit {
        fm.push_str(&format!("commit: {c}\n"));
    }
    fm.push_str("---\n\n# map\n");
    fm
}

fn git(root: &Path, args: &[&str]) -> Output {
    Command::new("git")
        .current_dir(root)
        .args(args)
        .output()
        .expect("run git command")
}

fn git_commit_all(root: &Path, message: &str) {
    assert!(git(root, &["add", "-A"]).status.success(), "git add");
    let out = Command::new("git")
        .current_dir(root)
        .args(["-c", "user.email=t@t", "-c", "user.name=t", "commit", "-m", message])
        .output()
        .expect("git commit");
    assert!(out.status.success(), "git commit: {}", String::from_utf8_lossy(&out.stderr));
}

fn short_head(root: &Path) -> String {
    let out = git(root, &["rev-parse", "--short", "HEAD"]);
    assert!(out.status.success(), "rev-parse");
    String::from_utf8(out.stdout).unwrap().trim().to_string()
}

// ── ls ──────────────────────────────────────────────────────────────────────

#[test]
fn ls_json_lists_discovered_maps() {
    let tmp = tempdir().unwrap();
    let root = tmp.path();
    write_marker(root);
    write_file(root, "src/alpha/CODEMAP.md", &map("alpha", "src/alpha", None));
    write_file(root, "src/alpha/a.rs", "pub fn a() {}\n");
    write_file(root, "src/beta/CODEMAP.md", &map("beta", "src/beta", None));
    write_file(root, "src/beta/b.rs", "pub fn b() {}\n");

    let out = run(root, &["codemap", "ls", "--json"]);
    assert!(out.status.success());
    let docs: Value = serde_json::from_slice(&out.stdout).unwrap();
    let arr = docs.as_array().unwrap();
    assert_eq!(arr.len(), 2);
    let domains: Vec<&str> = arr.iter().map(|d| d["domain"].as_str().unwrap()).collect();
    assert!(domains.contains(&"alpha"));
    assert!(domains.contains(&"beta"));
    // Paths are repo-relative.
    assert_eq!(arr[0]["root"], "src/alpha");
}

#[test]
fn ls_excludes_root_level_index_codemap() {
    let tmp = tempdir().unwrap();
    let root = tmp.path();
    write_marker(root);
    write_file(root, "src/alpha/CODEMAP.md", &map("alpha", "src/alpha", None));
    write_file(root, "src/alpha/a.rs", "pub fn a() {}\n");
    // A root-level CODEMAP.md is the rendered index — not a domain map.
    write_file(root, "CODEMAP.md", "# Codemap Index\n");

    let out = run(root, &["codemap", "ls", "--json"]);
    let docs: Value = serde_json::from_slice(&out.stdout).unwrap();
    let arr = docs.as_array().unwrap();
    assert_eq!(arr.len(), 1, "root CODEMAP.md must not be discovered as a domain map");
    assert_eq!(arr[0]["domain"], "alpha");
}

// ── check ───────────────────────────────────────────────────────────────────

#[test]
fn check_flags_dangling_and_orphan_and_strict_exits_nonzero() {
    let tmp = tempdir().unwrap();
    let root = tmp.path();
    write_marker(root);
    // A covered, valid map.
    write_file(root, "src/mapped/CODEMAP.md", &map("mapped", "src/mapped", None));
    write_file(root, "src/mapped/a.rs", "pub fn a() {}\n");
    // A dangling map: root points at a directory that does not exist.
    write_file(root, "src/dangling/CODEMAP.md", &map("dangling", "src/gone", None));
    // An uncovered subtree above the default threshold (5 code files).
    for f in ["a", "b", "c", "d", "e"] {
        write_file(root, &format!("src/orphan/{f}.rs"), "pub fn x() {}\n");
    }

    let out = run(root, &["codemap", "check", "--json"]);
    let findings: Value = serde_json::from_slice(&out.stdout).unwrap();
    let arr = findings.as_array().unwrap();

    let has_dangling = arr.iter().any(|f| f["kind"] == "dangling" && f["root"] == "src/gone");
    let has_orphan = arr
        .iter()
        .any(|f| f["kind"] == "orphan" && f["dir"] == "src/orphan" && f["file_count"] == 5);
    assert!(has_dangling, "expected a dangling finding: {arr:#?}");
    assert!(has_orphan, "expected an orphan finding: {arr:#?}");

    // --strict turns actionable findings into a non-zero exit.
    let strict = run(root, &["codemap", "check", "--strict"]);
    assert!(!strict.status.success(), "strict must exit non-zero when findings exist");
}

#[test]
fn check_clean_tree_strict_exits_zero() {
    let tmp = tempdir().unwrap();
    let root = tmp.path();
    write_marker(root);
    write_file(root, "src/only/CODEMAP.md", &map("only", "src/only", None));
    // Cover every code file so there are no gaps.
    write_file(root, "src/only/a.rs", "pub fn a() {}\n");

    let strict = run(root, &["codemap", "check", "--strict"]);
    assert!(strict.status.success(), "clean tree should pass --strict");
}

// ── render ──────────────────────────────────────────────────────────────────

#[test]
fn render_emits_index_with_table_and_coverage() {
    let tmp = tempdir().unwrap();
    let root = tmp.path();
    write_marker(root);
    write_file(root, "src/alpha/CODEMAP.md", &map("alpha", "src/alpha", None));
    write_file(root, "src/alpha/a.rs", "pub fn a() {}\n");
    for f in ["a", "b", "c", "d", "e"] {
        write_file(root, &format!("src/gap/{f}.rs"), "pub fn x() {}\n");
    }

    let out = run(root, &["codemap", "render"]);
    assert!(out.status.success());
    let md = String::from_utf8(out.stdout).unwrap();
    assert!(md.contains("# Codemap Index"));
    assert!(md.contains("[alpha](src/alpha/CODEMAP.md)"));
    assert!(md.contains("`src/gap` — 5 uncovered code file(s)"));
}

// ── staleness (git) ───────────────────────────────────────────────────────────

#[test]
fn check_staleness_stale_vs_fresh_with_git() {
    let tmp = tempdir().unwrap();
    let root = tmp.path();
    if !git(root, &["init"]).status.success() {
        eprintln!("git unavailable — skipping staleness test");
        return;
    }
    write_marker(root);
    write_file(root, "src/changed/f.rs", "pub fn f() {}\n");
    write_file(root, "src/stable/g.rs", "pub fn g() {}\n");
    git_commit_all(root, "v1");
    let pin = short_head(root);

    // Pin both maps to v1, then change only src/changed.
    write_file(root, "src/changed/CODEMAP.md", &map("changed", "src/changed", Some(&pin)));
    write_file(root, "src/stable/CODEMAP.md", &map("stable", "src/stable", Some(&pin)));
    write_file(root, "src/changed/f.rs", "pub fn f() { let _ = 1; }\n");
    git_commit_all(root, "v2");

    let out = run(root, &["codemap", "check", "--stale", "--json"]);
    let findings: Value = serde_json::from_slice(&out.stdout).unwrap();
    let arr = findings.as_array().unwrap();

    // src/changed is stale (its code changed since the pin)...
    let changed_stale = arr
        .iter()
        .any(|f| f["kind"] == "stale" && f["root"] == "src/changed");
    assert!(changed_stale, "src/changed should be stale: {arr:#?}");

    // ...but src/stable is NOT stale — only its own CODEMAP.md landed after the pin,
    // which is excluded from the staleness diff.
    let stable_stale = arr
        .iter()
        .any(|f| f["kind"] == "stale" && f["root"] == "src/stable");
    assert!(!stable_stale, "src/stable must not be stale (self-CODEMAP.md excluded): {arr:#?}");
}

#[test]
fn check_staleness_unverifiable_without_commit_pin() {
    let tmp = tempdir().unwrap();
    let root = tmp.path();
    write_marker(root);
    // No commit pin and no git repo.
    write_file(root, "src/x/CODEMAP.md", &map("x", "src/x", None));
    write_file(root, "src/x/a.rs", "pub fn a() {}\n");

    let out = run(root, &["codemap", "check", "--stale", "--json"]);
    let findings: Value = serde_json::from_slice(&out.stdout).unwrap();
    let arr = findings.as_array().unwrap();
    let unverifiable = arr.iter().any(|f| f["kind"] == "unverifiable" && f["file"] == "src/x/CODEMAP.md");
    assert!(unverifiable, "missing commit pin should be unverifiable: {arr:#?}");

    // Unverifiable is advisory — it must not fail --strict.
    let strict = run(root, &["codemap", "check", "--stale", "--strict"]);
    assert!(strict.status.success(), "unverifiable alone should not fail --strict");
}
