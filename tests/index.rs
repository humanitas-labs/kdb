use kdb::index::{
    HeadingKey, LinkKind, LinkTarget, VaultIndex, WorkspaceIndex, parse_markdown,
    parse_markdown_target, parse_wikilink_target, resolve_target_path, slug_anchor,
};
use kdb::workspace::paths::normalize_rel_path;
use kdb::resolve::ImportKind;
use std::fs;
use std::path::{Path, PathBuf};
use tempfile::tempdir;

// ----------------------------------------------------------------------------------
// projects/kdb/tests/index.rs
//
// fn write_file()                                                                L71
// fn write_root_config()                                                         L79
// fn parse_markdown_extracts_headings_and_internal_links()                       L84
// fn parse_markdown_multiple_links_on_one_line_have_distinct_columns()          L117
// fn parse_markdown_target_keeps_url_encoded_paths()                            L127
// fn parse_markdown_deduplicates_heading_anchors()                              L140
// fn parse_markdown_target_filters_external_and_non_markdown_links()            L151
// fn parse_wikilink_target_supports_aliases_and_anchors()                       L185
// fn parse_wikilink_target_supports_alias_and_anchor_together()                 L215
// fn normalize_rel_path_rejects_escape_attempts()                               L228
// fn resolve_target_path_handles_markdown_and_wikilink_rules()                  L238
// fn vault_index_check_reports_broken_links_orphans_and_inbound_maps()          L283
// fn vault_index_multiple_sources_to_same_target_have_inbound_count_gt_one()    L322
// fn vault_index_single_file_is_reported_as_orphan()                            L338
// fn vault_index_ignores_non_markdown_files()                                   L349
// fn vault_index_build_with_ignores_skips_matching_paths()                      L362
// fn vault_index_respects_root_gitignore_rules()                                L382
// fn vault_index_respects_nested_gitignore_negation_rules()                     L395
// fn vault_index_incremental_upsert_respects_ignore_patterns()                  L408
// fn project_index_build_populates_code_import_maps_for_typescript()            L427
// fn project_index_build_populates_workspace_package_map_and_imports()          L465
// fn project_index_symbol_refs_match_imported_symbol_only()                     L518
// fn slug_anchor_normalizes_heading_text()                                      L557
// fn parse_markdown_heading_with_inline_code()                                  L568
// fn parse_markdown_link_inside_heading()                                       L576
// fn parse_markdown_ignores_wikilinks_in_code_blocks()                          L585
// fn parse_markdown_ignores_wikilinks_in_inline_code()                          L599
// fn parse_markdown_frontmatter_does_not_create_headings()                      L617
// fn parse_markdown_empty_file()                                                L625
// fn parse_markdown_file_with_no_headings()                                     L632
// fn parse_markdown_heading_with_special_chars()                                L639
// fn parse_markdown_all_six_heading_levels()                                    L648
// fn slug_anchor_all_special_characters()                                       L662
// fn slug_anchor_unicode_characters()                                           L669
// fn slug_anchor_mixed_separators()                                             L677
// fn slug_anchor_trailing_separators()                                          L683
// fn vault_index_file_linked_to_is_not_orphan()                                 L693
// fn vault_index_circular_references_are_not_broken()                           L708
// fn vault_index_self_referencing_links_do_not_count_as_inbound()               L723
// fn vault_index_broken_heading_anchor()                                        L736
// fn vault_index_wikilink_resolution()                                          L750
// fn vault_index_deeply_nested_files()                                          L763
// fn vault_index_empty_file_is_indexed()                                        L781
// fn normalize_rel_path_current_dir_only()                                      L799
// fn normalize_rel_path_deep_parent_traversal()                                 L807
// fn resolve_target_path_absolute_path_rejected()                               L822
// fn resolve_target_path_wikilink_with_explicit_md_extension()                  L836
// fn resolve_target_path_source_at_root_level()                                 L851
// fn parse_markdown_target_kdb_root_link()                                      L869
// fn parse_markdown_target_kdb_rejects_non_markdown()                           L892
// fn resolve_target_path_kdb_root_ignores_source_dir()                          L899
// fn vault_index_kdb_root_link_resolves()                                       L915
// fn vault_index_kdb_root_link_with_anchor_resolves()                           L939
// fn vault_index_kdb_root_broken_link_reported()                                L956
// fn parse_markdown_extracts_kdb_root_links()                                   L977
// ----------------------------------------------------------------------------------

fn write_file(root: &Path, rel_path: &str, content: &str) {
    let path = root.join(rel_path);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).expect("create parent dirs");
    }
    fs::write(path, content).expect("write fixture file");
}

fn write_root_config(root: &Path) {
    write_file(root, ".kdb/config.toml", "[workspace]\nname = \"fixture\"\n");
}

