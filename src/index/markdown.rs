use regex::{Captures, Regex};
use std::collections::HashMap;
use std::path::Path;
use std::sync::LazyLock;
use tree_sitter::Node;
use tree_sitter_md::{MarkdownParser, MarkdownTree};

use super::prosaic::procedure_id_from_heading;
use super::{Heading, Link, LinkKind, LinkTarget, ParsedDocument, ProsaicBlock};

// ---------------------------------------------
// projects/kdb/src/index/markdown.rs
//
// static WIKILINK_RE                        L45
// static MARKDOWN_LINK_RE                   L47
// static INLINE_CODE_RE                     L50
// pub fn parse_markdown()                   L54
// pub fn parse_markdown_target()            L83
// fn parse_kdb_target()                    L133
// pub fn parse_wikilink_target()           L170
// pub fn slug_anchor()                     L217
// pub fn section_line_bounds()             L248
// pub fn section_byte_bounds()             L280
// fn collect_headings()                    L294
// fn collect_markdown_links()              L319
// fn collect_wikilink_excluded_ranges()    L348
// fn collect_prosaic_blocks()              L368
// fn assign_enclosing_procedures()         L425
// fn collect_wikilinks()                   L440
// fn assign_heading_anchors()              L476
// fn heading_level()                       L491
// fn heading_level_from_underline()        L509
// fn heading_title()                       L519
// fn normalize_heading_text()              L529
// fn child_text_by_kind()                  L572
// fn normalize_destination()               L581
// fn walk_markdown_tree()                  L590
// fn is_external()                         L614
// fn line_start_offsets()                  L624
// fn line_col()                            L634
// fn range_contains_offset()               L644
// fn normalize_inline_whitespace()         L650
// ---------------------------------------------

static WIKILINK_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\[\[([^\]\r\n]+)\]\]").expect("valid wikilink regex"));
static MARKDOWN_LINK_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\[([^\]\r\n]+)\]\(([^)\r\n]+)\)").expect("valid markdown link regex")
});
static INLINE_CODE_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"`([^`\r\n]*)`").expect("valid inline code regex"));

/// Parse markdown into headings and internal links.
pub fn parse_markdown(content: &str) -> ParsedDocument {
    let mut parser = MarkdownParser::default();
    let Some(tree) = parser.parse(content.as_bytes(), None) else {
        return ParsedDocument {
            headings: Vec::new(),
            links: Vec::new(),
            prosaic_blocks: Vec::new(),
        };
    };

    let mut headings = collect_headings(&tree, content);
    let mut links = collect_markdown_links(&tree, content);
    let excluded_ranges = collect_wikilink_excluded_ranges(&tree);
    let line_starts = line_start_offsets(content);
    links.extend(collect_wikilinks(content, &line_starts, &excluded_ranges));

    assign_heading_anchors(&mut headings);

    let mut prosaic_blocks = collect_prosaic_blocks(&tree, content, &line_starts);
    assign_enclosing_procedures(&mut prosaic_blocks, &headings);

    ParsedDocument {
        headings,
        links,
        prosaic_blocks,
    }
}

/// Parse a standard markdown link destination into a typed target.
pub fn parse_markdown_target(raw: &str) -> Option<LinkTarget> {
    let raw = raw.trim();
    if raw.is_empty() || is_external(raw) {
        return None;
    }

    if let Some(anchor) = raw.strip_prefix('#') {
        let anchor = anchor.trim();
        if anchor.is_empty() {
            return None;
        }
        return Some(LinkTarget {
            file: None,
            anchor: Some(anchor.to_string()),
            root_relative: false,
        });
    }

    // kdb:// root-anchored links — strip prefix, resolve relative to vault root.
    if let Some(rest) = raw.strip_prefix("kdb://") {
        return parse_kdb_target(rest);
    }

    let (file, anchor) = match raw.split_once('#') {
        Some((file, anchor)) => (file.trim(), Some(anchor.trim())),
        None => (raw, None),
    };
    if file.is_empty() {
        return None;
    }

    let is_markdown_path = Path::new(file)
        .extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| ext.eq_ignore_ascii_case("md"));
    if !is_markdown_path {
        return None;
    }

    let anchor = anchor
        .filter(|value| !value.is_empty())
        .map(ToString::to_string);
    Some(LinkTarget {
        file: Some(file.to_string()),
        anchor,
        root_relative: false,
    })
}

/// Parse the path after `kdb://` into a root-relative [`LinkTarget`].
fn parse_kdb_target(rest: &str) -> Option<LinkTarget> {
    let rest = rest.trim();
    if rest.is_empty() {
        return None;
    }

    let (file, anchor) = match rest.split_once('#') {
        Some((file, anchor)) => (file.trim(), Some(anchor.trim())),
        None => (rest, None),
    };
    if file.is_empty() {
        return None;
    }

    let is_markdown_path = Path::new(file)
        .extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| ext.eq_ignore_ascii_case("md"));
    if !is_markdown_path {
        return None;
    }

    let anchor = anchor
        .filter(|value| !value.is_empty())
        .map(ToString::to_string);
    Some(LinkTarget {
        file: Some(file.to_string()),
        anchor,
        root_relative: true,
    })
}

