//! CLI parity tests for the planning layer, ported from v1 `tests/cli.rs`,
//! plus the v2 additions: concurrent `tasks add`, dependencies, status icons.

mod common;

use std::collections::HashSet;
use std::process::Output;

use common::{TestWorkspace, stderr, stdout};
use serde_json::Value;

fn ok(out: Output) -> String {
    assert!(out.status.success(), "stderr: {}", stderr(&out));
    stdout(&out)
}

fn fail(out: Output) -> String {
    assert!(!out.status.success(), "expected failure, got: {}", stdout(&out));
    stderr(&out)
}

fn json(out: Output) -> Value {
    serde_json::from_str(&ok(out)).expect("json")
}

/// A workspace with project `kdb` (alias KDB) at `projects/kdb`.
fn with_kdb() -> TestWorkspace {
    let ws = TestWorkspace::new();
    std::fs::create_dir_all(ws.path("projects/kdb")).unwrap();
    ok(ws.kdb(&["projects", "add", "kdb", "--alias", "KDB", "--path", "projects/kdb"]));
    ws
}

fn titles(list: &str) -> Vec<&str> {
    list.lines().skip(1).filter(|l| !l.trim().is_empty()).filter_map(|l| l.splitn(4, "  ").nth(3)).map(str::trim).collect()
}

fn ids(list: &str) -> Vec<&str> {
    list.lines().skip(1).filter_map(|l| l.split("  ").next()).collect()
}

#[test]
fn projects_add_list_show_edit() {
    let ws = with_kdb();
    assert_eq!(
        ok(ws.kdb(&["projects", "list"])),
        "slug  alias  name  status  space  path\nkdb   KDB    kdb   active  -      projects/kdb\n"
    );
    ok(ws.kdb(&["projects", "add", "other", "-a", "oth", "-p", "projects/other", "-n", "Other One", "-d", "desc"]));
    let show = ok(ws.kdb(&["projects", "show", "other"]));
    assert!(show.starts_with("slug:        other\nalias:       OTH\nname:        Other One\npath:        projects/other\nstatus:      active\nspace:       -\ndescription: desc\n"));
    ok(ws.kdb(&["projects", "edit", "other", "--status", "archived"]));
    assert!(!ok(ws.kdb(&["projects", "ls"])).contains("other"));
    assert!(ok(ws.kdb(&["projects", "list", "-a"])).contains("other"));
    let j = json(ws.kdb(&["projects", "show", "other", "--json"]));
    assert_eq!(j["status"], "archived");
    assert!(fail(ws.kdb(&["projects", "show", "nope"])).contains("project not found: nope"));
    assert!(fail(ws.kdb(&["projects", "edit", "kdb"])).contains("no fields to update"));
    assert!(fail(ws.kdb(&["projects", "add", "dup", "-a", "KDB", "-p", "x"])).contains("failed to insert project dup"));
}

#[test]
fn spaces_group_projects_and_own_tasks() {
    let ws = with_kdb();
    ok(ws.kdb(&["spaces", "add", "ice", "--alias", "ICE", "-n", "Iceberg", "--path", "ice"]));
    ok(ws.kdb(&["projects", "edit", "kdb", "--space", "ice"]));
    let list = ok(ws.kdb(&["spaces", "list"]));
    assert_eq!(list, "slug  alias  name     status  projects  path\nice   ICE    Iceberg  active         1  ice\n");
    let show = ok(ws.kdb(&["spaces", "show", "ice"]));
    assert!(show.contains("projects:    1\n"));
    assert!(show.contains("\nprojects:\nslug  alias"));
    ok(ws.kdb(&["tasks", "add", "Space task", "-S", "ice"]));
    assert!(ok(ws.kdb(&["tasks", "list", "-S", "ice"])).contains("ICE-0001"));
    ok(ws.kdb(&["projects", "edit", "kdb", "--space", ""]));
    assert_eq!(json(ws.kdb(&["spaces", "show", "ice", "--json"]))["space"]["project_count"], 0);
    assert!(fail(ws.kdb(&["projects", "add", "clash", "-a", "ICE", "-p", "p"])).contains("alias already used by a space"));
    assert!(fail(ws.kdb(&["tasks", "add", "x", "-S", "nope"])).contains("space not found: nope"));
}

