//! Undo / redo round trips against a real `git`.

use std::path::{Path, PathBuf};

use gitchrono_core::{Repo, UndoEntry, git};

fn repo(name: &str) -> (PathBuf, Repo) {
    let dir = std::env::temp_dir().join(format!("gitchrono-undo-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    for args in [
        &["init", "-q", "-b", "main"][..],
        &["config", "user.name", "Test"],
        &["config", "user.email", "test@example.com"],
        &["config", "commit.gpgsign", "false"],
        &["config", "core.autocrlf", "false"],
    ] {
        git(&dir, args).unwrap();
    }
    commit_file(&dir, "a.txt", "one\n", "first");
    let repo = Repo::open(&dir).unwrap();
    (dir, repo)
}

fn commit_file(dir: &Path, file: &str, text: &str, message: &str) {
    std::fs::write(dir.join(file), text).unwrap();
    git(dir, &["add", file]).unwrap();
    git(dir, &["commit", "-qm", message]).unwrap();
}

/// Run `op`, returning its undo entry.
fn record(repo: &Repo, label: &str, soft: bool, op: impl FnOnce()) -> UndoEntry {
    let before = repo.ref_state().unwrap();
    op();
    UndoEntry::new(label, before, repo.ref_state().unwrap(), soft).expect("op changed refs")
}

fn head(dir: &Path) -> String {
    git(dir, &["rev-parse", "HEAD"]).unwrap().trim().to_string()
}

#[test]
fn undo_commit_keeps_changes_staged() {
    let (dir, repo) = repo("commit");
    let first = head(&dir);
    let entry = record(&repo, "Committing", true, || {
        std::fs::write(dir.join("a.txt"), "two\n").unwrap();
        repo.stage(&["a.txt"]).unwrap();
        repo.commit("second", false).unwrap();
    });
    let second = head(&dir);

    repo.undo(&entry).unwrap();
    assert_eq!(head(&dir), first);
    assert_eq!(git(&dir, &["diff", "--cached", "--name-only"]).unwrap().trim(), "a.txt");

    repo.redo(&entry).unwrap();
    assert_eq!(head(&dir), second);
    assert!(git(&dir, &["status", "--porcelain"]).unwrap().trim().is_empty());
}

#[test]
fn undo_create_and_checkout_branch() {
    let (dir, repo) = repo("branch");
    let entry = record(&repo, "Creating branch", false, || repo.create_branch("feature", None, true).unwrap());
    repo.undo(&entry).unwrap();
    assert_eq!(repo.ref_state().unwrap().head_branch.as_deref(), Some("main"));
    assert!(!repo.ref_state().unwrap().branches.contains_key("feature"));

    repo.redo(&entry).unwrap();
    assert_eq!(repo.ref_state().unwrap().head_branch.as_deref(), Some("feature"));
    let _ = dir;
}

#[test]
fn undo_delete_branch_and_hard_reset() {
    let (dir, repo) = repo("delete");
    git(&dir, &["branch", "keep"]).unwrap();
    commit_file(&dir, "b.txt", "b\n", "second");
    let second = head(&dir);

    let delete = record(&repo, "Deleting branch", false, || repo.delete_branch("keep", true).unwrap());
    repo.undo(&delete).unwrap();
    assert!(repo.ref_state().unwrap().branches.contains_key("keep"));

    let reset = record(&repo, "Resetting", false, || {
        git(&dir, &["reset", "--hard", "-q", "HEAD~1"]).unwrap();
    });
    repo.undo(&reset).unwrap();
    assert_eq!(head(&dir), second);
    assert!(dir.join("b.txt").exists());
}

#[test]
fn refuses_when_repository_changed_meanwhile() {
    let (dir, repo) = repo("stale");
    let entry = record(&repo, "Creating branch", false, || repo.create_branch("x", None, false).unwrap());
    commit_file(&dir, "c.txt", "c\n", "outside the app");
    assert!(repo.undo(&entry).is_err());
}

#[test]
fn no_entry_without_ref_changes() {
    let (_dir, repo) = repo("noop");
    let state = repo.ref_state().unwrap();
    assert!(UndoEntry::new("Fetching", state.clone(), state, false).is_none());
}
