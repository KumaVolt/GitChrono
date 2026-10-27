//! Checking out remote branches when a local branch of the same name exists.

use gitchrono_core::{Repo, git};

fn commit(dir: &std::path::Path, msg: &str) {
    std::fs::write(dir.join("f"), msg).unwrap();
    git(dir, &["commit", "-qam", msg]).unwrap();
}

#[test]
fn remote_checkout_reuses_and_fast_forwards_the_local_branch() {
    let base = std::env::temp_dir().join(format!("gitchrono-co-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    let (origin, work) = (base.join("origin"), base.join("work"));
    std::fs::create_dir_all(&origin).unwrap();
    for args in [&["init", "-q", "-b", "master"][..], &["config", "user.name", "T"], &["config", "user.email", "t@example.com"], &["config", "commit.gpgsign", "false"]] {
        git(&origin, args).unwrap();
    }
    std::fs::write(origin.join("f"), "1").unwrap();
    git(&origin, &["add", "."]).unwrap();
    git(&origin, &["commit", "-qm", "one"]).unwrap();
    git(&base, &["clone", "-q", origin.to_str().unwrap(), "work"]).unwrap();
    git(&work, &["checkout", "-qb", "other"]).unwrap();
    commit(&origin, "two");
    git(&work, &["fetch", "-q"]).unwrap();

    let repo = Repo::open(&work).unwrap();
    repo.checkout_remote("origin/master").unwrap();
    let head = git(&work, &["rev-parse", "--abbrev-ref", "HEAD"]).unwrap();
    assert_eq!(head.trim(), "master", "switched to the existing local branch");
    assert_eq!(git(&work, &["rev-parse", "master"]).unwrap(), git(&work, &["rev-parse", "origin/master"]).unwrap(), "fast-forwarded");

    // No local branch yet: a tracking branch is created.
    git(&origin, &["checkout", "-qb", "feature"]).unwrap();
    git(&work, &["fetch", "-q"]).unwrap();
    repo.checkout_remote("origin/feature").unwrap();
    assert_eq!(git(&work, &["rev-parse", "--abbrev-ref", "HEAD"]).unwrap().trim(), "feature");
    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn push_rejection_is_recognised_and_pull_then_push_fixes_it() {
    let base = std::env::temp_dir().join(format!("gitchrono-push-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    std::fs::create_dir_all(&base).unwrap();
    git(&base, &["init", "-q", "--bare", "-b", "main", "origin.git"]).unwrap();
    for name in ["a", "b"] {
        git(&base, &["clone", "-q", "origin.git", name]).unwrap();
        let dir = base.join(name);
        for args in [&["config", "user.name", "T"][..], &["config", "user.email", "t@example.com"], &["config", "commit.gpgsign", "false"]] {
            git(&dir, args).unwrap();
        }
    }
    let (a, b) = (base.join("a"), base.join("b"));
    std::fs::write(a.join("f"), "1").unwrap();
    git(&a, &["add", "."]).unwrap();
    git(&a, &["commit", "-qm", "one"]).unwrap();
    git(&a, &["push", "-q", "-u", "origin", "main"]).unwrap();
    git(&b, &["pull", "-q", "origin", "main"]).unwrap();
    git(&b, &["branch", "-q", "-u", "origin/main"]).unwrap();
    commit(&a, "two");
    git(&a, &["push", "-q"]).unwrap();
    std::fs::write(b.join("g"), "x").unwrap();
    git(&b, &["add", "."]).unwrap();
    git(&b, &["commit", "-qm", "local"]).unwrap();

    let repo = Repo::open(&b).unwrap();
    let err = repo.push(false).unwrap_err();
    assert!(err.downcast_ref::<gitchrono_core::GitError>().is_some_and(|e| e.is_push_rejected()), "{err:#}");
    repo.pull_then_push().unwrap();
    assert_eq!(git(&b, &["rev-parse", "HEAD"]).unwrap(), git(&b, &["rev-parse", "origin/main"]).unwrap());
    let _ = std::fs::remove_dir_all(&base);
}
