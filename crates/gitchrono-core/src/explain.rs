//! Plain-language versions of git's errors. Nobody should have to read git's
//! own output to find out what happened: each error gets a sentence saying
//! what went wrong and, where there is one, a fix the app can offer. The raw
//! output stays available as details.

use crate::cli::GitError;

/// Something the app can do about an error. An error with a fix isn't
/// shown as a failure but as a choice: git stopped to keep something safe.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Fix {
    /// Pull again, setting uncommitted changes aside meanwhile and putting
    /// them back afterwards (`git pull --autostash`).
    StashAndPull,
    /// Stash the uncommitted changes so the operation can be tried again.
    Stash,
    /// Pull with a merge (when a fast-forward-only pull can't).
    PullMerge,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Explained {
    /// One or two sentences for the user.
    pub message: String,
    /// Files the error is about, when git lists them. Not shown: the list can
    /// run to hundreds; the message gives the count.
    pub files: Vec<String>,
    pub fix: Option<Fix>,
    /// What git itself said, for a "Details" view.
    pub details: Option<String>,
}

impl Explained {
    pub fn plain(message: impl Into<String>) -> Self {
        Self { message: message.into(), files: Vec::new(), fix: None, details: None }
    }
}

/// Explains any error: git's own failures get a friendly message, other
/// errors (already written for people) pass through.
pub fn explain(error: &anyhow::Error) -> Explained {
    match error.downcast_ref::<GitError>() {
        Some(git) => explain_git(git),
        None => Explained::plain(format!("{error:#}")),
    }
}

/// What the command was trying to do, for "Couldn't …" messages.
fn doing(command: &str) -> &'static str {
    match command.split_whitespace().next().unwrap_or_default() {
        "pull" => "pull",
        "push" => "push",
        "fetch" => "fetch",
        "checkout" | "switch" => "switch branches",
        "merge" => "merge",
        "rebase" => "rebase",
        "cherry-pick" => "cherry-pick",
        "revert" => "revert",
        "commit" => "commit",
        "stash" => "stash",
        "reset" => "reset",
        "tag" => "tag",
        "branch" => "change the branch",
        "clone" => "clone",
        "apply" | "add" | "restore" | "rm" => "update the files",
        _ => "finish",
    }
}