/// Parse the inner content of a wikilink (`[[...]]`) into a typed target.
///
/// Recognizes the `kdb://` prefix for root-relative links, mirroring the
/// markdown-link parser.  Extension-less paths are fine here — resolution
/// appends `.md` for wikilinks automatically.
pub fn parse_wikilink_target(raw: &str) -> Option<LinkTarget> {
    let body = raw.split('|').next()?.trim();
    if body.is_empty() {
        return None;
    }

    // kdb:// root-anchored wikilinks — strip prefix, mark root-relative.
    let (body, root_relative) = match body.strip_prefix("kdb://") {
        Some(rest) => (rest.trim(), true),
        None => (body, false),
    };

    if let Some(anchor) = body.strip_prefix('#') {
        let anchor = anchor.trim();
        if anchor.is_empty() {
            return None;
        }
        return Some(LinkTarget {
            file: None,
            anchor: Some(anchor.to_string()),
            root_relative,
        });
    }

    let (file, anchor) = match body.split_once('#') {
        Some((file, anchor)) => (Some(file.trim()), Some(anchor.trim())),
        None => (Some(body), None),
    };

    let file = file
        .filter(|value| !value.is_empty())
        .map(ToString::to_string);
    let anchor = anchor
        .filter(|value| !value.is_empty())
        .map(ToString::to_string);
    if file.is_none() && anchor.is_none() {
        return None;
    }

    Some(LinkTarget {
        file,
        anchor,
        root_relative,
    })
}

/// Convert heading text into a normalized anchor slug.
pub fn slug_anchor(input: &str) -> String {
    let mut out = String::new();
    let mut last_dash = false;

    for ch in input.trim().to_ascii_lowercase().chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch);
            last_dash = false;
            continue;
        }

        if (ch.is_ascii_whitespace() || ch == '-' || ch == '_') && !out.is_empty() && !last_dash {
            out.push('-');
            last_dash = true;
        }
    }

    while out.ends_with('-') {
        out.pop();
    }

    if out.is_empty() {
        "section".to_string()
    } else {
        out
    }
}

/// Resolve section start/end lines for a heading anchor.
///
/// The end bound is the next heading with equal or higher level, if any.
pub fn section_line_bounds(
    parsed: &ParsedDocument,
    anchor: Option<&str>,
) -> Option<(usize, Option<usize>)> {
    if parsed.headings.is_empty() {
        return anchor.is_none().then_some((0, None));
    }

    let start_index = match anchor {
        Some(raw) => {
            let wanted = slug_anchor(raw.trim_start_matches('#'));
            parsed
                .headings
                .iter()
                .position(|heading| heading.anchor == wanted)?
        }
        None => 0,
    };

    let start_heading = &parsed.headings[start_index];
    let start_line = start_heading.line.saturating_sub(1);
    let end_line = parsed
        .headings
        .iter()
        .skip(start_index + 1)
        .find(|heading| heading.level <= start_heading.level)
        .map(|heading| heading.line.saturating_sub(1));

    Some((start_line, end_line))
}

/// Resolve section byte bounds for a heading anchor.
pub fn section_byte_bounds(
    content: &str,
    parsed: &ParsedDocument,
    anchor: Option<&str>,
) -> Option<(usize, usize)> {
    let (start_line, end_line) = section_line_bounds(parsed, anchor)?;
    let line_starts = line_start_offsets(content);
    let start = line_starts.get(start_line).copied().unwrap_or(0);
    let end = end_line
        .and_then(|line| line_starts.get(line).copied())
        .unwrap_or(content.len());
    (end > start).then_some((start, end))
}

fn collect_headings(tree: &MarkdownTree, content: &str) -> Vec<Heading> {
    let mut headings = Vec::new();

    walk_markdown_tree(tree, |node, is_inline| {
        if is_inline || !matches!(node.kind(), "atx_heading" | "setext_heading") {
            return;
        }

        let Some(level) = heading_level(node) else {
            return;
        };
        let title = heading_title(node, content);
        let start = node.start_position();
        headings.push(Heading {
            title,
            anchor: String::new(),
            level,
            line: start.row as usize + 1,
            column: start.column as usize + 1,
        });
    });

    headings
}

