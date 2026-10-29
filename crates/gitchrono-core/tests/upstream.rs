//! Pulling a branch with no upstream: each case says what would fix it, and
//! the fixes work.

use std::path::{Path, PathBuf};

use gitchrono_core::{GitError, NoUpstream, PullMode, Repo, git};

fn base(name: &str) -> PathBuf {
    let base = std::env::temp_dir().join(format!("gitchrono-upstream-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    std::fs::create_dir_all(&base).unwrap();
    base
}

/// A repository with one commit on `main` and no remotes.
fn work(base: &Path) -> PathBuf {
    let dir = base.join("work");
    git(base, &["init", "-q", "-b", "main", "work"]).unwrap();
    for args in [&["config", "user.name", "T"][..], &["config", "user.email", "t@example.com"], &["config", "commit.gpgsign", "false"]] {
        git(&dir, args).unwrap();
    }
    std::fs::write(dir.join("f"), "1").unwrap();
    git(&dir, &["add", "."]).unwrap();
    git(&dir, &["commit", "-qm", "one"]).unwrap();
    dir
}

fn bare(base: &Path, name: &str) -> String {
    git(base, &["init", "-q", "--bare", "-b", "main", name]).unwrap();
    base.join(name).to_str().unwrap().to_string()
}

fn no_upstream(result: anyhow::Result<()>) -> NoUpstream {
    result.unwrap_err().downcast::<NoUpstream>().expect("a NoUpstream error")
}

#[test]
fn no_remote_at_all() {
    let base = base("none");
    let dir = work(&base);
    let repo = Repo::open(&dir).unwrap();
    assert_eq!(no_upstream(repo.pull(PullMode::Config)), NoUpstream::NoRemote);
    assert_eq!(no_upstream(repo.push(false)), NoUpstream::NoRemote, "push explains it too, rather than naming origin");
    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn detached_head() {
    let base = base("detached");
    let dir = work(&base);
    git(&dir, &["checkout", "-q", "--detach"]).unwrap();
    assert_eq!(no_upstream(Repo::open(&dir).unwrap().pull(PullMode::Config)), NoUpstream::Detached);
    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn remote_without_the_branch_offers_publishing() {
    let base = base("publish");
    let dir = work(&base);
    let url = bare(&base, "origin.git");
    git(&dir, &["remote", "add", "origin", &url]).unwrap();
    let repo = Repo::open(&dir).unwrap();
    let why = no_upstream(repo.pull(PullMode::Config));
    assert_eq!(why, NoUpstream::Publish { branch: "main".into(), remotes: vec!["origin".into()] });
    assert!(why.to_string().contains("isn't on origin yet"), "{why}");

    repo.publish("origin").unwrap();
    assert_eq!(repo.status().unwrap().upstream.as_deref(), Some("origin/main"));
    repo.pull(PullMode::Config).unwrap();
    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn same_named_remote_branch_offers_tracking_even_before_a_fetch() {
    let base = base("track");
    let dir = work(&base);
    let url = bare(&base, "origin.git");
    git(&dir, &["push", "-q", &url, "main"]).unwrap();
    // The remote gets a commit we don't have, and we've never fetched from it.
    git(&base, &["clone", "-q", &url, "other"]).unwrap();
    let other = base.join("other");
    for args in [&["config", "user.name", "T"][..], &["config", "user.email", "t@example.com"], &["config", "commit.gpgsign", "false"]] {
        git(&other, args).unwrap();
    }
    std::fs::write(other.join("f"), "2").unwrap();
    git(&other, &["commit", "-qam", "two"]).unwrap();
    git(&other, &["push", "-q"]).unwrap();
    git(&dir, &["remote", "add", "origin", &url]).unwrap();

    let repo = Repo::open(&dir).unwrap();
    let why = no_upstream(repo.pull(PullMode::Config));
    assert_eq!(why, NoUpstream::Track { branch: "main".into(), candidates: vec!["origin/main".into()] });

    repo.track_and_pull("origin/main", PullMode::Config).unwrap();
    let status = repo.status().unwrap();
    assert_eq!((status.upstream.as_deref(), status.behind), (Some("origin/main"), 0));
    assert_eq!(std::fs::read_to_string(dir.join("f")).unwrap(), "2", "pulled the remote's commit");
    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn several_remotes_put_origin_first() {
    let base = base("several");
    let dir = work(&base);
    let (a, b) = (bare(&base, "a.git"), bare(&base, "b.git"));
    git(&dir, &["remote", "add", "upstream", &a]).unwrap();
    git(&dir, &["remote", "add", "origin", &b]).unwrap();
    let repo = Repo::open(&dir).unwrap();
    assert_eq!(
        no_upstream(repo.pull(PullMode::Config)),
        NoUpstream::Publish { branch: "main".into(), remotes: vec!["origin".into(), "upstream".into()] }
    );

    git(&dir, &["push", "-q", "upstream", "main"]).unwrap();
    git(&dir, &["push", "-q", "origin", "main"]).unwrap();
    assert_eq!(
        no_upstream(repo.pull(PullMode::Config)),
        NoUpstream::Track { branch: "main".into(), candidates: vec!["origin/main".into(), "upstream/main".into()] }
    );

    // remote.pushDefault outranks origin.
    git(&dir, &["config", "remote.pushDefault", "upstream"]).unwrap();
    let NoUpstream::Track { candidates, .. } = no_upstream(repo.pull(PullMode::Config)) else { panic!("expected Track") };
    assert_eq!(candidates[0], "upstream/main");
    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn gits_own_no_tracking_error_is_recognised() {
    let base = base("raw");
    let dir = work(&base);
    let err = git(&dir, &["pull", "--no-rebase"]).unwrap_err();
    assert!(err.downcast_ref::<GitError>().unwrap().is_no_upstream());
    let err = git(&dir, &["push", "nowhere"]).unwrap_err();
    assert!(!err.downcast_ref::<GitError>().unwrap().is_no_upstream());
    let _ = std::fs::remove_dir_all(&base);
}