#[test]
fn tasks_resolve_active_project_from_cwd() {
    let ws = with_kdb();
    std::fs::create_dir_all(ws.path("projects/kdb/src")).unwrap();
    assert_eq!(ok(ws.kdb_in(&ws.path("projects/kdb/src"), &["tasks", "add", "From cwd"])), "added task KDB-0001\n");
    let err = fail(ws.kdb(&["tasks", "add", "No project"]));
    assert!(err.contains("no project for current directory"), "{err}");
}

#[test]
fn tasks_view_supports_show_alias_and_orders_children_by_order_key() {
    let ws = with_kdb();
    ok(ws.kdb(&["tasks", "add", "Parent", "-P", "kdb"]));
    ok(ws.kdb(&["tasks", "add", "Child A", "-P", "kdb", "--parent", "KDB-0001"]));
    ok(ws.kdb(&["tasks", "add", "Child B", "-P", "kdb", "--parent", "KDB-0001"]));
    let conn = ws.db();
    conn.execute("UPDATE tasks SET \"order\" = 'b' WHERE child_seq = 1", []).unwrap();
    conn.execute("UPDATE tasks SET \"order\" = 'a' WHERE child_seq = 2", []).unwrap();
    let view = ok(ws.kdb(&["tasks", "view", "KDB-0001"]));
    assert!(view.contains("children:"));
    assert!(view.find("KDB-0001.2").unwrap() < view.find("KDB-0001.1").unwrap());
    assert!(ok(ws.kdb(&["tasks", "show", "KDB-0001"])).contains("children:"));
    let j = json(ws.kdb(&["tasks", "view", "KDB-0001", "--json"]));
    let children = j["children"].as_array().unwrap();
    assert_eq!(children.len(), 2);
    assert_eq!(children[0]["id"], "KDB-0001.2");
    assert_eq!(children[0]["order"], "a");
    assert_eq!(children[1]["id"], "KDB-0001.1");
    let child = ok(ws.kdb(&["tasks", "view", "kdb-1.1"]));
    assert!(child.starts_with("id:         KDB-0001.1\ntitle:      Child A\nstatus:     backlog\npriority:   3\norder:      b\nproject:    kdb\n"));
}

#[test]
fn tasks_delete_soft_hides_from_list_and_restore_brings_back() {
    let ws = with_kdb();
    ok(ws.kdb(&["tasks", "add", "Delete me", "-P", "kdb"]));
    assert_eq!(ok(ws.kdb(&["tasks", "delete", "KDB-0001"])), "deleted KDB-0001\n");
    assert_eq!(json(ws.kdb(&["tasks", "list", "--json"])).as_array().unwrap().len(), 0);
    let view = json(ws.kdb(&["tasks", "view", "KDB-0001", "--json"]));
    assert_eq!(view["status"], "backlog");
    assert!(!view["deleted_at"].is_null());
    assert_eq!(ok(ws.kdb(&["tasks", "restore", "KDB-0001"])), "restored KDB-0001\n");
    assert!(json(ws.kdb(&["tasks", "view", "KDB-0001", "--json"]))["deleted_at"].is_null());
    ok(ws.kdb(&["tasks", "add", "Two", "-P", "kdb"]));
    ok(ws.kdb(&["tasks", "d", "KDB-0002"]));
    ok(ws.kdb(&["tasks", "add", "Three", "-P", "kdb"]));
    ok(ws.kdb(&["tasks", "rm", "KDB-0003"]));
    assert_eq!(ok(ws.kdb(&["tasks"])), "id        st    p  title\nKDB-0001  [ ]  3  Delete me\n");
}