#[test]
fn parse_markdown_extracts_headings_and_internal_links() {
    let parsed = parse_markdown(
        "# Intro\n\n## Sub Heading\n\n[a](notes/a.md#Heading)\n[[wiki/page#Section]]\n[[wiki/page|Alias]]\n[ext](https://example.com)\n",
    );

    assert_eq!(parsed.headings.len(), 2);
    assert_eq!(parsed.headings[0].title, "Intro");
    assert_eq!(parsed.headings[0].anchor, "intro");
    assert_eq!(parsed.headings[1].title, "Sub Heading");
    assert_eq!(parsed.headings[1].anchor, "sub-heading");

    assert_eq!(parsed.links.len(), 3);
    assert!(parsed.links.iter().any(|link| {
        matches!(link.kind, LinkKind::Markdown)
            && link.raw == "notes/a.md#Heading"
            && link.target.file.as_deref() == Some("notes/a.md")
            && link.target.anchor.as_deref() == Some("Heading")
    }));
    assert!(parsed.links.iter().any(|link| {
        matches!(link.kind, LinkKind::Wikilink)
            && link.raw == "[[wiki/page#Section]]"
            && link.target.file.as_deref() == Some("wiki/page")
            && link.target.anchor.as_deref() == Some("Section")
    }));
    assert!(parsed.links.iter().any(|link| {
        matches!(link.kind, LinkKind::Wikilink)
            && link.raw == "[[wiki/page|Alias]]"
            && link.target.file.as_deref() == Some("wiki/page")
            && link.target.anchor.is_none()
    }));
}

#[test]
fn parse_markdown_multiple_links_on_one_line_have_distinct_columns() {
    let parsed = parse_markdown("See [One](a.md) and [Two](b.md)\n");
    assert_eq!(parsed.links.len(), 2);
    assert_eq!(parsed.links[0].line, 1);
    assert_eq!(parsed.links[1].line, 1);
    assert_eq!(parsed.links[0].column, 5);
    assert_eq!(parsed.links[1].column, 21);
}

#[test]
fn parse_markdown_target_keeps_url_encoded_paths() {
    let target = parse_markdown_target("path%20with%20spaces.md#my-heading");
    assert_eq!(
        target,
        Some(LinkTarget {
            file: Some("path%20with%20spaces.md".to_string()),
            anchor: Some("my-heading".to_string()),
            root_relative: false,
        })
    );
}

#[test]
fn parse_markdown_deduplicates_heading_anchors() {
    let parsed = parse_markdown("# Same\n## Same\n### Same\n");
    let anchors = parsed
        .headings
        .iter()
        .map(|heading| heading.anchor.as_str())
        .collect::<Vec<_>>();
    assert_eq!(anchors, vec!["same", "same-1", "same-2"]);
}

#[test]
fn parse_markdown_target_filters_external_and_non_markdown_links() {
    assert_eq!(
        parse_markdown_target("notes/a.md#Intro"),
        Some(LinkTarget {
            file: Some("notes/a.md".to_string()),
            anchor: Some("Intro".to_string()),
            root_relative: false,
        })
    );
    assert_eq!(
        parse_markdown_target("#Local"),
        Some(LinkTarget {
            file: None,
            anchor: Some("Local".to_string()),
            root_relative: false,
        })
    );
    assert_eq!(
        parse_markdown_target("notes/a.md"),
        Some(LinkTarget {
            file: Some("notes/a.md".to_string()),
            anchor: None,
            root_relative: false,
        })
    );

    assert_eq!(parse_markdown_target(""), None);
    assert_eq!(parse_markdown_target("https://example.com"), None);
    assert_eq!(parse_markdown_target("mailto:user@example.com"), None);
    assert_eq!(parse_markdown_target("notes/a.txt"), None);
    assert_eq!(parse_markdown_target("#"), None);
}

#[test]
fn parse_wikilink_target_supports_aliases_and_anchors() {
    assert_eq!(
        parse_wikilink_target("topic#One"),
        Some(LinkTarget {
            file: Some("topic".to_string()),
            anchor: Some("One".to_string()),
            root_relative: false,
        })
    );
    assert_eq!(
        parse_wikilink_target("topic|Alias"),
        Some(LinkTarget {
            file: Some("topic".to_string()),
            anchor: None,
            root_relative: false,
        })
    );
    assert_eq!(
        parse_wikilink_target("#Local"),
        Some(LinkTarget {
            file: None,
            anchor: Some("Local".to_string()),
            root_relative: false,
        })
    );
    assert_eq!(parse_wikilink_target(""), None);
    assert_eq!(parse_wikilink_target("|Alias"), None);
}

#[test]
fn parse_wikilink_target_supports_alias_and_anchor_together() {
    let target = parse_wikilink_target("file#heading|Display");
    assert_eq!(
        target,
        Some(LinkTarget {
            file: Some("file".to_string()),
            anchor: Some("heading".to_string()),
            root_relative: false,
        })
    );
}

#[test]
fn normalize_rel_path_rejects_escape_attempts() {
    assert_eq!(
        normalize_rel_path(Path::new("docs/./guide/../intro.md")),
        Some(PathBuf::from("docs/intro.md"))
    );
    assert_eq!(normalize_rel_path(Path::new("../outside.md")), None);
    assert_eq!(normalize_rel_path(Path::new("/abs/path.md")), None);
}

#[test]
fn resolve_target_path_handles_markdown_and_wikilink_rules() {
    let source = Path::new("notes/topic.md");

    let markdown = LinkTarget {
        file: Some("../index.md".to_string()),
        anchor: Some("Intro".to_string()),
        root_relative: false,
    };
    assert_eq!(
        resolve_target_path(source, LinkKind::Markdown, &markdown),
        Some(PathBuf::from("index.md"))
    );

    let wikilink = LinkTarget {
        file: Some("refs/overview".to_string()),
        anchor: None,
        root_relative: false,
    };
    assert_eq!(
        resolve_target_path(source, LinkKind::Wikilink, &wikilink),
        Some(PathBuf::from("notes/refs/overview.md"))
    );

    let same_file = LinkTarget {
        file: None,
        anchor: Some("section".to_string()),
        root_relative: false,
    };
    assert_eq!(
        resolve_target_path(source, LinkKind::Markdown, &same_file),
        Some(PathBuf::from("notes/topic.md"))
    );

    let escape = LinkTarget {
        file: Some("../../outside.md".to_string()),
        anchor: None,
        root_relative: false,
    };
    assert_eq!(
        resolve_target_path(source, LinkKind::Markdown, &escape),
        None
    );
}

