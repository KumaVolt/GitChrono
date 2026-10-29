//! Creating a repository with `git init`, and reading it before the first commit.

use gitchrono_core::{Repo, git};

#[test]
fn init_creates_an_empty_repository_that_reads_cleanly() {
    let dir = std::env::temp_dir().join(format!("gitchrono-init-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let path = dir.join("new-project");

    let repo = Repo::init(&path).unwrap();
    assert!(path.join(".git").is_dir(), "creates the folder and the repository");
    let branch = git(&path, &["symbolic-ref", "--short", "HEAD"]).unwrap();
    let expected = git(&path, &["config", "--get", "init.defaultBranch"]).unwrap_or_else(|_| "main".into());
    assert_eq!(branch.trim(), expected.trim());

    // No commits yet: every read the main view does still works.
    let snap = repo.snapshot(100).unwrap();
    assert!(snap.commits.is_empty());
    assert!(snap.refs.local.is_empty());
    assert_eq!(snap.refs.head_id, None);
    assert_eq!(snap.status.branch.as_deref(), Some(expected.trim()));
    assert!(snap.status.is_clean());

    // The first commit goes through the normal staging path.
    for args in [&["config", "user.name", "Ada"][..], &["config", "user.email", "ada@example.com"], &["config", "commit.gpgsign", "false"]] {
        git(&path, args).unwrap();
    }
    std::fs::write(path.join("README.md"), "hello\n").unwrap();
    let status = repo.status().unwrap();
    assert_eq!(status.unstaged.len(), 1);
    std::fs::create_dir_all(path.join("src")).unwrap();
    std::fs::write(path.join("src/main.rs"), "fn main() {}\n").unwrap();
    // Unstaging works without a HEAD to restore from, and keeps the files.
    repo.stage_all().unwrap();
    repo.unstage(&["src/main.rs"]).unwrap();
    let status = repo.status().unwrap();
    assert_eq!((status.staged.len(), status.unstaged.len()), (1, 1));
    repo.unstage_all().unwrap();
    assert!(repo.status().unwrap().staged.is_empty());
    assert!(path.join("src/main.rs").exists());
    std::fs::remove_dir_all(path.join("src")).unwrap();

    repo.stage_all().unwrap();
    assert!(!repo.staged_patch().unwrap().is_empty());
    repo.commit("first", false).unwrap();
    assert_eq!(repo.snapshot(100).unwrap().commits.len(), 1);

    // Running it again on an existing repository leaves it alone.
    let again = Repo::init(&path).unwrap();
    assert_eq!(again.snapshot(100).unwrap().commits.len(), 1);

    let _ = std::fs::remove_dir_all(&dir);
}
