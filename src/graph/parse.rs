//! tree-sitter-md parsing of one document into headings, links, and embeds.

use super::{Heading, Link};

/// The parsed pieces of one markdown source.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Parsed {
    pub headings: Vec<Heading>,
    pub links: Vec<Link>,
}

/// Parse markdown source. External URLs (`http:`, `mailto:` …) are dropped.
pub fn parse(source: &str) -> Parsed {
    todo!("graph agent")
}

/// GitHub-style anchor slug for a heading title (no dedup; see [`parse`]).
pub fn slug_anchor(title: &str) -> String {
    todo!("graph agent")
}
