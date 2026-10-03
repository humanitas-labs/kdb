//! tree-sitter-md parsing of one document into headings, links, and embeds.
//!
//! Behavior mirrors v1: headings come from the block tree (ATX and setext), markdown
//! links from `inline_link` nodes in the inline trees, wikilinks from a regex scan of
//! the source that skips code spans and code blocks. Anchors are GitHub-style slugs
//! deduplicated with `-1`, `-2` suffixes.

use std::collections::HashMap;
use std::path::Path;
use std::sync::LazyLock;

use regex::{Captures, Regex};
use tree_sitter::Node;
use tree_sitter_md::{MarkdownParser, MarkdownTree};

use super::{Heading, Link, LinkKind, LinkTarget};

static WIKILINK_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\[\[([^\]\r\n]+)\]\]").expect("valid wikilink regex"));
static MARKDOWN_LINK_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\[([^\]\r\n]+)\]\(([^)\r\n]+)\)").expect("valid link regex"));
static INLINE_CODE_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"`([^`\r\n]*)`").expect("valid inline code regex"));

/// The parsed pieces of one markdown source.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Parsed {
    pub headings: Vec<Heading>,
    pub links: Vec<Link>,
}

/// Parse markdown source. External URLs (`http:`, `mailto:` …) are dropped.
pub fn parse(source: &str) -> Parsed {
    let mut parser = MarkdownParser::default();
    let Some(tree) = parser.parse(source.as_bytes(), None) else {
        return Parsed::default();
    };
    let line_starts = line_start_offsets(source);
    let mut headings = Vec::new();
    let mut links = Vec::new();
    let mut excluded: Vec<(usize, usize)> = Vec::new();

    walk(&tree, |node, inline| {
        let kind = node.kind();
        if !inline && matches!(kind, "atx_heading" | "setext_heading") {
            if let Some(level) = heading_level(node) {
                let start = node.start_position();
                headings.push(Heading {
                    title: heading_title(node, source),
                    anchor: String::new(),
                    level,
                    line: start.row + 1,
                    column: start.column + 1,
                });
            }
        } else if inline && kind == "inline_link" {
            if let Some(dest) = child_text(node, source, "link_destination") {
                let raw = normalize_destination(&dest);
                if let Some(target) = parse_markdown_target(&raw) {
                    let start = node.start_position();
                    links.push(Link {
                        kind: LinkKind::Markdown,
                        embed: false,
                        raw,
                        target,
                        line: start.row + 1,
                        column: start.column + 1,
                        span: node.start_byte()..node.end_byte(),
                    });
                }
            }
        } else if (!inline && matches!(kind, "fenced_code_block" | "indented_code_block"))
            || (inline && kind == "code_span")
        {
            excluded.push((node.start_byte(), node.end_byte()));
        }
    });

    for cap in WIKILINK_RE.captures_iter(source) {
        let whole = cap.get(0).expect("match");
        if excluded.iter().any(|(s, e)| whole.start() >= *s && whole.start() < *e) {
            continue;
        }
        let inner = cap.get(1).expect("group").as_str();
        let Some(target) = parse_wikilink_target(inner) else { continue };
        let embed = whole.start() > 0 && source.as_bytes()[whole.start() - 1] == b'!';
        let (line, column) = line_col(&line_starts, whole.start());
        let span_start = if embed { whole.start() - 1 } else { whole.start() };
        links.push(Link {
            kind: LinkKind::Wikilink,
            embed,
            raw: format!("[[{inner}]]"),
            target,
            line,
            column,
            span: span_start..whole.end(),
        });
    }
    // v1 order: markdown links in tree order, then wikilinks; `check` output depends on it.

    let mut counts: HashMap<String, usize> = HashMap::new();
    for h in &mut headings {
        let base = slug_anchor(&h.title);
        let n = counts.entry(base.clone()).or_insert(0);
        h.anchor = if *n == 0 { base } else { format!("{base}-{n}") };
        *n += 1;
    }
    Parsed { headings, links }
}

