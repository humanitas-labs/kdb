//! Markdown graph parity tests, ported from v1 `tests/index.rs` and `tests/render.rs`,
//! exercised through the `kdb2` binary (`check --orphans`, `refs --json`, `outline --json`, `render`).
mod common;
use common::*;

use serde_json::Value;

fn check(ws: &TestWorkspace) -> (i32, String) {
    let out = ws.kdb2(&["check", "--orphans"]);
    (out.status.code().unwrap_or(-1), stdout(&out))
}

#[test]
fn headings_links_and_wikilinks_parse_with_v1_shapes() {
    let ws = TestWorkspace::new();
    ws.write(
        "a.md",
        "# Intro\n\n## Sub Heading\n\n[a](notes/a.md#Heading)\n[[wiki/page#Section]]\n[[wiki/page|Alias]]\n[ext](https://example.com)\n[mail](mailto:x@y.z)\n[txt](notes/a.txt)\n",
    );
    let (code, out) = check(&ws);
    assert_eq!(code, 1);
    assert_eq!(
        out,
        "a.md:5:1 broken link notes/a.md#Heading (target file not found: notes/a.md)\n\
         a.md:6:1 broken link [[wiki/page#Section]] (target file not found: wiki/page.md)\n\
         a.md:7:1 broken link [[wiki/page|Alias]] (target file not found: wiki/page.md)\n\
         a.md orphan file (0 inbound links)\n3 errors\n1 warning\n"
    );
}

#[test]
fn outline_rows_dedup_anchors_and_normalize_titles() {
    let ws = TestWorkspace::new();
    ws.write(
        "p.md",
        "---\ntitle: x\n---\n\n# Same\n## Same\n### Same\n## The `useState` Hook\n## See [Overview](overview.md)\n## What's New in v2.0?\n## [[a/b#c|Alias]] and [[#local]]\n## !!!\n\nSetext\n======\n",
    );
    let out = ws.kdb2(&["outline", "--json", "p.md"]);
    assert!(out.status.success(), "{}", stderr(&out));
    let rows: Vec<Value> = serde_json::from_str(&stdout(&out)).unwrap();
    let got: Vec<(String, String, u64, u64)> = rows
        .iter()
        .map(|r| {
            (
                r["name"].as_str().unwrap().to_string(),
                r["anchor"].as_str().unwrap().to_string(),
                r["level"].as_u64().unwrap(),
                r["line"].as_u64().unwrap(),
            )
        })
        .collect();
    let s = |a: &str, b: &str, l: u64, n: u64| (a.to_string(), b.to_string(), l, n);
    assert_eq!(
        got,
        vec![
            s("Same", "same", 1, 5),
            s("Same", "same-1", 2, 6),
            s("Same", "same-2", 3, 7),
            s("The useState Hook", "the-usestate-hook", 2, 8),
            s("See Overview", "see-overview", 2, 9),
            s("What\u{2019}s New in v2.0?", "whats-new-in-v20", 2, 10),
            s("Alias and local", "alias-and-local", 2, 11),
            s("!!!", "section", 2, 12),
            s("Setext", "setext", 1, 14),
        ]
    );
    assert_eq!(rows[0]["kind"], "heading");
    assert_eq!(rows[0]["public"], true);
}

#[test]
fn multiple_links_on_one_line_have_distinct_columns() {
    let ws = TestWorkspace::new();
    ws.write("a.md", "See [One](x.md) and [Two](y.md) and [[z]]\n");
    let (_, out) = check(&ws);
    assert!(out.contains("a.md:1:5 broken link x.md"), "{out}");
    assert!(out.contains("a.md:1:21 broken link y.md"), "{out}");
    assert!(out.contains("a.md:1:37 broken link [[z]]"), "{out}");
}

#[test]
fn links_in_code_blocks_and_spans_are_ignored() {
    let ws = TestWorkspace::new();
    ws.write(
        "a.md",
        "# Title\n\n```\n[[should/ignore]]\n[x](nope.md)\n```\n\nSee `[[not/a/link]]` here.\n\n    [[indented]]\n\n[[real/link]]\n",
    );
    let (_, out) = check(&ws);
    assert_eq!(
        out,
        "a.md:12:1 broken link [[real/link]] (target file not found: real/link.md)\na.md orphan file (0 inbound links)\n1 error\n1 warning\n"
    );
}

#[test]
fn resolution_rules_match_v1() {
    let ws = TestWorkspace::new();
    ws.write("index.md", "# Index\n\n## Intro\n\n[deep](a/b/c/deep.md)\n");
    ws.write("notes/topic.md", "# T\n\n[up](../index.md#Intro)\n[[refs/overview]]\n[self](#t)\n[[#t]]\n[esc](../../outside.md)\n[abs](/etc/passwd.md)\n[[other.md]]\n[sib](sibling.md)\n");
    ws.write("notes/refs/overview.md", "# O\n");
    ws.write("notes/other.md", "# Other\n");
    ws.write("a/b/c/deep.md", "# Deep\n\n[Top](kdb://index.md)\n[[kdb://index#intro]]\n[miss](kdb://nonexistent.md)\n[bad](index.md#nope)\n");
    ws.write("a/b/c/index.md", "# Local\n");
    let (code, out) = check(&ws);
    assert_eq!(code, 1);
    assert_eq!(
        out,
        "a/b/c/deep.md:5:1 broken link kdb://nonexistent.md (target file not found: nonexistent.md)\n\
         a/b/c/deep.md:6:1 broken link index.md#nope (target heading not found: a/b/c/index.md#nope)\n\
         notes/topic.md:7:1 broken link ../../outside.md (target resolves outside root)\n\
         notes/topic.md:8:1 broken link /etc/passwd.md (target resolves outside root)\n\
         notes/topic.md:10:1 broken link sibling.md (target file not found: notes/sibling.md)\n\
         notes/topic.md orphan file (0 inbound links)\n\
         5 errors\n1 warning\n"
    );
}

