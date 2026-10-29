//! Prompt builders. Diffs are trimmed to a character budget (lockfiles and
//! generated files dropped, large files truncated) and scrubbed of obvious
//! secrets before anything leaves the machine.

use std::sync::LazyLock;

use regex::Regex;

use crate::provider::Request;

/// ~3k tokens of diff: plenty for a message, cheap on every provider.
const DIFF_BUDGET: usize = 12_000;
/// No single file may take more than this unless it's the only one.
const MIN_FILE_SHARE: usize = 1_500;

// The recent commits we show are subjects only, so the convention-matching
// rule is scoped to line 1: otherwise models read "no bodies" into them.
const COMMIT_SYSTEM: &str = "You write git commit messages. Reply with the message only: no quotes, no code fences, no labels like \"Summary:\", no commentary.
Line 1 is a summary of at most 72 characters in the imperative mood (\"Add\", \"Fix\", not \"Added\").
Then a blank line and a body: one to five short lines or \"- \" bullets, wrapped at 72 columns, saying what changed and why. Leave the body out only for trivial changes such as a typo fix or a version bump.
Match the summary-line conventions of the repository's recent commits (prefixes like \"feat:\" or ticket ids, capitalisation, language). Those are subjects only, so they say nothing about whether to write a body. If they follow no convention, use plain sentences.
Never invent details that the diff does not show.";

const BRANCH_SYSTEM: &str = "You name git branches. Reply with one branch name only: lowercase words joined by hyphens, at most 5 words, optionally prefixed with feature/, fix/ or chore/. No quotes or commentary.";

const STASH_SYSTEM: &str = "You label git stashes. Reply with one short line (at most 60 characters) describing the work in progress. No quotes or commentary.";

pub fn commit_message(diff: &str, recent_subjects: &[&str], branch: Option<&str>) -> Request<'static> {
    let mut user = String::new();
    if !recent_subjects.is_empty() {
        user.push_str("Recent commit subjects in this repository (newest first):\n");
        for s in recent_subjects.iter().take(30) {
            user.push_str("- ");
            user.push_str(s);
            user.push('\n');
        }
        user.push('\n');
    }
    if let Some(branch) = branch {
        user.push_str(&format!("Current branch: {branch}\n\n"));
    }
    user.push_str("Staged diff:\n");
    user.push_str(&prepare_diff(diff, DIFF_BUDGET));
    Request { system: COMMIT_SYSTEM, user, max_tokens: 400 }
}

const EXPLAIN_SYSTEM: &str = "You explain git commits to a developer reviewing history. Reply in plain text, no headings or code fences.
Start with one or two sentences on what the commit does and why it likely matters, then up to five short \"- \" bullets on the notable changes (files, behaviour, risks).
Be concrete and brief. Never invent details that the message and diff do not show.";

pub fn explain_commit(summary: &str, body: &str, diff: &str) -> Request<'static> {
    let mut user = format!("Commit message:\n{summary}\n");
    if !body.trim().is_empty() {
        user.push_str(&format!("\n{}\n", body.trim()));
    }
    user.push_str("\nDiff:\n");
    user.push_str(&prepare_diff(diff, DIFF_BUDGET));
    Request { system: EXPLAIN_SYSTEM, user, max_tokens: 400 }
}

pub fn branch_name(diff: &str) -> Request<'static> {
    Request { system: BRANCH_SYSTEM, user: format!("Changes:\n{}", prepare_diff(diff, DIFF_BUDGET / 2)), max_tokens: 30 }
}

pub fn stash_message(diff: &str) -> Request<'static> {
    Request { system: STASH_SYSTEM, user: format!("Changes:\n{}", prepare_diff(diff, DIFF_BUDGET / 2)), max_tokens: 40 }
}

#[derive(Debug, PartialEq, Eq)]
pub struct CommitMessage {
    pub summary: String,
    pub body: String,
}

const SUMMARY_LABELS: &[&str] = &["summary", "subject", "title", "commit message"];
const BODY_LABELS: &[&str] = &["description", "body", "details"];

/// Split model output into summary and body, tolerating fences, quotes,
/// a lead-in line ("Here is the commit message:") and "Summary:" /
/// "Description:" labels.
pub fn parse_commit_message(text: &str) -> CommitMessage {
    let text = strip_wrapping(text);
    let mut lines = text.lines();
    let mut summary = "";
    for line in lines.by_ref() {
        let line = line.trim();
        if line.ends_with(':') {
            continue;
        }
        summary = strip_label(line, SUMMARY_LABELS).unwrap_or(line);
        if !summary.is_empty() {
            break;
        }
    }
    let summary = summary.trim_matches(['"', '`', '\'']).to_string();
    let mut body: Vec<&str> = lines.collect();
    if let Some(first) = body.iter_mut().find(|l| !l.trim().is_empty()) {
        let line: &str = first;
        if let Some(rest) = strip_label(line.trim(), BODY_LABELS) {
            *first = rest;
        }
    }
    let body = body.join("\n").trim().to_string();
    CommitMessage { summary, body }
}

