//! Applying and popping stashes over uncommitted changes, against a real `git`.

use std::path::{Path, PathBuf};

use gitchrono_core::{Repo, UndoEntry, git};

fn repo(name: &str) -> (PathBuf, Repo) {
    let dir = std::env::temp_dir().join(format!("gitchrono-stash-{name}-{}", std::process::id()));
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
    for file in ["a.txt", "b.txt"] {
        std::fs::write(dir.join(file), "base\n").unwrap();
    }
    git(&dir, &["add", "."]).unwrap();
    git(&dir, &["commit", "-qm", "base"]).unwrap();
    let repo = Repo::open(&dir).unwrap();
    (dir, repo)
}

fn write(dir: &Path, file: &str, text: &str) {
    std::fs::write(dir.join(file), text).unwrap();
}

fn read(dir: &Path, file: &str) -> String {
    std::fs::read_to_string(dir.join(file)).unwrap_or_default()
}

/// Stash `file` with `text` in it, leaving the tree clean.
fn stash(dir: &Path, file: &str, text: &str, message: &str) {
    write(dir, file, text);
    git(dir, &["stash", "push", "-q", "--include-untracked", "-m", message]).unwrap();
}

fn stash_list(dir: &Path) -> Vec<String> {
    git(dir, &["stash", "list", "--format=%gs"]).unwrap().lines().map(str::to_string).collect()
}

fn stash_ids(dir: &Path) -> Vec<String> {
    git(dir, &["stash", "list", "--format=%H"]).unwrap().lines().map(str::to_string).collect()
}

#[test]
fn apply_and_pop_keep_changes_in_other_files() {
    let (dir, repo) = repo("disjoint");
    stash(&dir, "a.txt", "stashed\n", "one");
    stash(&dir, "a.txt", "stashed again\n", "two");
    write(&dir, "b.txt", "local\n");

    assert!(repo.apply_stash(1, false).unwrap().is_none(), "no clash, nothing set aside");
    assert_eq!(read(&dir, "a.txt"), "stashed\n");
    assert_eq!(read(&dir, "b.txt"), "local\n");
    assert_eq!(stash_list(&dir).len(), 2);

    git(&dir, &["checkout", "-q", "a.txt"]).unwrap();
    assert!(repo.apply_stash(0, true).unwrap().is_none());
    assert_eq!(read(&dir, "a.txt"), "stashed again\n");
    assert_eq!(read(&dir, "b.txt"), "local\n");
    assert_eq!(stash_list(&dir), ["On main: one"]);
}

#[test]
fn clash_stashes_changes_first_and_undo_restores_them() {
    let (dir, repo) = repo("clash");
    stash(&dir, "a.txt", "stashed\n", "wanted");
    let wanted = stash_ids(&dir)[0].clone();
    write(&dir, "a.txt", "local\n");
    write(&dir, "b.txt", "staged\n");
    git(&dir, &["add", "b.txt"]).unwrap();
    write(&dir, "new.txt", "untracked\n");

    let swap = repo.apply_stash(0, false).unwrap().expect("a.txt clashes");
    assert_eq!(swap.overlap, ["a.txt"]);
    assert_eq!(read(&dir, "a.txt"), "stashed\n");
    assert_eq!(read(&dir, "b.txt"), "base\n", "all changes were set aside, not just the clashing file");
    assert!(!dir.join("new.txt").exists());
    let list = stash_list(&dir);
    assert_eq!(list.len(), 2);
    assert!(list[0].ends_with("Changes saved before applying a stash"), "{list:?}");
    assert_eq!(stash_ids(&dir)[1], wanted, "apply keeps the applied stash");

    let entry = UndoEntry::stash_swap("Applying stash", swap);
    repo.undo(&entry).unwrap();
    assert_eq!(read(&dir, "a.txt"), "local\n");
    assert_eq!(read(&dir, "new.txt"), "untracked\n");
    assert_eq!(git(&dir, &["diff", "--cached", "--name-only"]).unwrap().trim(), "b.txt", "staging is restored too");
    assert_eq!(stash_ids(&dir), [wanted.as_str()]);

    repo.redo(&entry).unwrap();
    assert_eq!(read(&dir, "a.txt"), "stashed\n");
    assert!(!dir.join("new.txt").exists());
    assert_eq!(stash_ids(&dir).len(), 2);
    repo.undo(&entry).unwrap();
    assert_eq!(read(&dir, "a.txt"), "local\n");
}

