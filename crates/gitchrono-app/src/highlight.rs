//! Syntax highlighting for diffs (syntect, pure-Rust regex engine).
//!
//! Old and new sides are highlighted as two separate line streams per hunk so
//! multi-line constructs (comments, strings) keep their state across lines.

use std::collections::HashMap;
use std::ops::Range;
use std::sync::LazyLock;

use gitchrono_core::{DiffLineKind, FileDiff};
use gpui_kit::{Hsla, Rgba};
use syntect::easy::HighlightLines;
use syntect::highlighting::{Theme, ThemeSet};
use syntect::parsing::{SyntaxReference, SyntaxSet};

/// Diffs bigger than this stay plain: highlighting cost grows with size.
const MAX_LINES: usize = 6000;

static SYNTAXES: LazyLock<SyntaxSet> = LazyLock::new(SyntaxSet::load_defaults_newlines);
static THEME: LazyLock<Theme> = LazyLock::new(|| {
    let mut themes = ThemeSet::load_defaults().themes;
    themes.remove("base16-ocean.dark").unwrap_or_default()
});

pub type Spans = Vec<(Range<usize>, Hsla)>;
/// Colored spans per `(hunk, line)`, over [`display_text`] of the line.
pub type Highlights = HashMap<(usize, usize), Spans>;

/// How a diff line is shown: no trailing CR, tabs as four spaces.
pub fn display_text(content: &str) -> String {
    content.trim_end_matches('\r').replace('\t', "    ")
}

fn syntax_for(path: &str) -> Option<&'static SyntaxReference> {
    let name = path.rsplit('/').next().unwrap_or(path);
    let ext = name.rsplit_once('.').map_or(name, |(_, e)| e);
    SYNTAXES.find_syntax_by_extension(ext).or_else(|| SYNTAXES.find_syntax_by_extension(name))
}

pub fn highlight_diff(path: &str, diff: &FileDiff) -> Highlights {
    let mut out = Highlights::new();
    let total: usize = diff.hunks.iter().map(|h| h.lines.len()).sum();
    let Some(syntax) = syntax_for(path).filter(|_| total <= MAX_LINES && !diff.binary) else { return out };
    for (h, hunk) in diff.hunks.iter().enumerate() {
        let mut old = HighlightLines::new(syntax, &THEME);
        let mut new = HighlightLines::new(syntax, &THEME);
        for (l, line) in hunk.lines.iter().enumerate() {
            let text = display_text(&line.content);
            let with_newline = format!("{text}\n");
            let spans = match line.kind {
                DiffLineKind::Context => {
                    // Advance the old side too so both stay in sync.
                    let _ = old.highlight_line(&with_newline, &SYNTAXES);
                    new.highlight_line(&with_newline, &SYNTAXES)
                }
                DiffLineKind::Del => old.highlight_line(&with_newline, &SYNTAXES),
                DiffLineKind::Add => new.highlight_line(&with_newline, &SYNTAXES),
            };
            let Ok(spans) = spans else { continue };
            let mut offset = 0;
            let mut colored = Spans::new();
            for (style, piece) in spans {
                let end = (offset + piece.len()).min(text.len());
                if end > offset {
                    let fg = style.foreground;
                    let color = Rgba { r: fg.r as f32 / 255., g: fg.g as f32 / 255., b: fg.b as f32 / 255., a: 1. };
                    colored.push((offset..end, color.into()));
                }
                offset += piece.len();
            }
            out.insert((h, l), colored);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use gitchrono_core::parse_patch;

    #[test]
    fn highlights_rust_and_skips_unknown() {
        let patch = "diff --git a/x.rs b/x.rs\n--- a/x.rs\n+++ b/x.rs\n@@ -1,2 +1,2 @@\n fn main() {\n-    let a = 1;\n+    let b = \"two\";\n";
        let diff = parse_patch("x.rs", patch);
        let hl = highlight_diff("src/x.rs", &diff);
        let spans = &hl[&(0, 2)];
        assert!(spans.len() > 2, "expected several colored spans, got {spans:?}");
        let text = display_text(&diff.hunks[0].lines[2].content);
        assert_eq!(spans.last().unwrap().0.end, text.len());

        assert!(highlight_diff("notes.unknownext", &diff).is_empty());
    }
}
