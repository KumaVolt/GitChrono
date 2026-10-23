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