#[test]
fn tasks_delete_hard_removes_row() {
    let ws = with_kdb();
    ok(ws.kdb(&["tasks", "add", "Goner", "-P", "kdb"]));
    assert_eq!(ok(ws.kdb(&["tasks", "delete", "KDB-0001", "--hard"])), "hard-deleted KDB-0001\n");
    assert!(fail(ws.kdb(&["tasks", "view", "KDB-0001", "--json"])).contains("task not found: KDB-0001"));
}

#[test]
fn tasks_purge_done_clears_done_tasks() {
    let ws = with_kdb();
    ok(ws.kdb(&["tasks", "add", "A", "-P", "kdb"]));
    ok(ws.kdb(&["tasks", "add", "B", "-P", "kdb"]));
    assert_eq!(ok(ws.kdb(&["tasks", "done", "KDB-0001"])), "KDB-0001 -> done\n");
    let dry = ok(ws.kdb(&["tasks", "purge", "--status", "done", "--dry-run"]));
    assert_eq!(dry, "would purge 1 tasks:\n  KDB-0001  A\n");
    ok(ws.kdb(&["tasks", "view", "KDB-0001"]));
    assert_eq!(ok(ws.kdb(&["tasks", "purge", "--status", "done"])), "purged 1 tasks:\n  KDB-0001  A\n");
    fail(ws.kdb(&["tasks", "view", "KDB-0001"]));
    assert_eq!(ok(ws.kdb(&["tasks", "purge", "--status", "done"])), "(no tasks matched)\n");
    assert!(fail(ws.kdb(&["tasks", "purge"])).contains("purge requires at least one selector"));
}

#[test]
fn tasks_move_before_reorders_project_list() {
    let ws = with_kdb();
    for t in ["a", "b", "c"] {
        ok(ws.kdb(&["tasks", "add", t, "-P", "kdb"]));
    }
    let mv = ok(ws.kdb(&["tasks", "move", "KDB-0003", "--before", "KDB-0001"]));
    assert!(mv.starts_with("moved KDB-0003 (order="), "{mv}");
    assert_eq!(titles(&ok(ws.kdb(&["tasks", "list", "-P", "kdb"]))), vec!["c", "a", "b"]);
}

#[test]
fn tasks_move_top_and_bottom_moves_to_ends() {
    let ws = with_kdb();
    for t in ["a", "b", "c"] {
        ok(ws.kdb(&["tasks", "add", t, "-P", "kdb"]));
    }
    ok(ws.kdb(&["tasks", "mv", "KDB-0001", "--bottom"]));
    assert_eq!(titles(&ok(ws.kdb(&["tasks", "list", "-P", "kdb"]))), vec!["b", "c", "a"]);
    ok(ws.kdb(&["tasks", "move", "KDB-0001", "--top"]));
    assert_eq!(titles(&ok(ws.kdb(&["tasks", "list", "-P", "kdb"]))), vec!["a", "b", "c"]);
    ok(ws.kdb(&["tasks", "move", "KDB-0002", "--after", "KDB-0003"]));
    assert_eq!(titles(&ok(ws.kdb(&["tasks", "list", "-P", "kdb"]))), vec!["a", "c", "b"]);
    assert!(fail(ws.kdb(&["tasks", "move", "KDB-0002", "--top", "--bottom"])).contains("cannot be used with"));
}

