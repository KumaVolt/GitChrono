//! Commit bodies are hard-wrapped (usually at 72 columns); for display in a
//! panel of any width the wrapped lines are joined back into paragraphs,
//! keeping bullets, numbered items, trailers and indented code on their own.

/// The body as lines to show; an empty string is a blank line between paragraphs.
pub fn reflow(body: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    // Whether the last output line may take a continuation.
    let mut open = false;
    for raw in body.trim().lines() {
        let line = raw.trim_end();
        if line.trim().is_empty() {
            if out.last().is_some_and(|l| !l.is_empty()) {
                out.push(String::new());
            }
            open = false;
            continue;
        }
        let indent = line.len() - line.trim_start().len();
        let text = line.trim_start();
        let starts_item = is_item(text) || is_trailer(text);
        let code = indent >= 4 && !out.last().is_some_and(|l| is_item(l.trim_start()));
        if open && !starts_item && !code {
            let last = out.last_mut().expect("open implies a line");
            last.push(' ');
            last.push_str(text);
        } else {
            out.push(if code { line.to_string() } else { text.to_string() });
            open = !code && !is_trailer(text);
        }
    }
    out
}

/// `Co-authored-by: Name <email>` trailers, as (name, email), in order and without repeats.
pub fn co_authors(body: &str) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = Vec::new();
    for line in body.lines() {
        let Some((key, value)) = line.trim().split_once(':') else { continue };
        if !key.trim().eq_ignore_ascii_case("co-authored-by") {
            continue;
        }
        let value = value.trim();
        let (name, email) = match value.split_once('<') {
            Some((name, rest)) => (name.trim(), rest.trim_end_matches('>').trim()),
            None => (value, ""),
        };
        if !name.is_empty() && !out.iter().any(|(n, e)| n == name && e == email) {
            out.push((name.to_string(), email.to_string()));
        }
    }
    out
}

fn is_item(text: &str) -> bool {
    ["- ", "* ", "• ", "> "].iter().any(|p| text.starts_with(p)) || {
        let digits = text.chars().take_while(char::is_ascii_digit).count();
        digits > 0 && (text[digits..].starts_with(". ") || text[digits..].starts_with(") "))
    }
}

/// `Co-Authored-By: …`, `Signed-off-by: …`, `Fixes: …`
fn is_trailer(text: &str) -> bool {
    text.split_once(": ").is_some_and(|(key, _)| !key.is_empty() && key.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') && key.chars().next().is_some_and(|c| c.is_ascii_uppercase()))
}

#[cfg(test)]
mod tests {
    use super::{co_authors, reflow};

    #[test]
    fn joins_wrapped_lines_and_keeps_structure() {
        let body = "Branches forked from the same commit used to run side by side until that\n\
                    commit's row; they now join the lower lane.\n\
                    \n\
                    - Commit details: message in a card, author initials avatar, absolute and\n  \
                      relative date, clickable parents.\n\
                    - Toolbar: Stash and Pop side by side.\n\
                    \n\
                    Example:\n    \
                        cargo run -p gitchrono-app\n\
                    \n\
                    Co-Authored-By: Claude <noreply@anthropic.com>\n\
                    Claude-Session: https://claude.ai/x";
        assert_eq!(
            reflow(body),
            [
                "Branches forked from the same commit used to run side by side until that commit's row; they now join the lower lane.",
                "",
                "- Commit details: message in a card, author initials avatar, absolute and relative date, clickable parents.",
                "- Toolbar: Stash and Pop side by side.",
                "",
                "Example:",
                "    cargo run -p gitchrono-app",
                "",
                "Co-Authored-By: Claude <noreply@anthropic.com>",
                "Claude-Session: https://claude.ai/x",
            ]
        );
    }

    #[test]
    fn finds_co_authors() {
        let body = "Fix it\n\nCo-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>\nco-authored-by: Ada Park <ada@example.com>\nCo-authored-by: Ada Park <ada@example.com>\nSigned-off-by: Leo <leo@x>";
        assert_eq!(
            co_authors(body),
            [("Claude Opus 5.5".to_string(), "noreply@anthropic.com".to_string()), ("Ada Park".to_string(), "ada@example.com".to_string())]
        );
    }

    #[test]
    fn numbered_items_stay_separate() {
        assert_eq!(reflow("1. one\n2) two\nmore"), ["1. one", "2) two more"]);
    }
}
