//! Unified-diff parsing for `git diff` / `git show` output, and building the
//! partial patches used for hunk- and line-level staging.

use std::collections::HashSet;

use crate::model::*;

pub fn parse_patch(path: &str, patch: &str) -> FileDiff {
    let mut diff = FileDiff { path: path.to_string(), ..Default::default() };
    let mut old_no = 0u32;
    let mut new_no = 0u32;

    // `split('\n')` rather than `lines()` so CRLF files keep their `\r`.
    let mut lines: Vec<&str> = patch.split('\n').collect();
    if lines.last() == Some(&"") {
        lines.pop();
    }
    for line in lines {
        if line.starts_with("Binary files ") || line == "GIT binary patch" {
            diff.binary = true;
            continue;
        }
        if let Some(header) = line.strip_prefix("@@ ") {
            let (old, new) = parse_hunk_header(header);
            old_no = old;
            new_no = new;
            diff.hunks.push(Hunk { header: line.to_string(), old_start: old, new_start: new, lines: Vec::new() });
            continue;
        }
        let Some(hunk) = diff.hunks.last_mut() else {
            diff.header.push_str(line);
            diff.header.push('\n');
            continue;
        };
        let (kind, content) = match line.as_bytes().first() {
            Some(b'+') => (DiffLineKind::Add, &line[1..]),
            Some(b'-') => (DiffLineKind::Del, &line[1..]),
            Some(b' ') => (DiffLineKind::Context, &line[1..]),
            Some(b'\\') => {
                if let Some(last) = hunk.lines.last_mut() {
                    last.no_eol = true;
                }
                continue;
            }
            None => (DiffLineKind::Context, ""),
            _ => continue,
        };
        let (o, n) = match kind {
            DiffLineKind::Add => {
                diff.additions += 1;
                new_no += 1;
                (None, Some(new_no - 1))
            }
            DiffLineKind::Del => {
                diff.deletions += 1;
                old_no += 1;
                (Some(old_no - 1), None)
            }
            DiffLineKind::Context => {
                old_no += 1;
                new_no += 1;
                (Some(old_no - 1), Some(new_no - 1))
            }
        };
        hunk.lines.push(DiffLine { kind, content: content.to_string(), old_no: o, new_no: n, no_eol: false });
    }
    diff
}

/// `-12,5 +14,7 @@ fn x()` -> (12, 14)
fn parse_hunk_header(header: &str) -> (u32, u32) {
    let mut parts = header.split(' ');
    let start = |s: Option<&str>| s.and_then(|s| s[1..].split(',').next()?.parse().ok()).unwrap_or(1);
    let old = start(parts.next());
    let new = start(parts.next());
    (old, new)
}

/// How a partial patch will be applied.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApplyMode {
    /// Apply an unstaged diff forward onto the index.
    Stage,
    /// Apply a staged diff in reverse onto the index.
    Unstage,
    /// Apply an unstaged diff in reverse onto the working tree.
    Discard,
}

impl ApplyMode {
    pub fn reverse(self) -> bool {
        !matches!(self, ApplyMode::Stage)
    }
}

/// Build a patch containing only the chosen lines of the given hunks.
///
/// `selected` holds `(hunk, line)` indices; `None` takes the hunks whole.
/// Unselected changes are neutralised so the patch still applies: going
/// forward an unselected `+` is dropped and an unselected `-` becomes context;
/// in reverse it's the other way round. Line counts are left to `git apply --recount`.
pub fn partial_patch(
    diff: &FileDiff,
    hunks: &[usize],
    selected: Option<&HashSet<(usize, usize)>>,
    mode: ApplyMode,
) -> Option<String> {
    let mut out = if diff.header.is_empty() {
        format!("diff --git a/{0} b/{0}\n--- a/{0}\n+++ b/{0}\n", diff.path)
    } else {
        diff.header.clone()
    };
    let mut any_change = false;
    for &h in hunks {
        let Some(hunk) = diff.hunks.get(h) else { continue };
        let mut body = String::new();
        let mut hunk_change = false;
        for (i, line) in hunk.lines.iter().enumerate() {
            let chosen = selected.is_none_or(|s| s.contains(&(h, i)));
            let prefix = match (line.kind, chosen, mode.reverse()) {
                (DiffLineKind::Context, ..) => ' ',
                (DiffLineKind::Add, true, _) => '+',
                (DiffLineKind::Del, true, _) => '-',
                (DiffLineKind::Add, false, false) | (DiffLineKind::Del, false, true) => continue,
                (DiffLineKind::Add, false, true) | (DiffLineKind::Del, false, false) => ' ',
            };
            hunk_change |= prefix != ' ';
            body.push(prefix);
            body.push_str(&line.content);
            body.push('\n');
            if line.no_eol {
                body.push_str("\\ No newline at end of file\n");
            }
        }
        if hunk_change {
            any_change = true;
            out.push_str(&format!("@@ -{},1 +{},1 @@\n{body}", hunk.old_start, hunk.new_start));
        }
    }
    any_change.then_some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    const PATCH: &str = "diff --git a/f b/f\n--- a/f\n+++ b/f\n@@ -1,3 +1,3 @@\n a\n-b\n+B\n c\n";

    #[test]
    fn parses_hunks_and_line_numbers() {
        let d = parse_patch("f", PATCH);
        assert_eq!(d.hunks.len(), 1);
        assert_eq!(d.header, "diff --git a/f b/f\n--- a/f\n+++ b/f\n");
        assert_eq!((d.additions, d.deletions), (1, 1));
        let lines = &d.hunks[0].lines;
        assert_eq!(lines[1].old_no, Some(2));
        assert_eq!(lines[2].new_no, Some(2));
        assert_eq!(lines[3].old_no, Some(3));
    }

    #[test]
    fn keeps_crlf_and_no_eol_marker() {
        let d = parse_patch("f", "@@ -1 +1 @@\n-x\r\n+y\n\\ No newline at end of file\n");
        assert_eq!(d.hunks[0].lines[0].content, "x\r");
        assert!(d.hunks[0].lines[1].no_eol);
    }

    #[test]
    fn stage_only_the_addition() {
        let d = parse_patch("f", PATCH);
        let sel: HashSet<(usize, usize)> = [(0, 2)].into();
        let p = partial_patch(&d, &[0], Some(&sel), ApplyMode::Stage).unwrap();
        assert!(p.ends_with(" a\n b\n+B\n c\n"), "{p}");
    }

    #[test]
    fn unstage_only_the_deletion() {
        let d = parse_patch("f", PATCH);
        let sel: HashSet<(usize, usize)> = [(0, 1)].into();
        let p = partial_patch(&d, &[0], Some(&sel), ApplyMode::Unstage).unwrap();
        assert!(p.ends_with(" a\n-b\n B\n c\n"), "{p}");
    }

    #[test]
    fn context_only_selection_is_none() {
        let d = parse_patch("f", PATCH);
        let sel: HashSet<(usize, usize)> = [(0, 0), (0, 3)].into();
        assert!(partial_patch(&d, &[0], Some(&sel), ApplyMode::Stage).is_none());
    }
}