#[test]
fn tasks_add_after_inserts_between_neighbors_and_adds_dependency() {
    let ws = with_kdb();
    ok(ws.kdb(&["tasks", "add", "a", "-P", "kdb"]));
    ok(ws.kdb(&["tasks", "add", "b", "-P", "kdb"]));
    ok(ws.kdb(&["tasks", "add", "mid", "-P", "kdb", "--after", "KDB-0001"]));
    let list = ok(ws.kdb(&["tasks", "list", "-P", "kdb"]));
    assert_eq!(titles(&list), vec!["a", "mid  (blocked by KDB-0001)", "b"]);
    ok(ws.kdb(&["tasks", "add", "first", "-P", "kdb", "--before", "KDB-0001"]));
    assert_eq!(ids(&ok(ws.kdb(&["tasks", "list", "-P", "kdb"]))), vec!["KDB-0004", "KDB-0001", "KDB-0003", "KDB-0002"]);
    assert!(fail(ws.kdb(&["tasks", "add", "x", "-P", "kdb", "--before", "KDB-0099"])).contains("anchor task not found: KDB-0099"));
}

#[test]
fn tasks_list_hides_children_by_default_and_include_children_shows_them() {
    let ws = with_kdb();
    ok(ws.kdb(&["tasks", "add", "Parent", "-P", "kdb"]));
    ok(ws.kdb(&["tasks", "add", "Child A", "-P", "kdb", "--parent", "KDB-0001"]));
    let list = ok(ws.kdb(&["tasks", "list", "-P", "kdb"]));
    assert!(list.contains("KDB-0001") && !list.contains("KDB-0001.1"));
    let list = ok(ws.kdb(&["tasks", "list", "-P", "kdb", "--include-children"]));
    assert!(list.contains("KDB-0001.1") && list.contains("Child A"));
}

#[test]
fn tasks_move_rejects_cross_parent_target() {
    let ws = with_kdb();
    ok(ws.kdb(&["tasks", "add", "parent", "-P", "kdb"]));
    ok(ws.kdb(&["tasks", "add", "child", "-P", "kdb", "--parent", "KDB-0001"]));
    ok(ws.kdb(&["tasks", "add", "root", "-P", "kdb"]));
    let err = fail(ws.kdb(&["tasks", "move", "KDB-0001.1", "--before", "KDB-0002"]));
    assert!(err.contains("different parent"), "unexpected stderr: {err}");
}

#[test]
fn tasks_edit_status_filters_and_labels() {
    let ws = with_kdb();
    ok(ws.kdb(&["cycles", "add", "C-1", "-s", "2026-01-01", "-e", "2026-01-14"]));
    ok(ws.kdb(&["tasks", "add", "A", "-P", "kdb", "-p", "1", "-b", "body text"]));
    ok(ws.kdb(&["tasks", "add", "B", "-P", "kdb"]));
    assert_eq!(ok(ws.kdb(&["tasks", "edit", "KDB-0002", "-t", "B2", "-c", "C-1", "-s", "in_progress"])), "updated task KDB-0002\n");
    assert_eq!(ok(ws.kdb(&["tasks", "list", "-c", "C-1"])), "id        st    p  title\nKDB-0002  [~]  3  B2\n");
    assert_eq!(ids(&ok(ws.kdb(&["tasks", "list", "-p", "1"]))), vec!["KDB-0001"]);
    assert_eq!(ids(&ok(ws.kdb(&["tasks", "list", "-s", "in_progress"]))), vec!["KDB-0002"]);
    assert!(fail(ws.kdb(&["tasks", "list", "-s", "bogus"])).contains("invalid status 'bogus'"));
    assert!(fail(ws.kdb(&["tasks", "edit", "KDB-0001"])).contains("no fields to update"));
    assert!(fail(ws.kdb(&["tasks", "edit", "KDB-0001", "-p", "9"])).contains("priority must be between 1 and 5"));
    assert!(fail(ws.kdb(&["tasks", "edit", "KDB-0001", "--parent", "KDB-0001"])).contains("cannot make a task its own parent"));
    ok(ws.kdb(&["tasks", "edit", "KDB-0002", "--parent", "KDB-0001"]));
    assert_eq!(json(ws.kdb(&["tasks", "view", "KDB-0001.1", "--json"]))["title"], "B2");
    ok(ws.kdb(&["tasks", "edit", "KDB-0001.1", "--parent", ""]));
    assert_eq!(json(ws.kdb(&["tasks", "view", "KDB-0002", "--json"]))["title"], "B2");

    assert_eq!(ok(ws.kdb(&["tasks", "label", "add", "KDB-0001", "feat", "bug"])), "attached 2 label(s) to KDB-0001\n");
    let view = ok(ws.kdb(&["tasks", "view", "KDB-0001"]));
    assert!(view.contains("labels:     bug, feat\n") && view.ends_with("\nbody text\n"), "{view}");
    assert_eq!(ok(ws.kdb(&["labels", "list"])), "slug  name  color\nbug   bug   \nfeat  feat  \n");
    assert_eq!(ok(ws.kdb(&["tasks", "label", "rm", "KDB-0001", "feat", "nope"])), "detached 1 label(s) from KDB-0001\n");
    assert_eq!(ok(ws.kdb(&["tasks", "park", "KDB-0001"])), "KDB-0001 -> parked\n");
    assert!(json(ws.kdb(&["tasks", "view", "KDB-0001", "--json"]))["closed_at"].is_null());
    assert_eq!(ok(ws.kdb(&["tasks", "reopen", "KDB-0001"])), "KDB-0001 -> backlog\n");
    ok(ws.kdb(&["tasks", "done", "KDB-0001"]));
    assert!(ok(ws.kdb(&["tasks", "view", "KDB-0001"])).contains("closed_at:  "));
    assert!(fail(ws.kdb(&["tasks", "view", "KDB"])).contains("invalid task id 'KDB'"));
}

