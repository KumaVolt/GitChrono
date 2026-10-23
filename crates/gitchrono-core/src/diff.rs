//! Unified-diff parser for `git diff` / `git show` output.

use crate::model::*;

pub fn parse_patch(path: &str, patch: &str) -> FileDiff {
    let mut diff = FileDiff { path: path.to_string(), ..Default::default() };
    let mut old_no = 0u32;
    let mut new_no = 0u32;

    for line in patch.lines() {
        if line.starts_with("Binary files ") || line == "GIT binary patch" {
            diff.binary = true;
            continue;
        }
        if let Some(header) = line.strip_prefix("@@ ") {
            let (old, new) = parse_hunk_header(header);
            old_no = old;
            new_no = new;
            diff.hunks.push(Hunk { header: line.to_string(), lines: Vec::new() });
            continue;
        }
        let Some(hunk) = diff.hunks.last_mut() else { continue };
        let (kind, content) = match line.as_bytes().first() {
            Some(b'+') => (DiffLineKind::Add, &line[1..]),
            Some(b'-') => (DiffLineKind::Del, &line[1..]),
            Some(b' ') => (DiffLineKind::Context, &line[1..]),
            Some(b'\\') => continue, // "\ No newline at end of file"
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
        hunk.lines.push(DiffLine { kind, content: content.to_string(), old_no: o, new_no: n });
    }
    diff
}

/// `-12,5 +14,7 @@ fn x()` -> (12, 14)
fn parse_hunk_header(header: &str) -> (u32, u32) {
    let mut parts = header.split(' ');
    let start = |s: Option<&str>| {
        s.and_then(|s| s[1..].split(',').next()?.parse().ok()).unwrap_or(1)
    };
    let old = start(parts.next());
    let new = start(parts.next());
    (old, new)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_hunks_and_line_numbers() {
        let patch = "diff --git a/f b/f\n--- a/f\n+++ b/f\n@@ -1,3 +1,3 @@\n a\n-b\n+B\n c\n";
        let d = parse_patch("f", patch);
        assert_eq!(d.hunks.len(), 1);
        assert_eq!((d.additions, d.deletions), (1, 1));
        let lines = &d.hunks[0].lines;
        assert_eq!(lines[1].old_no, Some(2));
        assert_eq!(lines[2].new_no, Some(2));
        assert_eq!(lines[3].old_no, Some(3));
    }
}
