//! Cloning into a new folder next to other repositories.

use gitchrono_core::{Repo, clone_repo, git, repo_name_from_url};

#[test]
fn clones_a_bare_repository_and_reports_progress() {
    let base = std::env::temp_dir().join(format!("gitchrono-clone-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    std::fs::create_dir_all(&base).unwrap();
    git(&base, &["init", "-q", "--bare", "-b", "main", "origin.git"]).unwrap();
    git(&base, &["clone", "-q", "origin.git", "seed"]).unwrap();
    let seed = base.join("seed");
    for args in [&["config", "user.name", "T"][..], &["config", "user.email", "t@example.com"], &["config", "commit.gpgsign", "false"]] {
        git(&seed, args).unwrap();
    }
    std::fs::write(seed.join("f"), "1").unwrap();
    git(&seed, &["add", "."]).unwrap();
    git(&seed, &["commit", "-qm", "one"]).unwrap();
    git(&seed, &["push", "-q", "origin", "main"]).unwrap();

    let url = format!("file://{}", base.join("origin.git").display());
    let name = repo_name_from_url(&url).unwrap();
    assert_eq!(name, "origin");
    let dest = base.join("projects").join(&name);
    let mut lines = Vec::new();
    clone_repo(&url, &dest, |l| lines.push(l.to_string())).unwrap();
    assert!(!lines.is_empty(), "git's progress is passed on");

    let repo = Repo::open(&dest).unwrap();
    assert_eq!(std::fs::read_to_string(dest.join("f")).unwrap(), "1");
    assert_eq!(git(&dest, &["rev-parse", "HEAD"]).unwrap(), git(&seed, &["rev-parse", "HEAD"]).unwrap());
    assert_eq!(repo.status().unwrap().branch.as_deref(), Some("main"));

    // A folder that's already there isn't cloned over.
    let err = clone_repo(&url, &dest, |_| {}).unwrap_err();
    assert!(format!("{err:#}").contains("already exists"), "{err:#}");
    // Git's own failure comes back as a GitError with its message.
    let err = clone_repo(&format!("file://{}", base.join("missing.git").display()), &base.join("missing"), |_| {}).unwrap_err();
    let git_err = err.downcast_ref::<gitchrono_core::GitError>().expect("a GitError");
    assert!(!git_err.stderr.is_empty() && !git_err.stderr.contains("Cloning into"), "{}", git_err.stderr);
    let _ = std::fs::remove_dir_all(&base);
}