/// What follows a leading `Label:` (or `**Label:**`) when the label is one of `labels`.
fn strip_label<'a>(line: &'a str, labels: &[&str]) -> Option<&'a str> {
    let (head, rest) = line.trim_start_matches(['*', '#', ' ']).split_once(':')?;
    let head = head.trim().trim_end_matches('*');
    labels.iter().any(|l| head.eq_ignore_ascii_case(l)).then(|| rest.trim_start_matches('*').trim())
}

/// A model's branch suggestion, made safe for `git branch`.
pub fn sanitize_ref_name(text: &str) -> String {
    let line = strip_wrapping(text).lines().map(str::trim).find(|l| !l.is_empty()).unwrap_or_default().to_lowercase();
    let mut out = String::new();
    for ch in line.chars() {
        match ch {
            'a'..='z' | '0'..='9' | '/' | '.' | '_' => out.push(ch),
            _ if !out.ends_with('-') => out.push('-'),
            _ => {}
        }
    }
    out.split('/').map(|part| part.trim_matches(['-', '.'])).filter(|p| !p.is_empty()).collect::<Vec<_>>().join("/")
}

/// The contents of the first code fence if there is one (models sometimes
/// put a sentence before it), else the trimmed text.
fn strip_wrapping(text: &str) -> &str {
    let text = text.trim();
    let Some((_, rest)) = text.split_once("```") else { return text };
    let rest = match rest.split_once('\n') {
        Some((tag, body)) if tag.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') => body,
        _ => rest,
    };
    rest.split_once("```").map_or(rest, |(inner, _)| inner).trim()
}

fn is_noise(path: &str) -> bool {
    let name = path.rsplit('/').next().unwrap_or(path);
    matches!(
        name,
        "Cargo.lock"
            | "package-lock.json"
            | "yarn.lock"
            | "pnpm-lock.yaml"
            | "bun.lockb"
            | "composer.lock"
            | "Gemfile.lock"
            | "poetry.lock"
            | "uv.lock"
            | "go.sum"
            | "Podfile.lock"
            | "pubspec.lock"
    ) || name.ends_with(".min.js")
        || name.ends_with(".min.css")
        || name.ends_with(".map")
        || name.ends_with(".snap")
        || path.contains("/dist/")
        || path.starts_with("dist/")
}

fn is_secret_file(path: &str) -> bool {
    let name = path.rsplit('/').next().unwrap_or(path);
    (name == ".env" || name.starts_with(".env.")) && !name.ends_with(".example") && !name.ends_with(".sample")
        || name.ends_with(".pem")
        || name.ends_with(".key")
        || name == "id_rsa"
        || name == "id_ed25519"
}

static SECRET_ASSIGNMENT: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?i)((?:api[_-]?key|secret|token|passw(?:or)?d|private[_-]?key|access[_-]?key)["']?\s*(?:=>|[:=])\s*["']?)([^\s"',;]{8,})"#)
        .unwrap()
});
static SECRET_TOKEN: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\b(?:sk-[A-Za-z0-9_-]{16,}|gh[pousr]_[A-Za-z0-9]{20,}|AKIA[0-9A-Z]{16}|xox[abpr]-[A-Za-z0-9-]{10,}|AIza[0-9A-Za-z_-]{30,})")
        .unwrap()
});

fn scrub(line: &str) -> String {
    let line = SECRET_ASSIGNMENT.replace_all(line, "${1}[REDACTED]");
    SECRET_TOKEN.replace_all(&line, "[REDACTED]").into_owned()
}

/// Path from a `diff --git a/x b/x` header.
fn section_path(section: &str) -> &str {
    let header = section.lines().next().unwrap_or_default();
    header.rsplit_once(" b/").map_or(header, |(_, b)| b)
}

