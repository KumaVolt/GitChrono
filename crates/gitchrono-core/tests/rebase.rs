//! Interactive rebase plans applied by a real `git`.

use gitchrono_core::{RebaseAction, Repo, git};

fn log(dir: &std::path::Path) -> Vec<String> {
    git(dir, &["log", "--format=%s", "--reverse"]).unwrap().lines().map(str::to_string).collect()
}

#[test]
fn reorder_reword_squash_drop() {
    let dir = std::env::temp_dir().join(format!("gitchrono-rebase-test-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    for args in [
        &["init", "-q", "-b", "main"][..],
        &["config", "user.name", "T"],
        &["config", "user.email", "t@example.com"],
        &["config", "commit.gpgsign", "false"],
        &["config", "core.autocrlf", "false"],
    ] {
        git(&dir, args).unwrap();
    }
    for (file, msg) in [("base", "base"), ("a", "add a"), ("b", "add b"), ("c", "add c"), ("d", "add d")] {
        std::fs::write(dir.join(file), msg).unwrap();
        git(&dir, &["add", "."]).unwrap();
        git(&dir, &["commit", "-qm", msg]).unwrap();
    }
    let repo = Repo::open(&dir).unwrap();
    let base = git(&dir, &["rev-parse", "HEAD~4"]).unwrap().trim().to_string();
    let mut plan = repo.rebase_plan(&base).unwrap();
    assert_eq!(plan.iter().map(|s| s.summary.as_str()).collect::<Vec<_>>(), ["add a", "add b", "add c", "add d"]);

    // d first (reworded), then a, b fixed up into a, c dropped.
    plan.swap(0, 3); // d b c a
    plan.swap(1, 3); // d a c b
    plan.swap(2, 3); // d a b c
    plan[0].action = RebaseAction::Reword;
    plan[0].message = "Add d first\n\nWith a body.".into();
    plan[2].action = RebaseAction::Fixup;
    plan[3].action = RebaseAction::Drop;
    repo.interactive_rebase(&base, &plan).unwrap();

    assert_eq!(log(&dir), ["base", "Add d first", "add a"]);
    assert_eq!(git(&dir, &["log", "-1", "--format=%b", "HEAD~1"]).unwrap().trim(), "With a body.");
    assert!(dir.join("b").exists(), "fixup keeps the change");
    assert!(!dir.join("c").exists(), "drop removes the change");

    let squash_first = vec![gitchrono_core::RebaseStep { action: RebaseAction::Squash, ..repo.rebase_plan(&base).unwrap()[0].clone() }];
    assert!(repo.interactive_rebase(&base, &squash_first).is_err());
}

#[test]
fn resolve_a_merge_conflict() {
    use gitchrono_core::Resolution;
    let dir = std::env::temp_dir().join(format!("gitchrono-conflict-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    for args in [
        &["init", "-q", "-b", "main"][..],
        &["config", "user.name", "T"],
        &["config", "user.email", "t@example.com"],
        &["config", "commit.gpgsign", "false"],
        &["config", "core.autocrlf", "false"],
    ] {
        git(&dir, args).unwrap();
    }
    std::fs::write(dir.join("f.txt"), "top\nmiddle\nbottom\n").unwrap();
    git(&dir, &["add", "."]).unwrap();
    git(&dir, &["commit", "-qm", "base"]).unwrap();
    git(&dir, &["checkout", "-qb", "feature"]).unwrap();
    std::fs::write(dir.join("f.txt"), "top\nfeature middle\nbottom\n").unwrap();
    git(&dir, &["commit", "-qam", "feature"]).unwrap();
    git(&dir, &["checkout", "-q", "main"]).unwrap();
    std::fs::write(dir.join("f.txt"), "top\nmain middle\nbottom\n").unwrap();
    git(&dir, &["commit", "-qam", "main"]).unwrap();
    let repo = Repo::open(&dir).unwrap();
    assert!(repo.merge("feature").is_err(), "merge conflicts");

    let file = repo.conflict_file("f.txt").unwrap();
    assert_eq!(file.conflict_count(), 1);
    assert!(!repo.save_resolution(&file, &[None]).unwrap(), "still unresolved");
    let file = repo.conflict_file("f.txt").unwrap();
    assert!(repo.save_resolution(&file, &[Some(Resolution::TheirsThenOurs)]).unwrap());
    assert_eq!(std::fs::read_to_string(dir.join("f.txt")).unwrap(), "top\nfeature middle\nmain middle\nbottom\n");
    assert!(repo.status().unwrap().conflicted.is_empty());
}

fn reword_repo(name: &str) -> (std::path::PathBuf, Repo) {
    let dir = std::env::temp_dir().join(format!("gitchrono-reword-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    for args in [
        &["init", "-q", "-b", "main"][..],
        &["config", "user.name", "T"],
        &["config", "user.email", "t@example.com"],
        &["config", "commit.gpgsign", "false"],
        &["config", "core.autocrlf", "false"],
    ] {
        git(&dir, args).unwrap();
    }
    for (file, msg) in [("a", "add a"), ("b", "add b"), ("c", "add c")] {
        std::fs::write(dir.join(file), msg).unwrap();
        git(&dir, &["add", "."]).unwrap();
        git(&dir, &["commit", "-qm", msg]).unwrap();
    }
    let repo = Repo::open(&dir).unwrap();
    (dir, repo)
}

#[test]
fn reword_head_amends_the_message_only() {
    let (dir, repo) = reword_repo("head");
    let tree = git(&dir, &["rev-parse", "HEAD^{tree}"]).unwrap();
    std::fs::write(dir.join("staged"), "x").unwrap();
    git(&dir, &["add", "staged"]).unwrap();

    repo.reword("HEAD", "Add c\n\nNow with a body.").unwrap();
    assert_eq!(log(&dir), ["add a", "add b", "Add c"]);
    assert_eq!(git(&dir, &["log", "-1", "--format=%b"]).unwrap().trim(), "Now with a body.");
    assert_eq!(git(&dir, &["rev-parse", "HEAD^{tree}"]).unwrap(), tree, "staged changes aren't committed");
    assert_eq!(git(&dir, &["diff", "--cached", "--name-only"]).unwrap().trim(), "staged");
    assert!(repo.reword("HEAD", "  \n").is_err(), "empty message");
}

#[test]
fn reword_older_and_root_commits() {
    let (dir, repo) = reword_repo("older");
    std::fs::write(dir.join("a"), "dirty").unwrap();
    let middle = git(&dir, &["rev-parse", "HEAD~1"]).unwrap().trim().to_string();
    let reworded = repo.reword(&middle, "Add b, reworded").unwrap();
    assert_eq!(git(&dir, &["rev-parse", "HEAD~1"]).unwrap().trim(), reworded);
    assert_eq!(log(&dir), ["add a", "Add b, reworded", "add c"]);
    assert_eq!(std::fs::read_to_string(dir.join("a")).unwrap(), "dirty", "local changes survive");

    let root = git(&dir, &["rev-list", "--max-parents=0", "HEAD"]).unwrap().trim().to_string();
    repo.reword(&root, "Add a first").unwrap();
    assert_eq!(log(&dir), ["Add a first", "Add b, reworded", "add c"]);
}

#[test]
fn reword_refuses_merges_and_other_branches() {
    let (dir, repo) = reword_repo("merge");
    git(&dir, &["checkout", "-qb", "side", "HEAD~1"]).unwrap();
    std::fs::write(dir.join("side"), "s").unwrap();
    git(&dir, &["add", "."]).unwrap();
    git(&dir, &["commit", "-qm", "side"]).unwrap();
    let side = git(&dir, &["rev-parse", "HEAD"]).unwrap().trim().to_string();
    git(&dir, &["checkout", "-q", "main"]).unwrap();
    git(&dir, &["merge", "-q", "--no-ff", "-m", "merge side", "side"]).unwrap();

    let err = repo.reword("HEAD", "renamed merge").unwrap_err().to_string();
    assert!(err.contains("merge commit"), "{err}");
    let err = repo.reword("HEAD~1", "below a merge").unwrap_err().to_string();
    assert!(err.contains("merges"), "{err}");
    git(&dir, &["reset", "-q", "--hard", "HEAD~1"]).unwrap();
    let err = repo.reword(&side, "elsewhere").unwrap_err().to_string();
    assert!(err.contains("current branch"), "{err}");
}

#[test]
fn pushed_commits_are_detected() {
    let (dir, repo) = reword_repo("pushed");
    let head = git(&dir, &["rev-parse", "HEAD"]).unwrap().trim().to_string();
    assert!(!repo.is_pushed(&head).unwrap());
    git(&dir, &["update-ref", "refs/remotes/origin/main", "HEAD~1"]).unwrap();
    assert!(repo.is_pushed("HEAD~1").unwrap());
    assert!(!repo.is_pushed(&head).unwrap());
}