pub fn explain_git(git: &GitError) -> Explained {
    let text = git.stderr.as_str();
    let has = |needles: &[&str]| needles.iter().any(|n| text.contains(n));
    let action = doing(&git.command);
    let pulling = git.command.starts_with("pull");
    let details = Some(format!("git {}\n\n{}", git.command, text.trim())).filter(|_| !text.trim().is_empty());
    let explained = |message: String, files: Vec<String>, fix: Option<Fix>| Explained { message, files, fix, details: details.clone() };

    if has(&["git-lfs' was not found", "git-lfs was not found", "'lfs' is not a git command", "git: 'lfs'"]) {
        return explained(
            format!("Couldn't {action}: this repository keeps some files in Git LFS, which isn't installed. Install it from git-lfs.com, then try again."),
            Vec::new(),
            None,
        );
    }
    if has(&["would be overwritten by", "Please commit your changes or stash them", "You have unstaged changes", "Your index contains uncommitted changes"]) {
        let files = listed_files(text);
        let untracked = text.contains("untracked working tree files");
        let what = match (files.len(), untracked) {
            (0, _) => "your uncommitted changes".to_string(),
            (1, false) => "your uncommitted changes to 1 file".to_string(),
            (n, false) => format!("your uncommitted changes to {n} files"),
            (1, true) => "1 untracked file".to_string(),
            (n, true) => format!("{n} untracked files"),
        };
        let message = if pulling {
            format!("The incoming commits would overwrite {what}. Stash and pull sets them aside, pulls, then puts them back.")
        } else {
            format!("Can't {action} yet: it would overwrite {what}. Stash them first, then try again.")
        };
        let fix = if pulling { Fix::StashAndPull } else { Fix::Stash };
        return explained(message, files, Some(fix));
    }
    if has(&["Not possible to fast-forward", "divergent branches", "Diverging branches can't be fast-forwarded"]) {
        return explained(
            "This branch and the remote have both moved on, so they can't simply be fast-forwarded. Pull with merge brings them together.".into(),
            Vec::new(),
            Some(Fix::PullMerge),
        );
    }
    if has(&["CONFLICT", "Merge conflict", "could not apply", "fix conflicts", "needs merge", "you need to resolve your current index first"]) {
        let files: Vec<String> = text.lines().filter_map(|l| l.split_once("Merge conflict in ").map(|(_, f)| f.trim().to_string())).collect();
        let message = match files.len() {
            0 => format!("Couldn't {action} cleanly: some changes conflict. Resolve the conflicted files, then continue."),
            1 => format!("Couldn't {action} cleanly: 1 file has conflicting changes. Resolve it, then continue."),
            n => format!("Couldn't {action} cleanly: {n} files have conflicting changes. Resolve them, then continue."),
        };
        return explained(message, files, None);
    }
    if has(&["Authentication failed", "Permission denied (publickey)", "could not read Username", "could not read Password", "terminal prompts disabled", "Invalid username or password", "returned error: 403"]) {
        return explained(
            format!("Couldn't {action}: the remote didn't accept your credentials. Check that you're signed in to it (a credential manager or an SSH key), then try again."),
            Vec::new(),
            None,
        );
    }
    if has(&["Repository not found", "does not appear to be a git repository"]) {
        return explained(
            format!("Couldn't {action}: the remote repository wasn't found. It may have moved, or your account may not have access to it."),
            Vec::new(),
            None,
        );
    }
    if has(&["Could not resolve host", "Connection timed out", "Connection refused", "Operation timed out", "Network is unreachable", "Failed to connect", "unable to access", "Could not read from remote repository"]) {
        return explained(
            format!("Couldn't {action}: the remote couldn't be reached. Check your internet connection (and VPN, if it needs one), then try again."),
            Vec::new(),
            None,
        );
    }
    if has(&["index.lock", "Another git process seems to be running"]) {
        return explained(
            format!("Couldn't {action}: another git program is working in this repository right now. Wait for it to finish, then try again."),
            Vec::new(),
            None,
        );
    }
    if has(&["nothing to commit", "no changes added to commit"]) {
        return explained("There's nothing to commit: stage some changes first.".into(), Vec::new(), None);
    }
    if has(&["Please tell me who you are", "unable to auto-detect email address"]) {
        return explained(
            "Couldn't commit: git doesn't know your name and email yet. Set them in Settings → Identity.".into(),
            Vec::new(),
            None,
        );
    }
    if has(&["gpg failed to sign", "error: cannot run gpg", "failed to sign the data"]) {
        return explained(
            format!("Couldn't {action}: signing the commit failed. Check that your signing key (GPG or SSH) is set up and unlocked."),
            Vec::new(),
            None,
        );
    }
    if has(&["hook declined", "pre-commit hook", "pre-push hook", "hook exited with error", "hook failed"]) {
        return explained(
            format!("Couldn't {action}: one of this repository's git hooks stopped it. Its message is in the details."),
            Vec::new(),
            None,
        );
    }

    // Anything else: git's first real message, without its "fatal:"/"error:"
    // prefixes and "hint:" lines.
    let line = text
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with("hint:"))
        .map(|l| l.strip_prefix("fatal: ").or_else(|| l.strip_prefix("error: ")).unwrap_or(l))
        .next()
        .map(sentence);
    let message = match line {
        Some(line) => format!("Couldn't {action}: {line}"),
        None => format!("Couldn't {action}: git stopped without saying why."),
    };
    explained(message, Vec::new(), None)
}

/// The files git lists under "…would be overwritten by merge:" (indented,
/// one per line, until "Please commit…" or "Aborting").
fn listed_files(text: &str) -> Vec<String> {
    let mut files = Vec::new();
    let mut listing = false;
    for line in text.lines() {
        if line.contains("would be overwritten by") || line.contains("would be removed by") {
            listing = true;
            continue;
        }
        if !listing {
            continue;
        }
        // The paths are indented; the first line that isn't ends the list.
        let file = line.trim();
        if file.is_empty() || !line.starts_with(['\t', ' ']) {
            listing = false;
            continue;
        }
        if !files.iter().any(|f| f == file) {
            files.push(file.to_string());
        }
    }
    files
}

