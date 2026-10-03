//! Text output for `check`, byte-for-byte the v1 format.

use std::fmt::Write as _;
use std::path::Path;

use super::{CheckReport, ProblemKind};

/// Keep only problems and orphans whose source file is `scope` (a file) or under it (a directory).
pub fn scope(report: &mut CheckReport, scope: &Path, is_dir: bool) {
    if scope.as_os_str().is_empty() {
        return;
    }
    let keep = |p: &Path| if is_dir { p.starts_with(scope) } else { p == scope };
    report.problems.retain(|p| keep(&p.file));
    report.orphans.retain(|o| keep(o));
}

/// Render the report: broken links, then broken embeds, then orphans (listed or counted),
/// then the `N errors` / `N warnings` summary, or `kdb check: no issues found`.
pub fn render(report: &CheckReport, list_orphans: bool) -> String {
    let mut out = String::new();
    for kind in [ProblemKind::BrokenLink, ProblemKind::BrokenEmbed] {
        let label = match kind {
            ProblemKind::BrokenLink => "broken link",
            ProblemKind::BrokenEmbed => "broken embed",
        };
        for p in report.problems.iter().filter(|p| p.kind == kind) {
            let _ = writeln!(out, "{}:{}:{} {label} {} ({})", p.file.display(), p.line, p.column, p.raw, p.reason);
        }
    }
    let orphans = report.orphans.len();
    if list_orphans {
        for o in &report.orphans {
            let _ = writeln!(out, "{} orphan file (0 inbound links)", o.display());
        }
    } else if orphans > 0 {
        let _ = writeln!(out, "{orphans} {} (run `kdb check --orphans` to list)", plural(orphans, "orphan file", "orphan files"));
    }
    let errors = report.problems.len();
    if errors == 0 && orphans == 0 {
        out.push_str("kdb check: no issues found\n");
        return out;
    }
    if errors > 0 {
        let _ = writeln!(out, "{errors} {}", plural(errors, "error", "errors"));
    }
    if orphans > 0 {
        let _ = writeln!(out, "{orphans} {}", plural(orphans, "warning", "warnings"));
    }
    out
}

fn plural<'a>(n: usize, one: &'a str, many: &'a str) -> &'a str {
    if n == 1 { one } else { many }
}
