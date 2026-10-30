//! Fetching and pulling many repositories, against a real `git`.

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use gitchrono_core::sync::{SyncAction, SyncOutcome, sync_repo, sync_repos};
use gitchrono_core::{Operation, PullMode, Repo, git};

const MERGE: SyncAction = SyncAction::Pull(PullMode::Merge);

fn base(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("gitchrono-sync-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn configure(dir: &Path) {
    for args in [&["config", "user.name", "Test"][..], &["config", "user.email", "test@example.com"], &["config", "commit.gpgsign", "false"], &["config", "core.autocrlf", "false"]] {
        git(dir, args).unwrap();
    }
}

fn commit(dir: &Path, file: &str, text: &str) {
    std::fs::write(dir.join(file), text).unwrap();
    git(dir, &["add", file]).unwrap();
    git(dir, &["commit", "-qm", file]).unwrap();
}

/// A repository `name` under `base` tracking a remote it's in step with.
fn clone(base: &Path, name: &str) -> PathBuf {
    let origin = base.join(format!("{name}.git"));
    git(base, &["init", "-q", "--bare", "-b", "main", origin.to_str().unwrap()]).unwrap();
    let work = base.join(name);
    git(base, &["clone", "-q", origin.to_str().unwrap(), work.to_str().unwrap()]).unwrap();
    configure(&work);
    commit(&work, "a.txt", "one\n");
    git(&work, &["push", "-q", "-u", "origin", "HEAD:main"]).unwrap();
    work
}

/// Someone else pushes `text` to `file`.
fn push_upstream(base: &Path, name: &str, file: &str, text: &str) {
    let other = base.join(format!("{name}-other"));
    if !other.exists() {
        git(base, &["clone", "-q", base.join(format!("{name}.git")).to_str().unwrap(), other.to_str().unwrap()]).unwrap();
        configure(&other);
    }
    git(&other, &["pull", "-q", "--no-rebase"]).unwrap();
    commit(&other, file, text);
    git(&other, &["push", "-q"]).unwrap();
}

fn tip(dir: &Path, rev: &str) -> String {
    git(dir, &["rev-parse", rev]).unwrap().trim().to_string()
}

fn skipped(outcome: SyncOutcome) -> String {
    match outcome {
        SyncOutcome::Skipped(why) => why,
        other => panic!("expected a skip, got {other:?}"),
    }
}

fn failed(outcome: SyncOutcome) -> String {
    match outcome {
        SyncOutcome::Failed(why) => why,
        other => panic!("expected a failure, got {other:?}"),
    }
}

#[test]
fn fetch_gets_remote_commits_without_touching_the_branch() {
    let base = base("fetch");
    let work = clone(&base, "repo");
    let before = tip(&work, "HEAD");
    push_upstream(&base, "repo", "b.txt", "two\n");

    assert_eq!(sync_repo(&work, SyncAction::Fetch), SyncOutcome::Done);
    assert_eq!(tip(&work, "HEAD"), before);
    assert_ne!(tip(&work, "origin/main"), before);
}

#[test]
fn pull_brings_the_branch_up_to_date() {
    let base = base("pull");
    let work = clone(&base, "repo");
    push_upstream(&base, "repo", "b.txt", "two\n");

    assert_eq!(sync_repo(&work, SyncAction::Pull(PullMode::FastForwardOnly)), SyncOutcome::Done);
    assert_eq!(tip(&work, "HEAD"), tip(&work, "origin/main"));
    assert!(work.join("b.txt").exists());

    assert_eq!(sync_repo(&work, SyncAction::Pull(PullMode::FastForwardOnly)), SyncOutcome::UpToDate);
}

#[test]
fn repositories_with_nothing_to_sync_are_skipped() {
    let base = base("skips");
    let lonely = base.join("lonely");
    std::fs::create_dir_all(&lonely).unwrap();
    git(&lonely, &["init", "-q", "-b", "main"]).unwrap();
    configure(&lonely);
    commit(&lonely, "a.txt", "one\n");
    assert_eq!(skipped(sync_repo(&lonely, SyncAction::Fetch)), "no remote");
    assert_eq!(skipped(sync_repo(&lonely, MERGE)), "no remote");

    let work = clone(&base, "repo");
    git(&work, &["switch", "-qc", "local-only"]).unwrap();
    assert_eq!(skipped(sync_repo(&work, MERGE)), "no upstream");

    git(&work, &["checkout", "-q", "--detach"]).unwrap();
    assert_eq!(skipped(sync_repo(&work, MERGE)), "not on a branch");
}

#[test]
fn a_repository_mid_merge_is_left_alone() {
    let base = base("mid-merge");
    let work = clone(&base, "repo");
    push_upstream(&base, "repo", "a.txt", "theirs\n");
    commit(&work, "a.txt", "ours\n");
    git(&work, &["fetch", "-q"]).unwrap();
    assert!(git(&work, &["merge", "--no-edit", "origin/main"]).is_err());

    let before = std::fs::read_to_string(work.join("a.txt")).unwrap();
    assert_eq!(skipped(sync_repo(&work, MERGE)), "an operation is in progress");
    assert_eq!(Repo::open(&work).unwrap().status().unwrap().operation, Operation::Merge, "still the user's to finish");
    assert_eq!(std::fs::read_to_string(work.join("a.txt")).unwrap(), before);
}

#[test]
fn a_conflicting_pull_is_undone() {
    for (name, mode) in [("merge", PullMode::Merge), ("rebase", PullMode::Rebase)] {
        let base = base(&format!("conflict-{name}"));
        let work = clone(&base, "repo");
        push_upstream(&base, "repo", "a.txt", "theirs\n");
        commit(&work, "a.txt", "ours\n");
        std::fs::write(work.join("notes.txt"), "uncommitted\n").unwrap();
        let before = tip(&work, "HEAD");

        assert_eq!(failed(sync_repo(&work, SyncAction::Pull(mode))), "conflicts, so the pull was undone", "{name}");
        let repo = Repo::open(&work).unwrap();
        assert_eq!(repo.status().unwrap().operation, Operation::None, "{name}");
        assert_eq!(tip(&work, "HEAD"), before, "{name}");
        assert_eq!(std::fs::read_to_string(work.join("a.txt")).unwrap(), "ours\n", "{name}");
        assert_eq!(std::fs::read_to_string(work.join("notes.txt")).unwrap(), "uncommitted\n", "{name}");
    }
}

#[test]
fn fast_forward_only_reports_diverged_branches() {
    let base = base("diverged");
    let work = clone(&base, "repo");
    push_upstream(&base, "repo", "b.txt", "theirs\n");
    commit(&work, "c.txt", "ours\n");
    let before = tip(&work, "HEAD");

    let why = failed(sync_repo(&work, SyncAction::Pull(PullMode::FastForwardOnly)));
    assert!(why.contains("fast-forward"), "{why}");
    assert_eq!(tip(&work, "HEAD"), before);
}

#[test]
fn a_failure_shows_gits_reason_not_its_hints() {
    let base = base("reason");
    let work = clone(&base, "repo");
    git(&work, &["remote", "set-url", "origin", base.join("missing.git").to_str().unwrap()]).unwrap();

    let why = failed(sync_repo(&work, SyncAction::Fetch));
    assert!(!why.starts_with("fatal:") && !why.contains('\n'), "{why}");
}

#[test]
fn every_repository_is_reported_once_and_a_failure_stops_nothing() {
    let base = base("many");
    let mut dirs: Vec<PathBuf> = (0..7).map(|i| clone(&base, &format!("repo{i}"))).collect();
    for i in 0..7 {
        push_upstream(&base, &format!("repo{i}"), "b.txt", "two\n");
    }
    dirs.insert(3, base.join("not-a-repository"));

    let seen = Mutex::new(Vec::new());
    sync_repos(&dirs, MERGE, |dir, outcome| seen.lock().unwrap().push((dir.to_path_buf(), outcome)));
    let seen = seen.into_inner().unwrap();

    assert_eq!(seen.len(), dirs.len());
    for dir in &dirs {
        let (_, outcome) = seen.iter().find(|(d, _)| d == dir).unwrap();
        if dir.exists() {
            assert_eq!(*outcome, SyncOutcome::Done, "{}", dir.display());
            assert!(dir.join("b.txt").exists());
        } else {
            assert!(matches!(outcome, SyncOutcome::Failed(_)), "{outcome:?}");
        }
    }
}