/// GitHub-style anchor slug for a heading title (no dedup; see [`parse`]).
pub fn slug_anchor(title: &str) -> String {
    let mut out = String::new();
    let mut last_dash = false;
    for ch in title.trim().to_ascii_lowercase().chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch);
            last_dash = false;
        } else if (ch.is_ascii_whitespace() || ch == '-' || ch == '_') && !out.is_empty() && !last_dash {
            out.push('-');
            last_dash = true;
        }
    }
    while out.ends_with('-') {
        out.pop();
    }
    if out.is_empty() { "section".to_string() } else { out }
}

/// Parse a standard markdown destination (`a/b.md#x`, `#x`, `kdb://a.md`). `None` for
/// external URLs, non-markdown paths, and empty targets.
pub fn parse_markdown_target(raw: &str) -> Option<LinkTarget> {
    let raw = raw.trim();
    if raw.is_empty() || is_external(raw) {
        return None;
    }
    if let Some(anchor) = raw.strip_prefix('#') {
        let anchor = anchor.trim();
        return (!anchor.is_empty()).then(|| LinkTarget { file: None, anchor: Some(anchor.to_string()), root_relative: false });
    }
    let (body, root_relative) = match raw.strip_prefix("kdb://") {
        Some(rest) => (rest.trim(), true),
        None => (raw, false),
    };
    let (file, anchor) = split_anchor(body);
    if file.is_empty() || !is_markdown_path(file) {
        return None;
    }
    Some(LinkTarget { file: Some(file.to_string()), anchor, root_relative })
}

/// Parse the inner text of `[[...]]` (alias after `|` dropped, `kdb://` honored, `.md` optional).
pub fn parse_wikilink_target(raw: &str) -> Option<LinkTarget> {
    let body = raw.split('|').next()?.trim();
    let (body, root_relative) = match body.strip_prefix("kdb://") {
        Some(rest) => (rest.trim(), true),
        None => (body, false),
    };
    if body.is_empty() {
        return None;
    }
    if let Some(anchor) = body.strip_prefix('#') {
        let anchor = anchor.trim();
        return (!anchor.is_empty()).then(|| LinkTarget { file: None, anchor: Some(anchor.to_string()), root_relative });
    }
    let (file, anchor) = split_anchor(body);
    let file = (!file.is_empty()).then(|| file.to_string());
    if file.is_none() && anchor.is_none() {
        return None;
    }
    Some(LinkTarget { file, anchor, root_relative })
}

fn split_anchor(body: &str) -> (&str, Option<String>) {
    match body.split_once('#') {
        Some((f, a)) => {
            let a = a.trim();
            (f.trim(), (!a.is_empty()).then(|| a.to_string()))
        }
        None => (body, None),
    }
}

fn is_markdown_path(file: &str) -> bool {
    Path::new(file).extension().and_then(|e| e.to_str()).is_some_and(|e| e.eq_ignore_ascii_case("md"))
}

fn is_external(raw: &str) -> bool {
    !raw.starts_with("kdb://")
        && (raw.contains("://") || raw.starts_with("mailto:") || raw.starts_with("tel:") || raw.starts_with("data:"))
}

fn heading_level(node: Node<'_>) -> Option<u8> {
    let mut cursor = node.walk();
    node.children(&mut cursor).find_map(|c| match c.kind() {
        "atx_h1_marker" | "setext_h1_underline" => Some(1),
        "atx_h2_marker" | "setext_h2_underline" => Some(2),
        "atx_h3_marker" => Some(3),
        "atx_h4_marker" => Some(4),
        "atx_h5_marker" => Some(5),
        "atx_h6_marker" => Some(6),
        _ => None,
    })
}

