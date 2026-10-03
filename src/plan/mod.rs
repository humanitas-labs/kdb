//! The planning layer: projects, spaces, tasks, cycles, labels, statuses, and
//! task dependencies, on the SQLite schema in `src/db/migrations/`.
//!
//! This file holds what the submodules share: external task ids, fractional
//! order keys, terminal color, and the aligned text table every `list` prints.

pub mod cli;
pub mod cycles;
pub mod deps;
pub mod labels;
pub mod projects;
pub mod spaces;
pub mod statuses;
pub mod tasks;

use std::io::IsTerminal;

use anyhow::{Context, Result, bail};

/// Zero-padded width of the top-level sequence in an external id (`KDB-0012`).
pub const SEQ_WIDTH: usize = 4;
const ORDER_KEY_WIDTH: usize = 12;
const ALPHABET: &[u8; 36] = b"0123456789abcdefghijklmnopqrstuvwxyz";

/// The order key a task gets when inserted at the end: its seq, zero-padded.
pub fn default_order_key(seq: i64) -> String {
    format!("{seq:0width$}", width = ORDER_KEY_WIDTH)
}

/// A parsed external id: `ALIAS-seq` or `ALIAS-seq.child.child`.
#[derive(Debug, Clone)]
pub struct TaskId {
    pub alias: String,
    pub seq: i64,
    pub child_path: Vec<i64>,
}

impl TaskId {
    pub fn parse(s: &str) -> Result<Self> {
        let idx = s.rfind('-').with_context(|| format!("invalid task id '{s}': expected ALIAS-seq"))?;
        let (prefix, rest) = s.split_at(idx);
        if prefix.is_empty() {
            bail!("invalid task id '{s}': empty alias");
        }
        let mut parts = rest[1..].split('.');
        let seq: i64 = parts
            .next()
            .with_context(|| format!("invalid task id '{s}': missing seq"))?
            .parse()
            .with_context(|| format!("invalid task id '{s}': seq not an integer"))?;
        let mut child_path = Vec::new();
        for p in parts {
            if p.is_empty() {
                bail!("invalid task id '{s}': empty child component");
            }
            let n: i64 = p.parse().with_context(|| format!("invalid task id '{s}': child '{p}' not an integer"))?;
            if n < 1 {
                bail!("invalid task id '{s}': child component must be >= 1");
            }
            child_path.push(n);
        }
        Ok(Self { alias: prefix.to_ascii_uppercase(), seq, child_path })
    }

    pub fn render(&self) -> String {
        let mut s = format!("{}-{:0width$}", self.alias, self.seq, width = SEQ_WIDTH);
        for n in &self.child_path {
            s.push('.');
            s.push_str(&n.to_string());
        }
        s
    }
}

// --- fractional order keys over the alphabet 0-9a-z ------------------------

fn rank(b: u8) -> usize {
    match b {
        b'0'..=b'9' => (b - b'0') as usize,
        b'a'..=b'z' => 10 + (b - b'a') as usize,
        _ => unreachable!("order keys use alphabet 0-9a-z (got byte {b:#x})"),
    }
}

fn utf8(v: Vec<u8>) -> String {
    String::from_utf8(v).expect("order keys are ascii")
}

/// A key strictly between `prev` and `next` (`None` = open end).
pub fn between(prev: Option<&str>, next: Option<&str>) -> String {
    match (prev, next) {
        (None, None) => String::from(ALPHABET[ALPHABET.len() / 2] as char),
        (None, Some(n)) => key_before(n),
        (Some(p), None) => key_after(p),
        (Some(p), Some(n)) => key_between_two(p, n),
    }
}

fn key_before(next: &str) -> String {
    let mut out = Vec::new();
    for &b in next.as_bytes() {
        let r = rank(b);
        if r > 0 {
            out.push(ALPHABET[r / 2]);
            return utf8(out);
        }
        out.push(b);
    }
    panic!("cannot produce key before all-min key: {next:?}");
}

fn key_after(prev: &str) -> String {
    let mut out = Vec::new();
    for &b in prev.as_bytes() {
        let r = rank(b);
        if r + 1 < ALPHABET.len() {
            out.push(ALPHABET[(r + ALPHABET.len()) / 2]);
            return utf8(out);
        }
        out.push(b);
    }
    out.push(ALPHABET[ALPHABET.len() / 2]);
    utf8(out)
}