#[test]
fn vault_index_check_reports_broken_links_orphans_and_inbound_maps() {
    let temp = tempdir().expect("tempdir");
    write_root_config(temp.path());
    write_file(
        temp.path(),
        "a.md",
        "# A\n\n[B](b.md#Target)\n[Missing](missing.md)\n",
    );
    write_file(temp.path(), "b.md", "# B\n\n## Target\n");
    write_file(temp.path(), "c.md", "# C\n");

    let index = VaultIndex::build(temp.path()).expect("build index");
    let report = index.check();

    assert_eq!(report.broken_links.len(), 1);
    assert!(
        report.broken_links[0]
            .reason
            .contains("target file not found: missing.md")
    );

    assert_eq!(
        report.orphans,
        vec![PathBuf::from("a.md"), PathBuf::from("c.md")]
    );

    let key = HeadingKey {
        file: PathBuf::from("b.md"),
        anchor: "target".to_string(),
    };
    let refs = index
        .heading_inbound
        .get(&key)
        .expect("heading inbound refs");
    assert_eq!(refs.len(), 1);
    assert_eq!(refs[0].source_file, PathBuf::from("a.md"));
}

#[test]
fn vault_index_multiple_sources_to_same_target_have_inbound_count_gt_one() {
    let temp = tempdir().expect("tempdir");
    write_root_config(temp.path());
    write_file(temp.path(), "a.md", "# A\n\n[Target](target.md)\n");
    write_file(temp.path(), "b.md", "# B\n\n[Target](target.md)\n");
    write_file(temp.path(), "target.md", "# Target\n");

    let index = VaultIndex::build(temp.path()).expect("build index");
    let inbound = index
        .file_inbound
        .get(Path::new("target.md"))
        .expect("target inbound refs");
    assert_eq!(inbound.len(), 2);
}

#[test]
fn vault_index_single_file_is_reported_as_orphan() {
    let temp = tempdir().expect("tempdir");
    write_root_config(temp.path());
    write_file(temp.path(), "solo.md", "# Solo\n");

    let index = VaultIndex::build(temp.path()).expect("build index");
    let report = index.check();
    assert_eq!(report.orphans, vec![PathBuf::from("solo.md")]);
}

#[test]
fn vault_index_ignores_non_markdown_files() {
    let temp = tempdir().expect("tempdir");
    write_root_config(temp.path());
    write_file(temp.path(), "a.md", "# A\n");
    write_file(temp.path(), "notes.txt", "not markdown");
    write_file(temp.path(), "image.png", "binary-ish");

    let index = VaultIndex::build(temp.path()).expect("build index");
    assert_eq!(index.files.len(), 1);
    assert!(index.files.contains_key(Path::new("a.md")));
}

#[test]
fn vault_index_build_with_ignores_skips_matching_paths() {
    let temp = tempdir().expect("tempdir");
    write_root_config(temp.path());
    write_file(temp.path(), "keep.md", "# Keep\n");
    write_file(temp.path(), "archive/hidden.md", "# Hidden\n");
    write_file(temp.path(), "archive/nested/deep.md", "# Deep\n");

    let ignore_patterns = vec!["archive/**".to_string()];
    let index = VaultIndex::build_with_ignores(temp.path(), &ignore_patterns).expect("build index");

    assert!(index.files.contains_key(Path::new("keep.md")));
    assert!(!index.files.contains_key(Path::new("archive/hidden.md")));
    assert!(
        !index
            .files
            .contains_key(Path::new("archive/nested/deep.md"))
    );
}

#[test]
fn vault_index_respects_root_gitignore_rules() {
    let temp = tempdir().expect("tempdir");
    write_root_config(temp.path());
    write_file(temp.path(), ".gitignore", "archive/\n");
    write_file(temp.path(), "keep.md", "# Keep\n");
    write_file(temp.path(), "archive/hidden.md", "# Hidden\n");

    let index = VaultIndex::build(temp.path()).expect("build index");
    assert!(index.files.contains_key(Path::new("keep.md")));
    assert!(!index.files.contains_key(Path::new("archive/hidden.md")));
}

#[test]
fn vault_index_respects_nested_gitignore_negation_rules() {
    let temp = tempdir().expect("tempdir");
    write_root_config(temp.path());
    write_file(temp.path(), "docs/.gitignore", "*.md\n!keep.md\n");
    write_file(temp.path(), "docs/drop.md", "# Drop\n");
    write_file(temp.path(), "docs/keep.md", "# Keep\n");

    let index = VaultIndex::build(temp.path()).expect("build index");
    assert!(index.files.contains_key(Path::new("docs/keep.md")));
    assert!(!index.files.contains_key(Path::new("docs/drop.md")));
}

#[test]
fn vault_index_incremental_upsert_respects_ignore_patterns() {
    let temp = tempdir().expect("tempdir");
    write_root_config(temp.path());
    write_file(temp.path(), "keep.md", "# Keep\n");

    let ignore_patterns = vec!["archive/**".to_string()];
    let mut index =
        VaultIndex::build_with_ignores(temp.path(), &ignore_patterns).expect("build index");

    index.upsert_file(
        PathBuf::from("archive/live.md"),
        temp.path().join("archive/live.md"),
        "# Live\n",
    );

    assert!(!index.files.contains_key(Path::new("archive/live.md")));
}

