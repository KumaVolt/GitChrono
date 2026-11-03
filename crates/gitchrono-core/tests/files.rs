//! Single-file actions from the file menus, against a real `git`.

use std::path::PathBuf;

use gitchrono_core::{FileChange, FileStatus, Repo, git, ignore_choices};

fn repo(name: &str) -> (PathBuf, Repo) {
    let dir = std::env::temp_dir().join(format!("gitchrono-files-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("docs")).unwrap();
    for args in [
        &["init", "-q", "-b", "main"][..],
        &["config", "user.name", "Test"],
        &["config", "user.email", "test@example.com"],
        &["config", "commit.gpgsign", "false"],
        &["config", "core.autocrlf", "false"],
    ] {
        git(&dir, args).unwrap();
    }
    std::fs::write(dir.join("a.txt"), "one\ntwo\n").unwrap();
    std::fs::write(dir.join("b.txt"), "b\n").unwrap();
    git(&dir, &["add", "."]).unwrap();
    git(&dir, &["commit", "-qm", "base"]).unwrap();
    let repo = Repo::open(&dir).unwrap();
    (dir, repo)
}

fn untracked(repo: &Repo) -> Vec<String> {
    repo.status().unwrap().unstaged.into_iter().filter(|f| f.status == FileStatus::Untracked).map(|f| f.path).collect()
}

fn change(path: &str, status: FileStatus) -> FileChange {
    FileChange { path: path.into(), old_path: None, status }
}

#[test]
fn each_ignore_choice_hides_what_it_says() {
    let (dir, repo) = repo("ignore");
    std::fs::write(dir.join("docs/SKILL.md"), "x").unwrap();
    std::fs::write(dir.join("docs/other.md"), "x").unwrap();
    std::fs::write(dir.join("top.md"), "x").unwrap();
    std::fs::write(dir.join("docs/keep.txt"), "x").unwrap();

    let choices = ignore_choices("docs/SKILL.md");
    repo.ignore(&choices[0].pattern).unwrap();
    assert_eq!(untracked(&repo), [".gitignore", "docs/keep.txt", "docs/other.md", "top.md"], "just that file");

    repo.ignore(&choices[1].pattern).unwrap();
    assert_eq!(untracked(&repo), [".gitignore", "docs/keep.txt"], "every .md, here and at the top");

    repo.ignore(&choices[2].pattern).unwrap();
    assert_eq!(untracked(&repo), [".gitignore"]);

    repo.ignore(&choices[2].pattern).unwrap();
    assert_eq!(std::fs::read_to_string(dir.join(".gitignore")).unwrap(), "/docs/SKILL.md\n*.md\n/docs/\n", "no repeats");
}

#[test]
fn ignoring_keeps_an_existing_gitignore_intact() {
    let (dir, repo) = repo("ignore-append");
    std::fs::write(dir.join(".gitignore"), "target").unwrap();
    repo.ignore("*.log").unwrap();
    assert_eq!(std::fs::read_to_string(dir.join(".gitignore")).unwrap(), "target\n*.log\n");
}

#[test]
fn stashing_one_file_leaves_the_others() {
    let (dir, repo) = repo("stash");
    std::fs::write(dir.join("a.txt"), "changed\n").unwrap();
    std::fs::write(dir.join("b.txt"), "changed too\n").unwrap();
    std::fs::write(dir.join("new.txt"), "new\n").unwrap();

    repo.stash_files(&["a.txt", "new.txt"]).unwrap();
    assert_eq!(std::fs::read_to_string(dir.join("a.txt")).unwrap(), "one\ntwo\n");
    assert!(!dir.join("new.txt").exists(), "untracked files go too");
    assert_eq!(std::fs::read_to_string(dir.join("b.txt")).unwrap(), "changed too\n");
    assert_eq!(repo.stashes().unwrap()[0].message, "On main: 2 files");

    repo.stash_pop(0).unwrap();
    assert_eq!(std::fs::read_to_string(dir.join("a.txt")).unwrap(), "changed\n");
    assert!(dir.join("new.txt").exists());
}

#[test]
fn a_file_patch_applies_elsewhere() {
    let (dir, repo) = repo("patch");
    std::fs::write(dir.join("a.txt"), "one\nTWO\n").unwrap();
    std::fs::write(dir.join("b.txt"), "b staged\n").unwrap();
    git(&dir, &["add", "b.txt"]).unwrap();
    std::fs::write(dir.join("new.txt"), "brand new\n").unwrap();

    let unstaged = repo.file_patch(&change("a.txt", FileStatus::Modified), false).unwrap();
    let staged = repo.file_patch(&change("b.txt", FileStatus::Modified), true).unwrap();
    let added = repo.file_patch(&change("new.txt", FileStatus::Untracked), false).unwrap();
    assert!(unstaged.contains("+TWO") && !unstaged.contains("b.txt"), "{unstaged}");
    assert!(staged.contains("+b staged"), "{staged}");
    assert!(added.contains("new file mode") && added.contains("+brand new"), "{added}");

    git(&dir, &["reset", "-q", "--hard"]).unwrap();
    std::fs::remove_file(dir.join("new.txt")).unwrap();
    // As saved to disk and applied by hand.
    for (i, patch) in [&unstaged, &staged, &added].into_iter().enumerate() {
        let file = dir.with_extension(format!("{i}.patch"));
        std::fs::write(&file, patch).unwrap();
        git(&dir, &["apply", file.to_str().unwrap()]).unwrap();
    }
    let read = |p: &str| std::fs::read_to_string(dir.join(p)).unwrap();
    assert_eq!((read("a.txt"), read("b.txt"), read("new.txt")), ("one\nTWO\n".into(), "b staged\n".into(), "brand new\n".into()));
}

#[test]
fn ignoring_a_tracked_file_needs_stop_tracking_too() {
    let (dir, repo) = repo("stop-tracking");
    std::fs::write(dir.join("a.txt"), "changed\n").unwrap();
    let choice = &ignore_choices("a.txt")[0];

    repo.ignore(&choice.pattern).unwrap();
    let modified = |repo: &Repo| repo.status().unwrap().unstaged.iter().any(|f| f.path == "a.txt");
    assert!(modified(&repo), "a tracked file's changes still show once it's in .gitignore");

    repo.stop_tracking("a.txt").unwrap();
    let status = repo.status().unwrap();
    assert!(!modified(&repo));
    assert!(status.staged.iter().any(|f| f.path == "a.txt" && f.status == FileStatus::Deleted), "removed from the repository at the next commit");
    assert!(!untracked(&repo).contains(&"a.txt".to_string()), "and ignored, not untracked");
    assert_eq!(std::fs::read_to_string(dir.join("a.txt")).unwrap(), "changed\n", "still on disk");
}

#[test]
fn stop_tracking_refuses_to_lose_staged_changes() {
    let (dir, repo) = repo("stop-tracking-staged");
    std::fs::write(dir.join("a.txt"), "staged\n").unwrap();
    git(&dir, &["add", "a.txt"]).unwrap();
    std::fs::write(dir.join("a.txt"), "edited again\n").unwrap();

    assert!(repo.stop_tracking("a.txt").is_err(), "the staged version is in neither the commit nor the file");
    assert!(repo.status().unwrap().staged.iter().any(|f| f.path == "a.txt"));
}