/// Trim a unified diff to roughly `budget` characters.
pub(crate) fn prepare_diff(diff: &str, budget: usize) -> String {
    let mut sections: Vec<&str> = Vec::new();
    let mut start = 0;
    for (i, _) in diff.match_indices("diff --git ") {
        if i == 0 || diff.as_bytes()[i - 1] == b'\n' {
            if i > start {
                sections.push(&diff[start..i]);
            }
            start = i;
        }
    }
    if start < diff.len() {
        sections.push(&diff[start..]);
    }

    let mut omitted = Vec::new();
    let mut kept = Vec::new();
    for section in sections {
        let path = section_path(section);
        if is_secret_file(path) {
            omitted.push(format!("{path} (contents withheld)"));
        } else if is_noise(path) {
            omitted.push(format!("{path} (generated/lockfile)"));
        } else {
            kept.push(section);
        }
    }

    let share = (budget / kept.len().max(1)).max(MIN_FILE_SHARE);
    let mut out = String::new();
    for (i, section) in kept.iter().enumerate() {
        if out.len() >= budget {
            let rest: Vec<&str> = kept[i..].iter().map(|s| section_path(s)).collect();
            out.push_str(&format!("[{} more file(s) not shown: {}]\n", rest.len(), rest.join(", ")));
            break;
        }
        let mut used = 0;
        for line in section.lines() {
            if used + line.len() > share {
                out.push_str("[… rest of this file truncated]\n");
                break;
            }
            let line = scrub(line);
            used += line.len() + 1;
            out.push_str(&line);
            out.push('\n');
        }
    }
    if !omitted.is_empty() {
        out.push_str("\nAlso changed (diff not shown):\n");
        for o in omitted {
            out.push_str(&format!("- {o}\n"));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(path: &str, body: &str) -> String {
        format!("diff --git a/{path} b/{path}\n--- a/{path}\n+++ b/{path}\n@@ -1 +1 @@\n{body}\n")
    }

    #[test]
    fn drops_lockfiles_and_secret_files() {
        let diff = file("src/main.rs", "+fn main() {}") + &file("Cargo.lock", "+version = 1") + &file(".env", "+DB_PASSWORD=hunter22");
        let out = prepare_diff(&diff, 10_000);
        assert!(out.contains("fn main"));
        assert!(!out.contains("version = 1"));
        assert!(!out.contains("hunter22"));
        assert!(out.contains("Cargo.lock (generated/lockfile)"));
        assert!(out.contains(".env (contents withheld)"));
    }

    #[test]
    fn scrubs_inline_secrets() {
        let diff = file("config.php", "+'api_key' => 'abcd1234efgh5678',\n+let t = \"sk-proj-ABCDEFGHIJKLMNOPQRSTUV\";");
        let out = prepare_diff(&diff, 10_000);
        assert!(!out.contains("abcd1234efgh5678"), "{out}");
        assert!(!out.contains("sk-proj-ABCDEFGHIJKLMNOPQRSTUV"), "{out}");
        assert!(out.contains("[REDACTED]"));
    }

    #[test]
    fn truncates_to_budget() {
        let big: String = (0..2000).map(|i| format!("+line {i}\n")).collect();
        let diff = file("a.txt", &big) + &file("b.txt", &big);
        let out = prepare_diff(&diff, 4_000);
        assert!(out.len() < 4_000 + 200, "{}", out.len());
        assert!(out.contains("truncated"));
        assert!(out.contains("b.txt"));
    }

    #[test]
    fn parses_messages() {
        let m = parse_commit_message("```\nfeat: add search\n\nIndexes commit subjects.\n```");
        assert_eq!(m, CommitMessage { summary: "feat: add search".into(), body: "Indexes commit subjects.".into() });
        let m = parse_commit_message("\"Fix crash on empty repo\"");
        assert_eq!(m.summary, "Fix crash on empty repo");
        assert!(m.body.is_empty());
    }

    #[test]
    fn parses_bodies_from_realistic_replies() {
        let expected = CommitMessage {
            summary: "Add commit search".into(),
            body: "Index subjects and bodies so the search box\nfinds older commits.".into(),
        };
        let replies = [
            // Plain, CRLF and a body with no blank line before it.
            "Add commit search\n\nIndex subjects and bodies so the search box\nfinds older commits.",
            "Add commit search\r\n\r\nIndex subjects and bodies so the search box\r\nfinds older commits.\r\n",
            "Add commit search\nIndex subjects and bodies so the search box\nfinds older commits.",
            // Fences with a language tag, and after a lead-in sentence.
            "```text\nAdd commit search\n\nIndex subjects and bodies so the search box\nfinds older commits.\n```",
            "Here is the commit message:\n\n```\nAdd commit search\n\nIndex subjects and bodies so the search box\nfinds older commits.\n```\n",
            // Lead-in without a fence.
            "Commit message:\nAdd commit search\n\nIndex subjects and bodies so the search box\nfinds older commits.",
            // Labels, inline and on their own line, plain and in markdown bold.
            "Summary: Add commit search\n\nDescription: Index subjects and bodies so the search box\nfinds older commits.",
            "**Summary:** Add commit search\n\n**Description:**\nIndex subjects and bodies so the search box\nfinds older commits.",
            "Subject:\nAdd commit search\n\nBody:\nIndex subjects and bodies so the search box\nfinds older commits.",
        ];
        for reply in replies {
            assert_eq!(parse_commit_message(reply), expected, "{reply:?}");
        }
    }

    #[test]
    fn keeps_conventional_prefixes_and_bullets() {
        let m = parse_commit_message("fix(ui): keep the description\n\n- Parse labelled replies\n- Ask for a body");
        assert_eq!(m.summary, "fix(ui): keep the description");
        assert_eq!(m.body, "- Parse labelled replies\n- Ask for a body");
    }

    #[test]
    fn asks_for_a_body() {
        let request = commit_message(&file("a.rs", "+x"), &["Fix typo"], None);
        assert!(request.system.contains("Then a blank line and a body"));
        assert!(!request.system.contains("Omit the body for small or obvious changes"));
    }

    #[test]
    fn sanitizes_branch_names() {
        assert_eq!(sanitize_ref_name("`feature/Add Search Box`"), "feature/add-search-box");
        assert_eq!(sanitize_ref_name("fix: crash!!\nexplanation"), "fix-crash");
        assert_eq!(sanitize_ref_name("/-weird-/name-"), "weird/name");
    }
}