#[test]
fn tasks_add_concurrently_assigns_distinct_seqs() {
    let ws = with_kdb();
    let bin = env!("CARGO_BIN_EXE_kdb");
    let children: Vec<_> = (0..12)
        .map(|i| {
            std::process::Command::new(bin)
                .args(["tasks", "add", &format!("parallel {i}"), "-P", "kdb"])
                .current_dir(&ws.root)
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::piped())
                .spawn()
                .unwrap()
        })
        .collect();
    let mut seen = HashSet::new();
    for child in children {
        let out = child.wait_with_output().unwrap();
        let line = ok(out);
        assert!(seen.insert(line.trim().to_string()), "duplicate id: {line}");
    }
    assert_eq!(seen.len(), 12);
    assert_eq!(json(ws.kdb(&["tasks", "list", "--json"])).as_array().unwrap().len(), 12);
}

#[test]
fn deps_block_ready_and_release() {
    let ws = with_kdb();
    ok(ws.kdb(&["projects", "add", "other", "-a", "OTH", "-p", "projects/other"]));
    for t in ["gate", "work", "later"] {
        ok(ws.kdb(&["tasks", "add", t, "-P", "kdb"]));
    }
    ok(ws.kdb(&["tasks", "add", "cross", "-P", "other", "--after", "KDB-0001"]));
    ok(ws.kdb(&["tasks", "add", "sub", "-P", "kdb", "--parent", "KDB-0002"]));
    assert_eq!(ok(ws.kdb(&["tasks", "deps", "add", "KDB-0002", "KDB-0001"])), "KDB-0002 now depends on 1 (blocks)\n");
    ok(ws.kdb(&["tasks", "deps", "add", "KDB-0003", "KDB-0002", "--kind", "related"]));
    let err = fail(ws.kdb(&["tasks", "deps", "add", "KDB-0001", "KDB-0002"]));
    assert!(err.contains("dependency cycle: KDB-0001 -> KDB-0002 -> KDB-0001"), "{err}");
    assert!(fail(ws.kdb(&["tasks", "deps", "add", "KDB-0001", "KDB-0001"])).contains("cannot depend on itself"));

    assert_eq!(ids(&ok(ws.kdb(&["tasks", "ready"]))), vec!["KDB-0001", "KDB-0003"]);
    assert_eq!(ids(&ok(ws.kdb(&["tasks", "list", "--blocked", "--include-children"]))), vec!["KDB-0002.1", "KDB-0002", "OTH-0001"]);
    assert_eq!(ids(&ok(ws.kdb(&["tasks", "list", "--ready", "-P", "kdb"]))), vec!["KDB-0001", "KDB-0003"]);
    assert_eq!(json(ws.kdb(&["tasks", "view", "KDB-0002.1", "--json"]))["blocked"], true);
    let show = ok(ws.kdb(&["tasks", "deps", "show", "KDB-0002"]));
    assert_eq!(show, "KDB-0002  blocked  work\n\nblocked by:\n- KDB-0001  [ ]  p3  gate\n\nblocks:\n- KDB-0003  [ ]  p3  later  (related)\n");
    let view = ok(ws.kdb(&["tasks", "view", "KDB-0002"]));
    assert!(view.contains("status:     backlog\nblocked:    yes\n"));
    assert!(view.ends_with("\nblocked by:\n- KDB-0001  [ ]  p3  gate\n\nblocks:\n- KDB-0003  [ ]  p3  later  (related)\n"), "{view}");

    assert_eq!(ok(ws.kdb(&["tasks", "park", "KDB-0001"])), "KDB-0001 -> parked\n");
    assert_eq!(ids(&ok(ws.kdb(&["tasks", "ready", "-P", "kdb"]))), vec!["KDB-0003"]);
    assert_eq!(ok(ws.kdb(&["tasks", "done", "KDB-0001"])), "released: KDB-0002, OTH-0001\nKDB-0001 -> done\n");
    assert_eq!(ids(&ok(ws.kdb(&["tasks", "ready", "-n", "1"]))), vec!["KDB-0002.1"]);
    assert_eq!(json(ws.kdb(&["tasks", "ready", "--json"])).as_array().unwrap().len(), 4);
    assert_eq!(ok(ws.kdb(&["tasks", "deps", "rm", "KDB-0003", "KDB-0002", "KDB-0001"])), "removed 1 dependency(ies) from KDB-0003\n");
    assert_eq!(ok(ws.kdb(&["tasks", "deps", "show", "KDB-0001"])), "KDB-0001  unblocked  gate\n\nblocked by:\n  (none)\n\nblocks:\n- KDB-0002  [ ]  p3  work\n- OTH-0001  [ ]  p3  cross\n");
}

