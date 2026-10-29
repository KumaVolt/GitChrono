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

#[test]
fn worktree_add_list_remove() {
    let (dir, repo) = repo("worktree");
    let path = dir.with_file_name(format!("{}-wt", dir.file_name().unwrap().to_string_lossy()));
    let _ = std::fs::remove_dir_all(&path);
    repo.add_worktree(&path, Some("feature/wt"), None).unwrap();

    let list = repo.worktrees().unwrap();
    assert_eq!(list.len(), 2);
    assert!(list[0].main && !list[1].main);
    assert_eq!(list[1].branch.as_deref(), Some("feature/wt"));
    assert_eq!(list[1].path.canonicalize().unwrap(), path.canonicalize().unwrap());

    repo.remove_worktree(&path, false).unwrap();
    assert_eq!(repo.worktrees().unwrap().len(), 1);
}

#[test]
fn undo_reword() {
    let (dir, repo) = repo("reword");
    commit_file(&dir, "b.txt", "b\n", "second");
    let entry = record(&repo, "Rewording", true, || repo.reword("HEAD~1", "First, reworded").map(drop).unwrap());
    assert_eq!(git(&dir, &["log", "--format=%s"]).unwrap(), "second\nFirst, reworded\n");
    repo.undo(&entry).unwrap();
    assert_eq!(git(&dir, &["log", "--format=%s"]).unwrap(), "second\nfirst\n");
    repo.redo(&entry).unwrap();
    assert_eq!(git(&dir, &["log", "--format=%s"]).unwrap(), "second\nFirst, reworded\n");
}

fn stash_list(dir: &Path) -> Vec<String> {
    git(dir, &["stash", "list", "--format=%gs"]).unwrap().lines().map(str::to_string).collect()
}

#[test]
fn rename_stash_in_place_and_undo() {
    let (dir, repo) = repo("stash");
    for n in ["one", "two", "three"] {
        std::fs::write(dir.join("a.txt"), format!("stashed {n}\n")).unwrap();
        repo.stash_push(Some(n)).unwrap();
    }
    assert_eq!(stash_list(&dir), ["On main: three", "On main: two", "On main: one"]);
    let tree = git(&dir, &["rev-parse", "stash@{1}^{tree}"]).unwrap();
    let date = git(&dir, &["log", "-1", "--format=%ad", "stash@{1}"]).unwrap();

    let edit = repo.rename_stash(1, "On main: second\n\nwith a body").unwrap();
    assert_eq!(stash_list(&dir), ["On main: three", "On main: second", "On main: one"]);
    assert_eq!(git(&dir, &["log", "-1", "--format=%B", "stash@{1}"]).unwrap().trim(), "On main: second\n\nwith a body");
    assert_eq!(git(&dir, &["rev-parse", "stash@{1}^{tree}"]).unwrap(), tree, "same changes");
    assert_eq!(git(&dir, &["log", "-1", "--format=%ad", "stash@{1}"]).unwrap(), date, "same date");
    assert!(repo.rename_stash(7, "nope").is_err());

    // The stash moved down a slot meanwhile: undo still finds it.
    std::fs::write(dir.join("a.txt"), "four\n").unwrap();
    repo.stash_push(Some("four")).unwrap();
    let entry = UndoEntry::stash("Renaming stash", edit);
    repo.undo(&entry).unwrap();
    assert_eq!(stash_list(&dir), ["On main: four", "On main: three", "On main: two", "On main: one"]);
    repo.redo(&entry).unwrap();
    assert_eq!(stash_list(&dir)[2], "On main: second");

    repo.stash_drop(2).unwrap();
    assert!(repo.undo(&entry).is_err(), "dropped since");
    repo.stash_pop(0).unwrap();
    assert_eq!(std::fs::read_to_string(dir.join("a.txt")).unwrap(), "four\n");
}
