//! Pushing tags to a remote.

use gitchrono_core::{Repo, git};

#[test]
fn pushes_a_tag_to_a_remote_without_forcing_it() {
    let base = std::env::temp_dir().join(format!("gitchrono-tags-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    std::fs::create_dir_all(&base).unwrap();
    git(&base, &["init", "-q", "--bare", "-b", "main", "origin.git"]).unwrap();
    git(&base, &["clone", "-q", "origin.git", "work"]).unwrap();
    let work = base.join("work");
    for args in [&["config", "user.name", "T"][..], &["config", "user.email", "t@example.com"], &["config", "commit.gpgsign", "false"]] {
        git(&work, args).unwrap();
    }
    for n in ["1", "2"] {
        std::fs::write(work.join("f"), n).unwrap();
        git(&work, &["add", "."]).unwrap();
        git(&work, &["commit", "-qm", n]).unwrap();
    }
    git(&work, &["tag", "v1", "HEAD~1"]).unwrap();
    let origin = base.join("origin.git");
    let remote_tag = || git(&origin, &["rev-parse", "--verify", "--quiet", "refs/tags/v1"]).ok();

    let repo = Repo::open(&work).unwrap();
    assert_eq!(repo.snapshot(100).unwrap().refs.remotes, ["origin"]);
    repo.push_tag("origin", "v1").unwrap();
    assert_eq!(remote_tag(), Some(git(&work, &["rev-parse", "HEAD~1"]).unwrap()));
    // Already there: git has nothing to do, which isn't an error.
    repo.push_tag("origin", "v1").unwrap();

    // The same tag on another commit is refused, and the remote keeps its own.
    git(&work, &["tag", "-f", "v1", "HEAD"]).unwrap();
    let err = repo.push_tag("origin", "v1").unwrap_err();
    assert!(format!("{err:#}").contains("origin already has a tag v1 on a different commit"), "{err:#}");
    assert_eq!(remote_tag(), Some(git(&work, &["rev-parse", "HEAD~1"]).unwrap()));

    assert!(repo.push_tag("nowhere", "v1").is_err());
    let _ = std::fs::remove_dir_all(&base);
}