fn key_between_two(prev: &str, next: &str) -> String {
    let (a, b) = (prev.as_bytes(), next.as_bytes());
    let mut out = Vec::new();
    let mut i = 0;
    while i < a.len() && i < b.len() && a[i] == b[i] {
        out.push(a[i]);
        i += 1;
    }
    match (a.get(i).copied().map(rank), b.get(i).copied().map(rank)) {
        (Some(ar), Some(br)) if br - ar >= 2 => out.push(ALPHABET[(ar + br) / 2]),
        (Some(ar), Some(_)) => {
            out.push(ALPHABET[ar]);
            out.extend_from_slice(key_after(std::str::from_utf8(&a[i + 1..]).unwrap()).as_bytes());
        }
        (None, Some(br)) if br > 0 => out.push(ALPHABET[br / 2]),
        (None, Some(_)) => {
            out.push(ALPHABET[0]);
            out.extend_from_slice(key_before(std::str::from_utf8(&b[i + 1..]).unwrap()).as_bytes());
        }
        _ => unreachable!("key_between_two requires prev < next ({prev:?} vs {next:?})"),
    }
    utf8(out)
}

// --- terminal color ---------------------------------------------------------

pub fn parse_hex(s: &str) -> Option<(u8, u8, u8)> {
    let s = s.strip_prefix('#').unwrap_or(s);
    if s.len() != 6 || !s.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    let n = u32::from_str_radix(s, 16).ok()?;
    Some(((n >> 16) as u8, (n >> 8) as u8, n as u8))
}

/// Wrap `text` in a truecolor escape when stdout is a terminal and `hex` parses.
pub fn colorize(text: &str, hex: Option<&str>) -> String {
    match hex.and_then(parse_hex) {
        Some((r, g, b)) if std::io::stdout().is_terminal() => format!("\x1b[38;2;{r};{g};{b}m{text}\x1b[0m"),
        _ => text.to_string(),
    }
}

// --- text tables ------------------------------------------------------------

/// Aligned table: each column is padded to the widest of header and cells, the
/// last column is left ragged. `right` lists right-aligned columns; `colors[i]`
/// tints row i's first cell. Widths are byte lengths, as v1 computed them.
pub fn table(header: &[&str], rows: &[Vec<String>], colors: &[Option<String>], right: &[usize]) -> String {
    let widths: Vec<usize> = (0..header.len())
        .map(|i| rows.iter().map(|r| r[i].len()).chain([header[i].len()]).max().unwrap_or(0))
        .collect();
    let mut out = String::new();
    let mut emit = |cells: &[String], color: Option<&str>| {
        for (i, cell) in cells.iter().enumerate() {
            if i > 0 {
                out.push_str("  ");
            }
            if i + 1 == cells.len() {
                out.push_str(cell);
                break;
            }
            let pad = " ".repeat(widths[i].saturating_sub(cell.chars().count()));
            let text = if i == 0 { colorize(cell, color) } else { cell.clone() };
            if right.contains(&i) {
                out.push_str(&pad);
                out.push_str(&text);
            } else {
                out.push_str(&text);
                out.push_str(&pad);
            }
        }
        out.push('\n');
    };
    let head: Vec<String> = header.iter().map(|h| h.to_string()).collect();
    emit(&head, None);
    for (i, row) in rows.iter().enumerate() {
        emit(row, colors.get(i).and_then(|c| c.as_deref()));
    }
    out
}

/// `key:<pad>value` lines for `show` output; `width` is the label column width.
pub fn kv(out: &mut String, width: usize, key: &str, value: impl std::fmt::Display) {
    out.push_str(&format!("{:<width$}{value}\n", format!("{key}:")));
}

/// `"-"` for `None`.
pub fn dash(v: Option<&str>) -> &str {
    v.unwrap_or("-")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn task_id_round_trip() {
        let id = TaskId::parse("kdb-12.3").unwrap();
        assert_eq!((id.alias.as_str(), id.seq, id.child_path.as_slice()), ("KDB", 12, &[3][..]));
        assert_eq!(id.render(), "KDB-0012.3");
        assert!(TaskId::parse("KDB").is_err());
        assert!(TaskId::parse("KDB-1.0").is_err());
    }

    #[test]
    fn order_keys_sort() {
        assert!(between(None, Some("5")).as_str() < "5");
        assert!(between(Some("5"), None).as_str() > "5");
        let mid = between(Some("5"), Some("6"));
        assert!("5" < mid.as_str() && mid.as_str() < "6");
        let mid = between(Some("000000000001"), Some("000000000002"));
        assert!("000000000001" < mid.as_str() && mid.as_str() < "000000000002");
    }
}
