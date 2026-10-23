//! End-to-end checks of hunk/line staging against a real `git`.

use std::collections::HashSet;
use std::path::PathBuf;
use std::process::Command;

use gitchrono_core::{ApplyMode, FileChange, FileStatus, Repo, git, partial_patch};

fn temp_repo(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("gitchrono-test-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    for args in [
        &["init", "-q", "-b", "main"][..],
        &["config", "user.name", "Test"],
        &["config", "user.email", "test@example.com"],
        &["config", "commit.gpgsign", "false"],
    ] {
        git(&dir, args).unwrap();
    }
    dir
}

fn write(dir: &PathBuf, lines: &[&str]) {
    std::fs::write(dir.join("f.txt"), lines.join("\n") + "\n").unwrap();
}

fn modified() -> FileChange {
    FileChange { path: "f.txt".into(), old_path: None, status: FileStatus::Modified }
}

fn setup(name: &str) -> (PathBuf, Repo) {
    let dir = temp_repo(name);
    let base: Vec<String> = (1..=20).map(|i| format!("line {i}")).collect();
    let base: Vec<&str> = base.iter().map(String::as_str).collect();
    write(&dir, &base);
    git(&dir, &["add", "."]).unwrap();
    git(&dir, &["commit", "-qm", "base"]).unwrap();
    // Two far-apart edits -> two hunks.
    let mut edited = base.clone();
    edited[1] = "line 2 changed";
    edited[17] = "line 18 changed";
    write(&dir, &edited);
    let repo = Repo::open(&dir).unwrap();
    (dir, repo)
}

fn cached(dir: &PathBuf) -> String {
    let out = Command::new("git").args(["diff", "--cached", "--no-color"]).current_dir(dir).output().unwrap();
    String::from_utf8(out.stdout).unwrap()
}

#[test]
fn stage_one_hunk() {
    let (dir, repo) = setup("hunk");
    let diff = repo.workdir_file_diff(&modified(), false).unwrap();
    assert_eq!(diff.hunks.len(), 2);
    let patch = partial_patch(&diff, &[1], None, ApplyMode::Stage).unwrap();
    repo.apply_patch(&patch, ApplyMode::Stage).unwrap();
    let staged = cached(&dir);
    assert!(staged.contains("+line 18 changed") && !staged.contains("+line 2 changed"), "{staged}");
}

#[test]
fn stage_only_the_added_line_then_unstage_it() {
    let (dir, repo) = setup("lines");
    let diff = repo.workdir_file_diff(&modified(), false).unwrap();
    let add = diff.hunks[0].lines.iter().position(|l| l.content == "line 2 changed").unwrap();
    let sel: HashSet<(usize, usize)> = [(0, add)].into();
    let patch = partial_patch(&diff, &[0], Some(&sel), ApplyMode::Stage).unwrap();
    repo.apply_patch(&patch, ApplyMode::Stage).unwrap();
    // Added without removing the old line: both survive in the index.
    let staged = cached(&dir);
    assert!(staged.contains("+line 2 changed") && !staged.contains("-line 2\n"), "{staged}");

    let diff = repo.workdir_file_diff(&modified(), true).unwrap();
    let patch = partial_patch(&diff, &[0], None, ApplyMode::Unstage).unwrap();
    repo.apply_patch(&patch, ApplyMode::Unstage).unwrap();
    assert_eq!(cached(&dir), "");
}

#[test]
fn discard_one_hunk() {
    let (dir, repo) = setup("discard");
    let diff = repo.workdir_file_diff(&modified(), false).unwrap();
    let patch = partial_patch(&diff, &[0], None, ApplyMode::Discard).unwrap();
    repo.apply_patch(&patch, ApplyMode::Discard).unwrap();
    let text = std::fs::read_to_string(dir.join("f.txt")).unwrap();
    assert!(text.contains("line 2\n") && text.contains("line 18 changed"), "{text}");
}