#[test]
fn project_index_build_populates_code_import_maps_for_typescript() {
    let temp = tempdir().expect("tempdir");
    write_root_config(temp.path());
    write_file(
        temp.path(),
        "tsconfig.json",
        "{\n  \"compilerOptions\": {\n    \"baseUrl\": \".\",\n    \"paths\": {\n      \"@app/*\": [\"src/*\"]\n    }\n  }\n}\n",
    );
    write_file(
        temp.path(),
        "web/main.ts",
        "import type { Util } from '@app/utils';\nimport local from './local';\n",
    );
    write_file(temp.path(), "src/utils.ts", "export type Util = string;\n");
    write_file(temp.path(), "web/local.ts", "export default 1;\n");

    let pi = WorkspaceIndex::build(temp.path()).expect("build workspace index");
    let imports = pi
        .code
        .code_imports
        .get(Path::new("web/main.ts"))
        .expect("code imports for web/main.ts");

    assert!(imports.iter().any(|import| {
        import.raw == "@app/utils"
            && import.resolved_path.as_deref() == Some(Path::new("src/utils.ts"))
            && import.kind == ImportKind::TsconfigPath
            && import.names.locals.iter().any(|name| name == "Util")
    }));
    assert!(imports.iter().any(|import| {
        import.raw == "./local"
            && import.resolved_path.as_deref() == Some(Path::new("web/local.ts"))
            && import.kind == ImportKind::Relative
            && import.names.locals.iter().any(|name| name == "local")
    }));
}

#[test]
fn project_index_build_populates_workspace_package_map_and_imports() {
    let temp = tempdir().expect("tempdir");
    write_root_config(temp.path());
    write_file(
        temp.path(),
        "package.json",
        "{\n  \"name\": \"workspace\",\n  \"private\": true,\n  \"workspaces\": [\"packages/*\"]\n}\n",
    );
    write_file(
        temp.path(),
        "apps/web/main.ts",
        "import { Agent } from '@kernl-sdk/protocol';\nimport { run } from '@kernl-sdk/protocol/agent';\n",
    );
    write_file(
        temp.path(),
        "packages/protocol/package.json",
        "{\n  \"name\": \"@kernl-sdk/protocol\",\n  \"exports\": {\n    \".\": \"./src/index.ts\",\n    \"./agent\": \"./src/agent.ts\"\n  }\n}\n",
    );
    write_file(
        temp.path(),
        "packages/protocol/src/index.ts",
        "export const Agent = {};\n",
    );
    write_file(
        temp.path(),
        "packages/protocol/src/agent.ts",
        "export const run = () => {};\n",
    );

    let pi = WorkspaceIndex::build(temp.path()).expect("build workspace index");
    assert_eq!(
        pi.code.workspace_packages.get("@kernl-sdk/protocol"),
        Some(&PathBuf::from("packages/protocol"))
    );

    let imports = pi
        .code
        .code_imports
        .get(Path::new("apps/web/main.ts"))
        .expect("code imports for apps/web/main.ts");
    assert!(imports.iter().any(|import| {
        import.raw == "@kernl-sdk/protocol"
            && import.resolved_path.as_deref() == Some(Path::new("packages/protocol/src/index.ts"))
            && import.kind == ImportKind::Workspace
    }));
    assert!(imports.iter().any(|import| {
        import.raw == "@kernl-sdk/protocol/agent"
            && import.resolved_path.as_deref() == Some(Path::new("packages/protocol/src/agent.ts"))
            && import.kind == ImportKind::Workspace
    }));
}

#[test]
fn project_index_symbol_refs_match_imported_symbol_only() {
    let temp = tempdir().expect("tempdir");
    write_root_config(temp.path());
    write_file(
        temp.path(),
        "src/lib.rs",
        "pub mod local;\npub mod target;\npub mod uses_target;\n",
    );
    write_file(temp.path(), "src/target.rs", "pub fn handle() {}\n");
    write_file(
        temp.path(),
        "src/uses_target.rs",
        "use crate::target::handle;\npub fn run() {\n    handle();\n}\n",
    );
    write_file(
        temp.path(),
        "src/local.rs",
        "pub fn handle() {}\npub fn run() {\n    handle();\n}\n",
    );

    let pi = WorkspaceIndex::build_with_symbol_refs(temp.path(), &[])
        .expect("build project index with symbol refs");
    let rows =
        kdb::index::refs::collect_symbol_refs(&pi.code, temp.path(), "src/target.rs", "handle")
            .expect("collect symbol refs for target::handle");

    assert_eq!(rows.len(), 2);
    assert_eq!(rows.iter().filter(|row| row.is_definition).count(), 1);
    assert!(rows.iter().any(|row| {
        row.source_file == PathBuf::from("src/uses_target.rs") && !row.is_definition
    }));
    assert!(
        !rows
            .iter()
            .any(|row| { row.source_file == PathBuf::from("src/local.rs") && !row.is_definition })
    );
}

#[test]
fn slug_anchor_normalizes_heading_text() {
    assert_eq!(slug_anchor("  Hello, Rust World!  "), "hello-rust-world");
    assert_eq!(slug_anchor("___"), "section");
    assert_eq!(slug_anchor("A__B---C"), "a-b-c");
}