/// Starts with a lowercase letter (after "Couldn't …:") and ends with a full stop.
fn sentence(line: &str) -> String {
    let mut s = line.trim_end_matches('.').to_string();
    if s.chars().nth(1).is_some_and(|c| c.is_lowercase()) {
        if let Some(first) = s.get(0..1) {
            s.replace_range(0..1, &first.to_lowercase());
        }
    }
    s.push('.');
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    fn err(command: &str, stderr: &str) -> GitError {
        GitError { command: command.into(), stderr: stderr.into(), code: Some(1) }
    }

    #[test]
    fn a_pull_blocked_by_local_changes_offers_stash_and_pull() {
        // What a colleague saw on Windows, as a wall of red text.
        let stderr = "From git.example.com:team/site\n   c6b476e..cbc21ecf  design_system -> origin/design_system\nerror: Your local changes to the following files would be overwritten by merge:\n\tresources/css/vendor/a/app.css\n\tresources/css/vendor/b/app.css\nPlease commit your changes or stash them before you merge.\nAborting\nMerge with strategy ort failed.";
        let e = explain_git(&err("pull --no-rebase", stderr));
        assert_eq!(e.fix, Some(Fix::StashAndPull));
        assert_eq!(e.files, ["resources/css/vendor/a/app.css", "resources/css/vendor/b/app.css"]);
        assert!(e.message.starts_with("The incoming commits would overwrite your uncommitted changes to 2 files."), "{}", e.message);
        assert!(e.details.unwrap().contains("Merge with strategy ort failed"), "git's text is kept for Details");
    }

    #[test]
    fn a_checkout_blocked_by_local_changes_offers_a_stash() {
        let e = explain_git(&err("checkout feature", "error: Your local changes to the following files would be overwritten by checkout:\n\tsrc/main.rs\nPlease commit your changes or stash them before you switch branches.\nAborting"));
        assert_eq!((e.fix, e.files.as_slice()), (Some(Fix::Stash), ["src/main.rs".to_string()].as_slice()));
        assert!(e.message.starts_with("Can't switch branches yet: it would overwrite your uncommitted changes to 1 file."), "{}", e.message);
    }

    #[test]
    fn untracked_files_in_the_way_are_named_as_such() {
        let e = explain_git(&err("pull --no-rebase", "error: The following untracked working tree files would be overwritten by merge:\n\tnotes.txt\nPlease move or remove them before you merge.\nAborting"));
        assert!(e.message.contains("would overwrite 1 untracked file."), "{}", e.message);
        assert_eq!(e.files, ["notes.txt"]);
    }

    #[test]
    fn common_failures_read_like_sentences() {
        let cases = [
            ("push", "fatal: unable to access 'https://github.com/a/b.git/': Could not resolve host: github.com", "the remote couldn't be reached"),
            ("fetch", "git@github.com: Permission denied (publickey).\nfatal: Could not read from remote repository.", "didn't accept your credentials"),
            ("commit -m x", "fatal: Unable to create '/r/.git/index.lock': File exists.\n\nAnother git process seems to be running", "another git program"),
            ("pull --ff-only", "fatal: Not possible to fast-forward, aborting.", "both moved on"),
            ("push", "This repository is configured for Git LFS but 'git-lfs' was not found on your path.", "Git LFS, which isn't installed"),
            ("merge feature", "CONFLICT (content): Merge conflict in src/a.rs\nCONFLICT (content): Merge conflict in src/b.rs", "2 files have conflicting changes"),
        ];
        for (command, stderr, expected) in cases {
            let e = explain_git(&err(command, stderr));
            assert!(e.message.contains(expected), "{command}: {}", e.message);
        }
    }

    #[test]
    fn unknown_errors_keep_gits_message_without_its_prefixes() {
        let e = explain_git(&err("tag v1", "fatal: tag 'v1' already exists\nhint: use -f"));
        assert_eq!(e.message, "Couldn't tag: tag 'v1' already exists.");
        let empty = explain_git(&err("rebase main", ""));
        assert_eq!((empty.message.as_str(), empty.details), ("Couldn't rebase: git stopped without saying why.", None));
    }

    #[test]
    fn errors_that_are_not_gits_pass_through() {
        let e = explain(&anyhow::anyhow!("This branch has no upstream."));
        assert_eq!((e.message.as_str(), e.fix, e.details), ("This branch has no upstream.", None, None));
    }
}