#[test]
fn orphans_and_inbound_follow_v1_rules() {
    let ws = TestWorkspace::new();
    ws.write("a.md", "# A\n\n[B](b.md#Target)\n[B again](b.md)\n");
    ws.write("b.md", "# B\n\n## Target\n\n[A](a.md)\n");
    ws.write("self.md", "# Self\n\n[top](#self)\n");
    ws.write("empty.md", "");
    ws.write("c.md", "# C\n\n[empty](empty.md)\n[[empty]]\n");
    let (code, out) = check(&ws);
    assert_eq!(code, 0);
    assert_eq!(out, "c.md orphan file (0 inbound links)\nself.md orphan file (0 inbound links)\n2 warnings\n");

    let out = ws.kdb2(&["refs", "b.md#target", "--json"]);
    let rows: Vec<Value> = serde_json::from_str(&stdout(&out)).unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["source_file"], "a.md");
    assert_eq!(rows[0]["line"], 3);
    assert_eq!(rows[0]["column"], 1);
    assert_eq!(rows[0]["raw"], "b.md#Target");

    let out = ws.kdb2(&["refs", "empty.md", "--count"]);
    assert_eq!(stdout(&out).trim(), "2");
}

#[test]
fn check_honors_gitignore_config_ignore_and_non_markdown() {
    let ws = TestWorkspace::new();
    ws.write(".kdb/config.toml", "[workspace]\nname = \"t\"\n[index]\nignore = [\"archive/**\"]\n");
    ws.write(".gitignore", "private/\n");
    ws.write("docs/.gitignore", "*.md\n!keep.md\n");
    ws.write("a.md", "# A\n\n[B](b.md)\n");
    ws.write("b.md", "# B\n\n[A](a.md)\n");
    ws.write("notes.txt", "[x](missing.md)");
    ws.write("archive/bad.md", "[x](missing.md)\n");
    ws.write("private/bad.md", "[x](missing.md)\n");
    ws.write("docs/drop.md", "[x](missing.md)\n");
    ws.write("docs/keep.md", "[x](../a.md)\n");
    let (code, out) = check(&ws);
    assert_eq!(code, 0);
    assert_eq!(out, "docs/keep.md orphan file (0 inbound links)\n1 warning\n");
}

#[test]
fn render_resolves_embeds_recursively() {
    let ws = TestWorkspace::new();
    ws.write("note.md", "# Hello\n\nNo embeds here.\n");
    ws.write("sop.md", "# SOP\n\n## Setup\n\nDo the setup.\n\n### Sub\n\nmore\n\n## Teardown\n\nClean up.\n");
    ws.write("lib/snippet.md", "snippet content\n");
    ws.write("lib/main.md", "![[snippet.md]]\n");
    ws.write("glossary.md", "definitions here\n");
    ws.write("c.md", "leaf\n");
    ws.write("b.md", "![[c.md]]\n");
    ws.write("a.md", "![[b.md]]\n");
    ws.write(
        "main.md",
        "start\n![[sop.md#Setup]]\nmiddle\n![[kdb://lib/snippet.md]]\n![[glossary]]\nsee ![[c.md]] inline\n![[a.md|alias]]\n```markdown\n![[nonexistent.md]]\n```\nend\n",
    );
    let r = |f: &str| {
        let out = ws.kdb2(&["render", f]);
        assert!(out.status.success(), "{}", stderr(&out));
        stdout(&out)
    };
    assert_eq!(r("note.md"), "# Hello\n\nNo embeds here.\n");
    assert_eq!(r("lib/main.md"), "snippet content\n");
    assert_eq!(
        r("main.md"),
        "start\n## Setup\n\nDo the setup.\n\n### Sub\n\nmore\nmiddle\nsnippet content\ndefinitions here\nsee ![[c.md]] inline\nleaf\n```markdown\n![[nonexistent.md]]\n```\nend\n"
    );
}

#[test]
fn render_errors_match_v1_wording() {
    let ws = TestWorkspace::new();
    ws.write("a.md", "![[b.md]]\n");
    ws.write("b.md", "![[a.md]]\n");
    ws.write("missing.md", "![[nonexistent.md]]\n");
    ws.write("sop.md", "# SOP\n\n## Setup\n");
    ws.write("heading.md", "![[sop.md#nonexistent]]\n");
    let err = |f: &str| {
        let out = ws.kdb2(&["render", f]);
        assert_eq!(out.status.code(), Some(1));
        stderr(&out)
    };
    assert_eq!(err("a.md"), "error: failed to render a.md: include cycle detected: a.md -> b.md -> a.md\n");
    assert_eq!(err("missing.md"), "error: failed to render missing.md: include target file not found: nonexistent.md\n");
    assert_eq!(err("heading.md"), "error: failed to render heading.md: include target heading not found: sop.md#nonexistent\n");
}

#[test]
fn check_reports_broken_embeds_and_skips_code_blocks() {
    let ws = TestWorkspace::new();
    ws.write("sop.md", "# SOP\n\n## Real\n");
    ws.write("main.md", "# Main\n\n![[missing.md]]\n![[sop.md#fake]]\n![[sop.md#real]]\n\n```\n![[nonexistent.md]]\n```\n");
    let (code, out) = check(&ws);
    assert_eq!(code, 1);
    assert_eq!(
        out,
        "main.md:3:2 broken embed [[missing.md]] (target file not found: missing.md)\n\
         main.md:4:2 broken embed [[sop.md#fake]] (target heading not found: sop.md#fake)\n\
         main.md orphan file (0 inbound links)\n2 errors\n1 warning\n"
    );
}