#[test]
fn clash_on_pop_and_undo_puts_the_stash_back_in_place() {
    let (dir, repo) = repo("pop");
    stash(&dir, "a.txt", "wanted\n", "wanted");
    stash(&dir, "b.txt", "newer\n", "newer");
    let before = stash_ids(&dir);
    // An untracked file the stash would create counts as a clash too.
    stash(&dir, "c.txt", "from stash\n", "untracked");
    let before = [stash_ids(&dir)[0].clone()].into_iter().chain(before).collect::<Vec<_>>();
    write(&dir, "c.txt", "local\n");

    let swap = repo.apply_stash(0, true).unwrap().expect("c.txt clashes");
    assert_eq!(swap.overlap, ["c.txt"]);
    assert_eq!(read(&dir, "c.txt"), "from stash\n");
    assert_eq!(stash_list(&dir).len(), 3, "one popped, one saved");

    let entry = UndoEntry::stash_swap("Popping stash", swap);
    repo.undo(&entry).unwrap();
    assert_eq!(read(&dir, "c.txt"), "local\n");
    assert_eq!(stash_ids(&dir), before);

    // Pop from the middle of the list.
    git(&dir, &["stash", "drop", "-q", "stash@{0}"]).unwrap();
    write(&dir, "a.txt", "local\n");
    let swap = repo.apply_stash(1, true).unwrap().expect("a.txt clashes");
    assert_eq!(read(&dir, "a.txt"), "wanted\n");
    let entry = UndoEntry::stash_swap("Popping stash", swap);
    repo.undo(&entry).unwrap();
    assert_eq!(read(&dir, "a.txt"), "local\n");
    assert_eq!(stash_ids(&dir), before[1..]);
    assert_eq!(stash_list(&dir), ["On main: newer", "On main: wanted"]);
}

#[test]
fn undo_refuses_after_further_edits() {
    let (dir, repo) = repo("edited");
    stash(&dir, "a.txt", "stashed\n", "wanted");
    write(&dir, "a.txt", "local\n");
    let swap = repo.apply_stash(0, false).unwrap().unwrap();
    write(&dir, "a.txt", "edited after\n");
    assert!(repo.undo(&UndoEntry::stash_swap("Applying stash", swap)).is_err());
    assert_eq!(read(&dir, "a.txt"), "edited after\n");
}

#[test]
fn applying_on_a_clean_tree_is_plain_apply() {
    let (dir, repo) = repo("clean");
    stash(&dir, "a.txt", "stashed\n", "wanted");
    assert!(repo.apply_stash(0, true).unwrap().is_none());
    assert_eq!(read(&dir, "a.txt"), "stashed\n");
    assert!(stash_list(&dir).is_empty());
}

#[test]
fn a_conflicting_pop_says_why() {
    let (dir, repo) = repo("conflict-reason");
    git(&dir, &["checkout", "-qb", "other"]).unwrap();
    git(&dir, &["rm", "-q", "a.txt"]).unwrap();
    git(&dir, &["commit", "-qm", "drop a"]).unwrap();
    git(&dir, &["checkout", "-q", "main"]).unwrap();
    stash(&dir, "a.txt", "stashed\n", "wanted");
    git(&dir, &["checkout", "-q", "other"]).unwrap();
    // git reports the conflict on stdout only; the error must still say it.
    let err = format!("{:#}", repo.apply_stash(0, true).unwrap_err());
    assert!(err.contains("CONFLICT"), "{err}");
    assert_eq!(stash_list(&dir), ["On main: wanted"]);
}