fn collect_markdown_links(tree: &MarkdownTree, content: &str) -> Vec<Link> {
    let mut links = Vec::new();

    walk_markdown_tree(tree, |node, is_inline| {
        if !is_inline || node.kind() != "inline_link" {
            return;
        }

        let Some(destination) = child_text_by_kind(node, content, "link_destination") else {
            return;
        };
        let normalized = normalize_destination(destination);
        let Some(target) = parse_markdown_target(&normalized) else {
            return;
        };

        let start = node.start_position();
        links.push(Link {
            kind: LinkKind::Markdown,
            raw: normalized,
            target,
            line: start.row as usize + 1,
            column: start.column as usize + 1,
        });
    });

    links
}

fn collect_wikilink_excluded_ranges(tree: &MarkdownTree) -> Vec<(usize, usize)> {
    let mut ranges = Vec::new();

    walk_markdown_tree(tree, |node, is_inline| {
        let is_block_code =
            !is_inline && matches!(node.kind(), "fenced_code_block" | "indented_code_block");
        let is_inline_code = is_inline && node.kind() == "code_span";
        if is_block_code || is_inline_code {
            ranges.push((node.start_byte(), node.end_byte()));
        }
    });

    ranges
}

/// Collect every `prosaic`-fenced code block with its body and 1-based start line.
///
/// Uses the parsed fence delimiters so nested/illustrative fences (e.g. a
/// ```` ``` ````-in-```` ```` ```` example) are handled by the parser rather than
/// a naive line scan.
fn collect_prosaic_blocks(
    tree: &MarkdownTree,
    content: &str,
    line_starts: &[usize],
) -> Vec<ProsaicBlock> {
    let mut blocks = Vec::new();

    walk_markdown_tree(tree, |node, is_inline| {
        if is_inline || node.kind() != "fenced_code_block" {
            return;
        }

        let mut cursor = node.walk();
        let delims: Vec<Node<'_>> = node
            .children(&mut cursor)
            .filter(|child| child.kind() == "fenced_code_block_delimiter")
            .collect();
        if delims.len() < 2 {
            return; // unterminated fence
        }
        let open_row = delims[0].start_position().row as usize;
        let close_row = delims[delims.len() - 1].start_position().row as usize;

        // Info string is whatever follows the opening delimiter on its line.
        let info_start = delims[0].end_byte();
        let info_end = line_starts
            .get(open_row + 1)
            .map(|next| next.saturating_sub(1))
            .unwrap_or(content.len());
        let info = content.get(info_start..info_end).unwrap_or("").trim();
        let lang = info.split_whitespace().next().unwrap_or("");
        if !lang.eq_ignore_ascii_case("prosaic") {
            return;
        }

        let body_first_row = open_row + 1;
        let start_line = body_first_row + 1; // 1-based
        let body = if body_first_row >= close_row {
            String::new()
        } else {
            let start_byte = line_starts.get(body_first_row).copied().unwrap_or(content.len());
            let end_byte = line_starts.get(close_row).copied().unwrap_or(content.len());
            content.get(start_byte..end_byte).unwrap_or("").to_string()
        };

        blocks.push(ProsaicBlock {
            start_line,
            body,
            enclosing_procedure: None,
        });
    });

    blocks
}

/// For each block, record the ID of the nearest enclosing procedure heading
/// (level ≤ 2 heading that matches `SOP-<ID> :: …`), if any.
fn assign_enclosing_procedures(blocks: &mut [ProsaicBlock], headings: &[Heading]) {
    for block in blocks {
        let mut enclosing: Option<&str> = None;
        for heading in headings {
            if heading.line >= block.start_line {
                break;
            }
            if heading.level <= 2 {
                enclosing = Some(&heading.title);
            }
        }
        block.enclosing_procedure = enclosing.and_then(procedure_id_from_heading);
    }
}

fn collect_wikilinks(
    content: &str,
    line_starts: &[usize],
    excluded: &[(usize, usize)],
) -> Vec<Link> {
    let mut links = Vec::new();

    for captures in WIKILINK_RE.captures_iter(content) {
        let Some(full_match) = captures.get(0) else {
            continue;
        };
        if range_contains_offset(excluded, full_match.start()) {
            continue;
        }

        let Some(inner_match) = captures.get(1) else {
            continue;
        };
        let raw_inner = inner_match.as_str();
        let Some(target) = parse_wikilink_target(raw_inner) else {
            continue;
        };

        let (line, column) = line_col(line_starts, full_match.start());
        links.push(Link {
            kind: LinkKind::Wikilink,
            raw: format!("[[{raw_inner}]]"),
            target,
            line,
            column,
        });
    }

    links
}

fn assign_heading_anchors(headings: &mut [Heading]) {
    let mut anchor_counts: HashMap<String, usize> = HashMap::new();

    for heading in headings {
        let base = slug_anchor(&heading.title);
        let count = anchor_counts.entry(base.clone()).or_insert(0);
        heading.anchor = if *count == 0 {
            base
        } else {
            format!("{}-{}", base, *count)
        };
        *count += 1;
    }
}