#[test]
fn cycles_add_list_show_edit() {
    let ws = TestWorkspace::new();
    assert_eq!(ok(ws.kdb(&["cycles", "list"])), "(no cycles)\n");
    assert_eq!(
        ok(ws.kdb(&["cycles", "add", "C-1", "-s", "2026-01-01", "-e", "2026-01-14", "-d", "first"])),
        "added cycle C-1 (2026-01-01 → 2026-01-14)\n"
    );
    ok(ws.kdb(&["cycles", "add", "C-2", "-s", "2026-01-15", "-e", "2026-01-28", "--status", "active", "-p", ".plan/cycle/c-2"]));
    assert_eq!(
        ok(ws.kdb(&["cycles", "ls"])),
        "key  status   start       end         description\nC-2  active   2026-01-15  2026-01-28  \nC-1  planned  2026-01-01  2026-01-14  first\n"
    );
    assert_eq!(ok(ws.kdb(&["cycles", "edit", "C-1", "--status", "done"])), "updated cycle C-1\n");
    let show = ok(ws.kdb(&["cycles", "show", "C-2"]));
    assert!(show.starts_with("key:         C-2\nstatus:      active\nstart_date:  2026-01-15\nend_date:    2026-01-28\npath:        .plan/cycle/c-2\ncreated_at:  "));
    assert_eq!(json(ws.kdb(&["cycles", "show", "C-1", "--json"]))["status"], "done");
    assert!(fail(ws.kdb(&["cycles", "add", "C-3", "-s", "x", "-e", "y", "--status", "weird"])).contains("invalid value"));
    assert!(fail(ws.kdb(&["cycles", "show", "C-9"])).contains("cycle not found: C-9"));
}

