//! File history (with renames) and blame against a real `git`.

use gitchrono_core::{Repo, git};

#[test]
fn history_follows_renames_and_blame_attributes_lines() {
    let dir = std::env::temp_dir().join(format!("gitchrono-history-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    for args in [
        &["init", "-q", "-b", "main"][..],
        &["config", "user.name", "Ada"],
        &["config", "user.email", "ada@example.com"],
        &["config", "commit.gpgsign", "false"],
        &["config", "core.autocrlf", "false"],
    ] {
        git(&dir, args).unwrap();
    }
    std::fs::write(dir.join("old.txt"), "one\ntwo\nthree\n").unwrap();
    git(&dir, &["add", "."]).unwrap();
    git(&dir, &["commit", "-qm", "add file"]).unwrap();
    git(&dir, &["mv", "old.txt", "new.txt"]).unwrap();
    git(&dir, &["commit", "-qm", "rename"]).unwrap();
    git(&dir, &["config", "user.name", "Bo"]).unwrap();
    std::fs::write(dir.join("new.txt"), "one\nTWO\nthree\n").unwrap();
    git(&dir, &["commit", "-qam", "shout two"]).unwrap();

    let repo = Repo::open(&dir).unwrap();
    let history = repo.file_history("new.txt", 100).unwrap();
    let summaries: Vec<&str> = history.iter().map(|c| c.summary.as_str()).collect();
    assert_eq!(summaries, ["shout two", "rename", "add file"]);
    assert_eq!(history[0].path, "new.txt");
    assert_eq!(history[2].path, "old.txt", "path at the commit before the rename");
    assert_eq!(history[0].author_name, "Bo");

    let blame = repo.blame("new.txt", None).unwrap();
    assert_eq!(blame.len(), 3);
    assert_eq!((blame[0].author_name.as_str(), blame[0].summary.as_str()), ("Ada", "add file"));
    assert_eq!((blame[1].author_name.as_str(), blame[1].text.as_str()), ("Bo", "TWO"));
    assert_eq!(blame[2].line_no, 3);
}
