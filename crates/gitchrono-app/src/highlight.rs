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

/// bat's extended set (two-face): TypeScript, TSX, Vue, Svelte, TOML,
/// Dockerfile, Kotlin, Swift, … on top of syntect's defaults.
static SYNTAXES: LazyLock<SyntaxSet> = LazyLock::new(two_face::syntax::extra_newlines);
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
    // Hunks start mid-file, and the PHP grammar starts in HTML mode until
    // `<?php`, so plain PHP files use the code-only grammar. Templates
    // (Blade, .phtml) really are HTML with PHP in it.
    if name.ends_with(".php") && !name.ends_with(".blade.php") {
        if let Some(code) = SYNTAXES.find_syntax_by_name("PHP Source") {
            return Some(code);
        }
    }
    let ext = name.rsplit_once('.').map_or(name, |(_, e)| e);
    // Extensions the grammars don't list themselves.
    let ext = match ext {
        "jsx" | "mjs" | "cjs" => "js",
        "mts" | "cts" => "ts",
        other => other,
    };
    SYNTAXES.find_syntax_by_extension(ext).or_else(|| SYNTAXES.find_syntax_by_extension(name)).or_else(|| SYNTAXES.find_syntax_by_token(ext))
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
            out.insert((h, l), colored(&spans, &text));
        }
    }
    out
}

/// Colored spans for each line of a whole file, read top to bottom; empty
/// when it has no grammar or is too big.
pub fn highlight_lines(path: &str, lines: &[&str]) -> Vec<Spans> {
    let Some(syntax) = syntax_for(path).filter(|_| lines.len() <= MAX_LINES) else { return Vec::new() };
    let mut highlighter = HighlightLines::new(syntax, &THEME);
    lines
        .iter()
        .map(|line| {
            let text = display_text(line);
            highlighter.highlight_line(&format!("{text}\n"), &SYNTAXES).map_or_else(|_| Spans::new(), |spans| colored(&spans, &text))
        })
        .collect()
}

fn colored(spans: &[(syntect::highlighting::Style, &str)], text: &str) -> Spans {
    let mut offset = 0;
    let mut out = Spans::new();
    for (style, piece) in spans {
        let end = (offset + piece.len()).min(text.len());
        if end > offset {
            let fg = style.foreground;
            let color = Rgba { r: fg.r as f32 / 255., g: fg.g as f32 / 255., b: fg.b as f32 / 255., a: 1. };
            out.push((offset..end, color.into()));
        }
        offset += piece.len();
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

    #[test]
    fn php_hunks_from_the_middle_of_a_file_are_colored() {
        // No `<?php` in the hunk: the HTML-mode grammar would leave it plain.
        let patch = "diff --git a/A.php b/A.php\n--- a/A.php\n+++ b/A.php\n@@ -10,2 +10,2 @@\n     public function run(): void\n-        $a = 1;\n+        $b = strtoupper('two');\n";
        let diff = parse_patch("A.php", patch);
        let hl = highlight_diff("app/A.php", &diff);
        let colors: std::collections::HashSet<_> = hl[&(0, 2)].iter().map(|(_, c)| format!("{c:?}")).collect();
        assert!(colors.len() > 2, "expected several colors, got {colors:?}");
    }

    #[test]
    fn whole_files_keep_state_across_lines() {
        let lines = ["/* a comment", "still the comment */", "fn main() {}"];
        let spans = highlight_lines("x.rs", &lines);
        assert_eq!(spans.len(), 3);
        assert_eq!(spans[0][0].1, spans[1][0].1, "the comment carries on");
        assert_ne!(spans[1][0].1, spans[2][0].1);
        assert!(highlight_lines("notes.unknownext", &lines).is_empty());
    }

    #[test]
    fn common_languages_are_covered() {
        for path in ["a.ts", "a.tsx", "a.jsx", "a.vue", "a.svelte", "a.scss", "Cargo.toml", "a.kt", "a.swift", "a.dart", "Dockerfile", "a.rs", "a.py", "a.go", "a.blade.php", "a.php"] {
            assert!(syntax_for(path).is_some(), "no syntax for {path}");
        }
    }
}

