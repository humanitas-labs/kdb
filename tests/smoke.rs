mod common;
use common::*;

#[test]
fn root_and_version() {
    let ws = TestWorkspace::new();
    ws.write("a/b.md", "# b");
    let out = ws.kdb2_in(&ws.path("a"), &["root"]);
    assert!(out.status.success(), "{}", stderr(&out));
    assert_eq!(stdout(&out).trim(), ws.root.to_string_lossy());
    let out = ws.kdb2(&["--version"]);
    assert!(stdout(&out).starts_with("kdb2 2.0.0"));
}