#[test]
fn labels_add_list_show_edit() {
    let ws = TestWorkspace::new();
    assert_eq!(ok(ws.kdb(&["labels", "list"])), "(no labels)\n");
    assert_eq!(ok(ws.kdb(&["labels", "add", "feat", "-n", "Feature", "-c", "#00ff00"])), "added label feat\n");
    ok(ws.kdb(&["labels", "add", "bug"]));
    assert_eq!(ok(ws.kdb(&["labels", "ls"])), "slug  name     color\nbug   bug      \nfeat  Feature  #00ff00\n");
    assert_eq!(ok(ws.kdb(&["labels", "edit", "bug", "-n", "Bug"])), "updated label bug\n");
    assert_eq!(ok(ws.kdb(&["labels", "show", "feat"])), "slug:  feat\nname:  Feature\ncolor: #00ff00\n");
    assert_eq!(json(ws.kdb(&["labels", "show", "bug", "--json"]))["name"], "Bug");
    assert!(fail(ws.kdb(&["labels", "edit", "bug"])).contains("no fields to update"));
    assert!(fail(ws.kdb(&["labels", "show", "x"])).contains("label not found: x"));
}

#[test]
fn statuses_list_add_edit_rm_with_icons() {
    let ws = TestWorkspace::new();
    let list = ok(ws.kdb(&["statuses", "list", "--tasks"]));
    assert!(list.starts_with("slug         name         closed  hidden  icon         color\nin_progress  In Progress  no      no      in_progress  \n"), "{list}");
    assert!(list.contains("parked       Parked       no      yes                  \n"));
    assert_eq!(
        ok(ws.kdb(&["statuses", "list", "--projects"])),
        "slug      name      archived  hidden  color\nactive    Active    no        no      \npaused    Paused    no        no      \narchived  Archived  yes       no      \n"
    );
    assert_eq!(
        ok(ws.kdb(&["statuses", "add", "in_review", "--tasks", "-n", "In Review", "--icon", "in_review", "--order", "35", "-c", "#112233"])),
        "added status in_review\n"
    );
    let show = ok(ws.kdb(&["statuses", "show", "in_review", "--tasks"]));
    assert_eq!(show, "slug:        in_review\nname:        In Review\nclosed:      no\nhidden:      no\nsort_order:  35\ncolor:       #112233\nicon:        in_review\n");
    assert_eq!(json(ws.kdb(&["statuses", "show", "in_review", "--tasks", "--json"]))["icon"], "in_review");
    assert_eq!(ok(ws.kdb(&["statuses", "edit", "in_review", "--tasks", "--icon", "", "--closed", "--hidden", "1"])), "updated status in_review\n");
    let j = json(ws.kdb(&["statuses", "show", "in_review", "--tasks", "--json"]));
    assert!(j.get("icon").is_none() && j["flag"] == true && j["is_hidden"] == true, "{j}");
    assert!(fail(ws.kdb(&["statuses", "add", "x", "--projects", "--icon", "y"])).contains("--icon applies only to task statuses"));
    assert!(fail(ws.kdb(&["statuses", "add", "x", "--projects", "--closed"])).contains("--closed applies only to task statuses"));
    assert!(fail(ws.kdb(&["statuses", "edit", "done", "--tasks", "--archived"])).contains("apply only to project statuses"));
    assert!(fail(ws.kdb(&["statuses", "add", "x", "--tasks", "-c", "red"])).contains("invalid color 'red'"));
    assert!(fail(ws.kdb(&["statuses", "list"])).contains("required"));
    assert_eq!(ok(ws.kdb(&["statuses", "rm", "in_review", "--tasks"])), "removed status in_review\n");
    ok(ws.kdb(&["projects", "add", "p", "-a", "PP", "-p", "p"]));
    assert!(fail(ws.kdb(&["statuses", "rm", "active", "--projects"])).contains("still used by 1 projects row(s)"));
}
