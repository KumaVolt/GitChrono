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

const COMMIT_SYSTEM: &str = "You write git commit messages. Reply with the message only: no quotes, no code fences, no commentary.
Line 1 is a summary of at most 72 characters in the imperative mood (\"Add\", \"Fix\", not \"Added\").
If the change needs explaining, add a blank line and a short body (wrapped at 72 columns) that says why, not how. Omit the body for small or obvious changes.
Match the conventions of the repository's recent commits (prefixes like \"feat:\" or ticket ids, capitalisation, language). If they follow no convention, use plain sentences.
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
    Request { system: COMMIT_SYSTEM, user, max_tokens: 300 }
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

/// Split model output into summary and body, tolerating fences and quotes.
pub fn parse_commit_message(text: &str) -> CommitMessage {
    let text = strip_wrapping(text);
    let mut lines = text.lines();
    let summary = lines.by_ref().map(str::trim).find(|l| !l.is_empty()).unwrap_or_default();
    let summary = summary.trim_matches(['"', '`', '\'']).to_string();
    let body = lines.collect::<Vec<_>>().join("\n").trim().to_string();
    CommitMessage { summary, body }
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

fn strip_wrapping(text: &str) -> &str {
    let text = text.trim();
    let text = text.strip_prefix("```").map_or(text, |rest| rest.split_once('\n').map_or(rest, |(_, body)| body));
    text.strip_suffix("```").unwrap_or(text).trim()
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
    fn sanitizes_branch_names() {
        assert_eq!(sanitize_ref_name("`feature/Add Search Box`"), "feature/add-search-box");
        assert_eq!(sanitize_ref_name("fix: crash!!\nexplanation"), "fix-crash");
        assert_eq!(sanitize_ref_name("/-weird-/name-"), "weird/name");
    }
}