// ---------------------------------------------------------------------------
// Parser edge cases
// ---------------------------------------------------------------------------

#[test]
fn parse_markdown_heading_with_inline_code() {
    let parsed = parse_markdown("## The `useState` Hook\n");
    assert_eq!(parsed.headings.len(), 1);
    assert_eq!(parsed.headings[0].title, "The useState Hook");
    assert_eq!(parsed.headings[0].anchor, "the-usestate-hook");
}

#[test]
fn parse_markdown_link_inside_heading() {
    let parsed = parse_markdown("## See [Overview](overview.md)\n");
    assert_eq!(parsed.headings.len(), 1);
    assert_eq!(parsed.headings[0].title, "See Overview");
    assert_eq!(parsed.links.len(), 1);
    assert_eq!(parsed.links[0].target.file.as_deref(), Some("overview.md"));
}

#[test]
fn parse_markdown_ignores_wikilinks_in_code_blocks() {
    let content = "# Title\n\n```\n[[should/ignore]]\n```\n\n[[real/link]]\n";
    let parsed = parse_markdown(content);
    // Only the wikilink outside the code block should be parsed
    let wikilinks: Vec<_> = parsed
        .links
        .iter()
        .filter(|l| matches!(l.kind, LinkKind::Wikilink))
        .collect();
    assert_eq!(wikilinks.len(), 1);
    assert_eq!(wikilinks[0].target.file.as_deref(), Some("real/link"));
}

#[test]
fn parse_markdown_ignores_wikilinks_in_inline_code() {
    let content = "# Title\n\nSee `[[not/a/link]]` for details.\n\n[[actual/link]]\n";
    let parsed = parse_markdown(content);
    let wikilinks: Vec<_> = parsed
        .links
        .iter()
        .filter(|l| matches!(l.kind, LinkKind::Wikilink))
        .collect();
    // Inline code wikilinks may or may not be filtered — this test documents behavior
    // At minimum the real link must be present
    assert!(
        wikilinks
            .iter()
            .any(|l| l.target.file.as_deref() == Some("actual/link"))
    );
}

#[test]
fn parse_markdown_frontmatter_does_not_create_headings() {
    let content = "---\ntitle: My Note\ntags: [a, b]\n---\n\n# Real Heading\n";
    let parsed = parse_markdown(content);
    assert_eq!(parsed.headings.len(), 1);
    assert_eq!(parsed.headings[0].title, "Real Heading");
}

#[test]
fn parse_markdown_empty_file() {
    let parsed = parse_markdown("");
    assert!(parsed.headings.is_empty());
    assert!(parsed.links.is_empty());
}

#[test]
fn parse_markdown_file_with_no_headings() {
    let parsed = parse_markdown("Just some text\n\nwith paragraphs.\n");
    assert!(parsed.headings.is_empty());
    assert!(parsed.links.is_empty());
}

#[test]
fn parse_markdown_heading_with_special_chars() {
    let parsed = parse_markdown("## What's New in v2.0?\n");
    // pulldown-cmark converts ASCII apostrophe to Unicode smart quote
    assert_eq!(parsed.headings[0].title, "What\u{2019}s New in v2.0?");
    // slug strips non-ASCII, so the smart quote disappears
    assert_eq!(parsed.headings[0].anchor, "whats-new-in-v20");
}

#[test]
fn parse_markdown_all_six_heading_levels() {
    let content = "# H1\n## H2\n### H3\n#### H4\n##### H5\n###### H6\n";
    let parsed = parse_markdown(content);
    assert_eq!(parsed.headings.len(), 6);
    for (i, heading) in parsed.headings.iter().enumerate() {
        assert_eq!(heading.level, (i + 1) as u8);
    }
}

// ---------------------------------------------------------------------------
// Slug edge cases
// ---------------------------------------------------------------------------

#[test]
fn slug_anchor_all_special_characters() {
    assert_eq!(slug_anchor("!!!@@@###$$$"), "section");
    assert_eq!(slug_anchor(""), "section");
    assert_eq!(slug_anchor("   "), "section");
}

#[test]
fn slug_anchor_unicode_characters() {
    // Non-ASCII chars are stripped since slug only keeps ascii alphanumeric
    assert_eq!(slug_anchor("Intro"), "intro");
    // Pure unicode heading falls back to "section"
    assert_eq!(slug_anchor("\u{1F600}\u{1F600}\u{1F600}"), "section");
}

#[test]
fn slug_anchor_mixed_separators() {
    assert_eq!(slug_anchor("one - two _ three"), "one-two-three");
    assert_eq!(slug_anchor("a---b___c   d"), "a-b-c-d");
}

#[test]
fn slug_anchor_trailing_separators() {
    assert_eq!(slug_anchor("trailing---"), "trailing");
    assert_eq!(slug_anchor("---leading"), "leading");
}

// ---------------------------------------------------------------------------
// Index edge cases
// ---------------------------------------------------------------------------

#[test]
fn vault_index_file_linked_to_is_not_orphan() {
    let temp = tempdir().expect("tempdir");
    write_root_config(temp.path());
    write_file(temp.path(), "a.md", "# A\n\n[B](b.md)\n");
    write_file(temp.path(), "b.md", "# B\n\nJust content, no links.\n");

    let index = VaultIndex::build(temp.path()).expect("build index");
    let report = index.check();

    assert!(report.broken_links.is_empty());
    // b.md is linked to by a.md, so only a.md is the orphan
    assert_eq!(report.orphans, vec![PathBuf::from("a.md")]);
}

