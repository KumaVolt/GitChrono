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
    use super::reflow;

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
    fn numbered_items_stay_separate() {
        assert_eq!(reflow("1. one\n2) two\nmore"), ["1. one", "2) two more"]);
    }
}