fn heading_title(node: Node<'_>, source: &str) -> String {
    let raw = node
        .child_by_field_name("heading_content")
        .and_then(|c| c.utf8_text(source.as_bytes()).ok())
        .unwrap_or_default();
    normalize_heading_text(raw).split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Replace `[t](x)` with `t`, `[[x#a|alias]]` with the alias (or target name), strip
/// backticks, and turn `'` into `’` (what pulldown-cmark did in the original tool).
fn normalize_heading_text(input: &str) -> String {
    let links = MARKDOWN_LINK_RE.replace_all(input, |c: &Captures<'_>| c[1].to_string());
    let wikis = WIKILINK_RE.replace_all(&links, |c: &Captures<'_>| {
        let inner = c[1].trim();
        if let Some((_, alias)) = inner.split_once('|')
            && !alias.trim().is_empty()
        {
            return alias.trim().to_string();
        }
        let target = inner.split('|').next().unwrap_or(inner).trim();
        match target.strip_prefix('#') {
            Some(a) => a.trim().to_string(),
            None => target.split('#').next().unwrap_or(target).trim().to_string(),
        }
    });
    INLINE_CODE_RE.replace_all(&wikis, |c: &Captures<'_>| c[1].to_string()).replace('\'', "\u{2019}")
}

fn child_text(node: Node<'_>, source: &str, kind: &str) -> Option<String> {
    let mut cursor = node.walk();
    node.children(&mut cursor)
        .find(|c| c.kind() == kind)
        .and_then(|c| c.utf8_text(source.as_bytes()).ok())
        .map(str::to_string)
}

fn normalize_destination(dest: &str) -> String {
    let t = dest.trim();
    match t.strip_prefix('<').and_then(|t| t.strip_suffix('>')) {
        Some(inner) => inner.trim().to_string(),
        None => t.to_string(),
    }
}

/// Depth-first visit of the block tree and every inline tree.
fn walk(tree: &MarkdownTree, mut visit: impl FnMut(Node<'_>, bool)) {
    let mut cursor = tree.walk();
    loop {
        visit(cursor.node(), cursor.is_inline());
        if cursor.goto_first_child() || cursor.goto_next_sibling() {
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

/// Byte offset of the start of every line.
pub fn line_start_offsets(source: &str) -> Vec<usize> {
    let mut starts = vec![0];
    starts.extend(source.bytes().enumerate().filter(|(_, b)| *b == b'\n').map(|(i, _)| i + 1));
    starts
}

/// 1-based (line, byte column) for a byte offset.
pub fn line_col(line_starts: &[usize], offset: usize) -> (usize, usize) {
    let idx = match line_starts.binary_search(&offset) {
        Ok(i) => i,
        Err(0) => 0,
        Err(i) => i - 1,
    };
    (idx + 1, offset - line_starts[idx] + 1)
}

/// Section bounds for `anchor` (already slugged) as 0-based line indexes: start line of
/// the heading and the start line of the next heading of equal or higher level, if any.
/// `None` anchor means the whole file.
pub fn section_line_bounds(headings: &[Heading], anchor: Option<&str>) -> Option<(usize, Option<usize>)> {
    if headings.is_empty() {
        return anchor.is_none().then_some((0, None));
    }
    let start = match anchor {
        Some(a) => headings.iter().position(|h| h.anchor == a)?,
        None => 0,
    };
    let first = &headings[start];
    let end = headings[start + 1..].iter().find(|h| h.level <= first.level).map(|h| h.line - 1);
    Some((first.line - 1, end))
}

/// Byte bounds of a section; see [`section_line_bounds`].
pub fn section_byte_bounds(source: &str, headings: &[Heading], anchor: Option<&str>) -> Option<(usize, usize)> {
    let (start_line, end_line) = section_line_bounds(headings, anchor)?;
    let starts = line_start_offsets(source);
    let start = starts.get(start_line).copied().unwrap_or(0);
    let end = end_line.and_then(|l| starts.get(l).copied()).unwrap_or(source.len());
    (end > start).then_some((start, end))
}