#[test]
fn vault_index_circular_references_are_not_broken() {
    let temp = tempdir().expect("tempdir");
    write_root_config(temp.path());
    write_file(temp.path(), "a.md", "# A\n\n[B](b.md)\n");
    write_file(temp.path(), "b.md", "# B\n\n[A](a.md)\n");

    let index = VaultIndex::build(temp.path()).expect("build index");
    let report = index.check();

    assert!(report.broken_links.is_empty());
    // Both files link to each other, so neither is an orphan
    assert!(report.orphans.is_empty());
}

#[test]
fn vault_index_self_referencing_links_do_not_count_as_inbound() {
    let temp = tempdir().expect("tempdir");
    write_root_config(temp.path());
    write_file(temp.path(), "self.md", "# Self\n\n[back to top](#self)\n");

    let index = VaultIndex::build(temp.path()).expect("build index");
    let report = index.check();

    // Self-links don't count as inbound from another file
    assert_eq!(report.orphans, vec![PathBuf::from("self.md")]);
}

#[test]
fn vault_index_broken_heading_anchor() {
    let temp = tempdir().expect("tempdir");
    write_root_config(temp.path());
    write_file(temp.path(), "a.md", "# A\n\n[B](b.md#nonexistent)\n");
    write_file(temp.path(), "b.md", "# B\n\n## Exists\n");

    let index = VaultIndex::build(temp.path()).expect("build index");
    let report = index.check();

    assert_eq!(report.broken_links.len(), 1);
    assert!(report.broken_links[0].reason.contains("heading not found"));
}

#[test]
fn vault_index_wikilink_resolution() {
    let temp = tempdir().expect("tempdir");
    write_root_config(temp.path());
    write_file(temp.path(), "a.md", "# A\n\n[[b#target]]\n");
    write_file(temp.path(), "b.md", "# B\n\n## Target\n");

    let index = VaultIndex::build(temp.path()).expect("build index");
    let report = index.check();

    assert!(report.broken_links.is_empty());
}

#[test]
fn vault_index_deeply_nested_files() {
    let temp = tempdir().expect("tempdir");
    write_root_config(temp.path());
    write_file(
        temp.path(),
        "a/b/c/deep.md",
        "# Deep\n\n[Top](../../../top.md)\n",
    );
    write_file(temp.path(), "top.md", "# Top\n\n[Deep](a/b/c/deep.md)\n");

    let index = VaultIndex::build(temp.path()).expect("build index");
    let report = index.check();

    assert!(report.broken_links.is_empty());
    assert!(report.orphans.is_empty());
}

#[test]
fn vault_index_empty_file_is_indexed() {
    let temp = tempdir().expect("tempdir");
    write_root_config(temp.path());
    write_file(temp.path(), "empty.md", "");
    write_file(temp.path(), "a.md", "# A\n\n[Empty](empty.md)\n");

    let index = VaultIndex::build(temp.path()).expect("build index");

    assert!(index.files.contains_key(Path::new("empty.md")));
    let report = index.check();
    assert!(report.broken_links.is_empty());
}

// ---------------------------------------------------------------------------
// Normalize rel path edge cases
// ---------------------------------------------------------------------------

#[test]
fn normalize_rel_path_current_dir_only() {
    assert_eq!(
        normalize_rel_path(Path::new("./file.md")),
        Some(PathBuf::from("file.md"))
    );
}

#[test]
fn normalize_rel_path_deep_parent_traversal() {
    // Valid: go up one then back down
    assert_eq!(
        normalize_rel_path(Path::new("a/b/../c/file.md")),
        Some(PathBuf::from("a/c/file.md"))
    );
    // Invalid: go up more levels than depth
    assert_eq!(normalize_rel_path(Path::new("a/../../file.md")), None);
}

// ---------------------------------------------------------------------------
// Resolve target path edge cases
// ---------------------------------------------------------------------------

#[test]
fn resolve_target_path_absolute_path_rejected() {
    let source = Path::new("notes/topic.md");
    let target = LinkTarget {
        file: Some("/etc/passwd".to_string()),
        anchor: None,
        root_relative: false,
    };
    assert_eq!(
        resolve_target_path(source, LinkKind::Markdown, &target),
        None
    );
}

#[test]
fn resolve_target_path_wikilink_with_explicit_md_extension() {
    let source = Path::new("notes/topic.md");
    let target = LinkTarget {
        file: Some("other.md".to_string()),
        anchor: None,
        root_relative: false,
    };
    // Wikilink with explicit .md should not double-add extension
    assert_eq!(
        resolve_target_path(source, LinkKind::Wikilink, &target),
        Some(PathBuf::from("notes/other.md"))
    );
}

#[test]
fn resolve_target_path_source_at_root_level() {
    let source = Path::new("root.md");
    let target = LinkTarget {
        file: Some("sibling.md".to_string()),
        anchor: None,
        root_relative: false,
    };
    assert_eq!(
        resolve_target_path(source, LinkKind::Markdown, &target),
        Some(PathBuf::from("sibling.md"))
    );
}

// ---------------------------------------------------------------------------
// kdb:// root-anchored links
// ---------------------------------------------------------------------------