fn heading_level(node: Node<'_>) -> Option<u8> {
    if node.kind() == "setext_heading" {
        return heading_level_from_underline(node);
    }

    let mut cursor = node.walk();
    node.children(&mut cursor)
        .find_map(|child| match child.kind() {
            "atx_h1_marker" => Some(1),
            "atx_h2_marker" => Some(2),
            "atx_h3_marker" => Some(3),
            "atx_h4_marker" => Some(4),
            "atx_h5_marker" => Some(5),
            "atx_h6_marker" => Some(6),
            _ => None,
        })
}

fn heading_level_from_underline(node: Node<'_>) -> Option<u8> {
    let mut cursor = node.walk();
    node.children(&mut cursor)
        .find_map(|child| match child.kind() {
            "setext_h1_underline" => Some(1),
            "setext_h2_underline" => Some(2),
            _ => None,
        })
}

fn heading_title(node: Node<'_>, content: &str) -> String {
    let source = content.as_bytes();
    let raw = node
        .child_by_field_name("heading_content")
        .and_then(|child| child.utf8_text(source).ok())
        .unwrap_or_default();

    normalize_inline_whitespace(normalize_heading_text(raw))
}

fn normalize_heading_text(input: &str) -> String {
    let with_links = MARKDOWN_LINK_RE.replace_all(input, |captures: &Captures<'_>| {
        captures
            .get(1)
            .map_or_else(String::new, |value| value.as_str().to_string())
    });

    let with_wikilinks = WIKILINK_RE.replace_all(&with_links, |captures: &Captures<'_>| {
        let Some(inner) = captures.get(1).map(|value| value.as_str().trim()) else {
            return String::new();
        };

        if let Some((_, alias)) = inner.split_once('|') {
            let alias = alias.trim();
            if !alias.is_empty() {
                return alias.to_string();
            }
        }

        let target = inner.split('|').next().unwrap_or(inner).trim();
        if let Some(anchor) = target.strip_prefix('#') {
            return anchor.trim().to_string();
        }

        target
            .split('#')
            .next()
            .unwrap_or(target)
            .trim()
            .to_string()
    });

    let normalized = INLINE_CODE_RE
        .replace_all(&with_wikilinks, |captures: &Captures<'_>| {
            captures
                .get(1)
                .map_or_else(String::new, |value| value.as_str().to_string())
        })
        .into_owned();

    normalized.replace('\'', "\u{2019}")
}

fn child_text_by_kind<'a>(node: Node<'a>, content: &str, wanted: &str) -> Option<String> {
    let mut cursor = node.walk();
    let source = content.as_bytes();
    node.children(&mut cursor)
        .find(|child| child.kind() == wanted)
        .and_then(|child| child.utf8_text(source).ok())
        .map(|value| value.to_string())
}

fn normalize_destination(input: String) -> String {
    let trimmed = input.trim();
    if trimmed.starts_with('<') && trimmed.ends_with('>') && trimmed.len() >= 2 {
        trimmed[1..trimmed.len() - 1].trim().to_string()
    } else {
        trimmed.to_string()
    }
}

fn walk_markdown_tree(tree: &MarkdownTree, mut visit: impl FnMut(Node<'_>, bool)) {
    let mut cursor = tree.walk();

    loop {
        visit(cursor.node(), cursor.is_inline());

        if cursor.goto_first_child() {
            continue;
        }
        if cursor.goto_next_sibling() {
            continue;
        }

        loop {
            if !cursor.goto_parent() {
                return;
            }
            if cursor.goto_next_sibling() {
                break;
            }
        }
    }
}

fn is_external(raw: &str) -> bool {
    if raw.starts_with("kdb://") {
        return false;
    }
    raw.contains("://")
        || raw.starts_with("mailto:")
        || raw.starts_with("tel:")
        || raw.starts_with("data:")
}

fn line_start_offsets(content: &str) -> Vec<usize> {
    let mut starts = vec![0];
    for (index, byte) in content.bytes().enumerate() {
        if byte == b'\n' {
            starts.push(index + 1);
        }
    }
    starts
}

fn line_col(line_starts: &[usize], byte_index: usize) -> (usize, usize) {
    let line_idx = match line_starts.binary_search(&byte_index) {
        Ok(index) => index,
        Err(0) => 0,
        Err(index) => index - 1,
    };
    let line_start = line_starts[line_idx];
    (line_idx + 1, byte_index.saturating_sub(line_start) + 1)
}

fn range_contains_offset(ranges: &[(usize, usize)], offset: usize) -> bool {
    ranges
        .iter()
        .any(|(start, end)| offset >= *start && offset < *end)
}

fn normalize_inline_whitespace(input: String) -> String {
    input.split_whitespace().collect::<Vec<_>>().join(" ")
}