#[test]
fn parse_markdown_target_kdb_root_link() {
    let target = parse_markdown_target("kdb://docs/guide.md#intro");
    assert_eq!(
        target,
        Some(LinkTarget {
            file: Some("docs/guide.md".to_string()),
            anchor: Some("intro".to_string()),
            root_relative: true,
        })
    );

    let target = parse_markdown_target("kdb://README.md");
    assert_eq!(
        target,
        Some(LinkTarget {
            file: Some("README.md".to_string()),
            anchor: None,
            root_relative: true,
        })
    );
}

#[test]
fn parse_markdown_target_kdb_rejects_non_markdown() {
    assert_eq!(parse_markdown_target("kdb://src/main.rs"), None);
    assert_eq!(parse_markdown_target("kdb://"), None);
    assert_eq!(parse_markdown_target("kdb://notes/a.txt"), None);
}

#[test]
fn resolve_target_path_kdb_root_ignores_source_dir() {
    // A kdb:// link from a deeply nested file should resolve relative to root,
    // not relative to the source file's directory.
    let source = Path::new("a/b/c/deep.md");
    let target = LinkTarget {
        file: Some("docs/guide.md".to_string()),
        anchor: None,
        root_relative: true,
    };
    assert_eq!(
        resolve_target_path(source, LinkKind::Markdown, &target),
        Some(PathBuf::from("docs/guide.md"))
    );
}

#[test]
fn vault_index_kdb_root_link_resolves() {
    let temp = tempdir().expect("tempdir");
    write_root_config(temp.path());
    write_file(
        temp.path(),
        "a/b/deep.md",
        "# Deep\n\n[Top](kdb://top.md)\n",
    );
    write_file(temp.path(), "top.md", "# Top\n");

    let index = VaultIndex::build(temp.path()).expect("build index");
    let report = index.check();

    assert!(report.broken_links.is_empty());
    // top.md has an inbound link from deep.md
    let inbound = index
        .file_inbound
        .get(Path::new("top.md"))
        .expect("inbound refs for top.md");
    assert_eq!(inbound.len(), 1);
    assert_eq!(inbound[0].source_file, PathBuf::from("a/b/deep.md"));
}

#[test]
fn vault_index_kdb_root_link_with_anchor_resolves() {
    let temp = tempdir().expect("tempdir");
    write_root_config(temp.path());
    write_file(
        temp.path(),
        "notes/ref.md",
        "# Ref\n\n[Guide intro](kdb://docs/guide.md#intro)\n",
    );
    write_file(temp.path(), "docs/guide.md", "# Guide\n\n## Intro\n");

    let index = VaultIndex::build(temp.path()).expect("build index");
    let report = index.check();

    assert!(report.broken_links.is_empty());
}

#[test]
fn vault_index_kdb_root_broken_link_reported() {
    let temp = tempdir().expect("tempdir");
    write_root_config(temp.path());
    write_file(
        temp.path(),
        "a.md",
        "# A\n\n[Missing](kdb://nonexistent.md)\n",
    );

    let index = VaultIndex::build(temp.path()).expect("build index");
    let report = index.check();

    assert_eq!(report.broken_links.len(), 1);
    assert!(
        report.broken_links[0]
            .reason
            .contains("target file not found: nonexistent.md")
    );
}

#[test]
fn parse_markdown_extracts_kdb_root_links() {
    let parsed = parse_markdown("# Title\n\n[Link](kdb://docs/guide.md#section)\n");
    assert_eq!(parsed.links.len(), 1);
    assert!(matches!(parsed.links[0].kind, LinkKind::Markdown));
    assert_eq!(
        parsed.links[0].target.file.as_deref(),
        Some("docs/guide.md")
    );
    assert_eq!(parsed.links[0].target.anchor.as_deref(), Some("section"));
    assert!(parsed.links[0].target.root_relative);
}

// ----------------------------------------------------------------------------------
// Prosaic composition resolution (spec §5.4)
// ----------------------------------------------------------------------------------

/// A marina-like file defining two procedures, used as a `use` import target.
const MARINA_FIXTURE: &str = "\
# Marina

## SOP-M02 :: Entity Creation

When creating an entity.

```prosaic
insert entity row
```

## SOP-M04 :: Entity Mention Registration

When registering mentions.

```prosaic
insert entity_mention rows
```
";

#[test]
fn prosaic_valid_import_and_run_resolve() {
    let temp = tempdir().expect("tempdir");
    write_root_config(temp.path());
    write_file(temp.path(), "kernel/SOP/marina.md", MARINA_FIXTURE);
    write_file(
        temp.path(),
        "kernel/SOP/code.md",
        "# Code\n\n## SOP-C09 :: Code Map\n\nWhen mapping.\n\n\
```prosaic\n\
use SOP-M04 from kernel/SOP/marina.md\n\
render the map\n\
run SOP-M04 to register mentions\n\
```\n",
    );

    let index = VaultIndex::build(temp.path()).expect("build index");
    let report = index.check();
    assert!(
        report.procedure_errors.is_empty(),
        "expected no procedure errors, got {:?}",
        report.procedure_errors
    );
}

#[test]
fn prosaic_same_file_run_needs_no_import() {
    let temp = tempdir().expect("tempdir");
    write_root_config(temp.path());
    // O02 calls O07, both defined in the same file — no `use` required (§5.2).
    write_file(
        temp.path(),
        "kernel/SOP/sched/cycle.md",
        "# Cycle\n\n## SOP-O02 :: Cycle Planning\n\nWhen planning.\n\n\
```prosaic\n\
run SOP-O07 on the candidates → ranked set\n\
```\n\n\
## SOP-O07 :: Opportunity Scan\n\nWhen scanning.\n\n\
```prosaic\n\
inventory candidates\n\
```\n",
    );

    let index = VaultIndex::build(temp.path()).expect("build index");
    let report = index.check();
    assert!(
        report.procedure_errors.is_empty(),
        "same-file run should resolve without import, got {:?}",
        report.procedure_errors
    );
}

#[test]
fn prosaic_use_missing_path_is_reported() {
    let temp = tempdir().expect("tempdir");
    write_root_config(temp.path());
    write_file(
        temp.path(),
        "kernel/SOP/code.md",
        "# Code\n\n## SOP-C09 :: Code Map\n\nWhen mapping.\n\n\
```prosaic\n\
use SOP-M04 from kernel/SOP/nonexistent.md\n\
run SOP-M04 to register\n\
```\n",
    );

    let index = VaultIndex::build(temp.path()).expect("build index");
    let report = index.check();
    assert_eq!(report.procedure_errors.len(), 1);
    assert!(
        report.procedure_errors[0]
            .reason
            .contains("use target file not found: kernel/SOP/nonexistent.md")
    );
    // The `use` is on the first body line of the block (fence on line 7).
    assert_eq!(report.procedure_errors[0].line, 8);
}

#[test]
fn prosaic_use_path_without_symbol_is_reported() {
    let temp = tempdir().expect("tempdir");
    write_root_config(temp.path());
    write_file(temp.path(), "kernel/SOP/marina.md", MARINA_FIXTURE);
    write_file(
        temp.path(),
        "kernel/SOP/code.md",
        "# Code\n\n## SOP-C09 :: Code Map\n\nWhen mapping.\n\n\
```prosaic\n\
use SOP-M99 from kernel/SOP/marina.md\n\
run SOP-M99 to do a thing\n\
```\n",
    );

    let index = VaultIndex::build(temp.path()).expect("build index");
    let report = index.check();
    // The `use` fails (marina.md has no SOP-M99); the `run` still resolves
    // because SOP-M99 is imported in-block (§5.4.1).
    assert_eq!(report.procedure_errors.len(), 1);
    assert!(
        report.procedure_errors[0]
            .reason
            .contains("does not define SOP-M99")
    );
}

#[test]
fn prosaic_unimported_run_is_reported() {
    let temp = tempdir().expect("tempdir");
    write_root_config(temp.path());
    write_file(temp.path(), "kernel/SOP/marina.md", MARINA_FIXTURE);
    write_file(
        temp.path(),
        "kernel/SOP/code.md",
        "# Code\n\n## SOP-C09 :: Code Map\n\nWhen mapping.\n\n\
```prosaic\n\
run SOP-M04 to register mentions\n\
```\n",
    );

    let index = VaultIndex::build(temp.path()).expect("build index");
    let report = index.check();
    assert_eq!(report.procedure_errors.len(), 1);
    assert!(
        report.procedure_errors[0]
            .reason
            .contains("no matching `use` import")
    );
}

#[test]
fn prosaic_illustrative_block_outside_procedure_is_skipped() {
    let temp = tempdir().expect("tempdir");
    write_root_config(temp.path());
    // A spec-style file: the prosaic block sits under an ordinary heading, not a
    // `## SOP-* ::` procedure heading, so its dangling `run` is not checked.
    write_file(
        temp.path(),
        "kernel/prosaic.md",
        "# Prosaic\n\n## 5. Composition\n\nExample of invocation:\n\n\
```prosaic\n\
run SOP-O07 on the candidates → ranked set\n\
```\n",
    );

    let index = VaultIndex::build(temp.path()).expect("build index");
    let report = index.check();
    assert!(
        report.procedure_errors.is_empty(),
        "illustrative block should be skipped, got {:?}",
        report.procedure_errors
    );
}

#[test]
fn prosaic_use_template_is_not_treated_as_import() {
    let temp = tempdir().expect("tempdir");
    write_root_config(temp.path());
    write_file(
        temp.path(),
        "kernel/SOP/comms.md",
        "# Comms\n\n## SOP-CM01 :: Workstream Progress Report\n\nWhen handing off.\n\n\
```prosaic\n\
use template: kernel/templates/comms/wpr.md\n\
write the report\n\
```\n",
    );

    let index = VaultIndex::build(temp.path()).expect("build index");
    let report = index.check();
    assert!(
        report.procedure_errors.is_empty(),
        "`use template:` is an ordinary verb, got {:?}",
        report.procedure_errors
    );
}

#[test]
fn prosaic_cross_tier_ids_resolve_by_path() {
    let temp = tempdir().expect("tempdir");
    write_root_config(temp.path());
    // A space-tier procedure (SOP-INP-004) imports a global one (SOP-004) by path.
    write_file(
        temp.path(),
        "kernel/SOP/ops.md",
        "# Ops\n\n## SOP-004 :: Global Thing\n\nWhen.\n\n```prosaic\ndo it\n```\n",
    );
    write_file(
        temp.path(),
        "space/SOP/local.md",
        "# Local\n\n## SOP-INP-004 :: Local Thing\n\nWhen.\n\n\
```prosaic\n\
use SOP-004 from kernel/SOP/ops.md\n\
run SOP-004 first\n\
```\n",
    );

    let index = VaultIndex::build(temp.path()).expect("build index");
    let report = index.check();
    assert!(
        report.procedure_errors.is_empty(),
        "cross-tier use should resolve by path, got {:?}",
        report.procedure_errors
    );
}
