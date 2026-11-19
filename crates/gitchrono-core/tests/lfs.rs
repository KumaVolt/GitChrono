//! Git LFS against the real `git lfs`. Skipped where it isn't installed.

use std::path::{Path, PathBuf};

use gitchrono_core::lfs::{self, LfsState};
use gitchrono_core::{FileChange, FileStatus, Repo, git};

fn dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("gitchrono-lfs-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn configure(dir: &Path) {
    for args in [
        &["config", "user.name", "Test"][..],
        &["config", "user.email", "test@example.com"],
        &["config", "commit.gpgsign", "false"],
    ] {
        git(dir, args).unwrap();
    }
}

#[test]
fn track_commit_clone_and_download() {
    if !lfs::installed() {
        eprintln!("git-lfs isn't installed; skipping");
        return;
    }
    let root = dir("flow");
    let (origin, work) = (root.join("origin.git"), root.join("work"));
    git(&root, &["init", "-q", "--bare", "-b", "main", origin.to_str().unwrap()]).unwrap();
    git(&root, &["clone", "-q", origin.to_str().unwrap(), work.to_str().unwrap()]).unwrap();
    configure(&work);
    std::fs::write(work.join("readme.txt"), "hi\n").unwrap();
    let repo = Repo::open(&work).unwrap();
    assert_eq!(repo.lfs_state(), LfsState::NotUsed);

    // Tracking sets LFS up for this repository and writes .gitattributes.
    repo.lfs_track("*.bin").unwrap();
    assert!(std::fs::read_to_string(work.join(".gitattributes")).unwrap().contains("*.bin filter=lfs"));
    assert!(repo.uses_lfs(), "a new .gitattributes counts");
    let big = vec![7u8; 300_000];
    std::fs::write(work.join("asset.bin"), &big).unwrap();
    assert!(repo.is_lfs_path("asset.bin") && !repo.is_lfs_path("readme.txt"));

    // A new LFS file's diff says where it's going, not its bytes.
    let new = FileChange { path: "asset.bin".into(), old_path: None, status: FileStatus::Untracked };
    let diff = repo.workdir_file_diff(&new, false).unwrap();
    assert_eq!(diff.lfs.map(|l| (l.old_size, l.new_size)), Some((None, Some(300_000))));

    git(&work, &["add", "."]).unwrap();
    git(&work, &["commit", "-qm", "add asset"]).unwrap();
    std::fs::write(work.join("asset.bin"), vec![8u8; 400_000]).unwrap();
    git(&work, &["commit", "-qam", "bigger asset"]).unwrap();
    let diff = repo.commit_file_diff("HEAD", "asset.bin").unwrap();
    assert_eq!(diff.lfs.map(|l| (l.old_size, l.new_size)), Some((Some(300_000), Some(400_000))), "sizes, not pointer text");
    assert!(diff.hunks.is_empty());
    git(&work, &["push", "-q", "-u", "origin", "main"]).unwrap();
    assert_eq!(repo.lfs_state(), LfsState::Ready { not_downloaded: 0 });

    // A clone that skipped the download has placeholders until `lfs_pull`.
    let copy = root.join("copy");
    let out = gitchrono_core::gitbin::git_command()
        .args(["clone", "-q", origin.to_str().unwrap(), copy.to_str().unwrap()])
        .env("GIT_LFS_SKIP_SMUDGE", "1")
        .output()
        .unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let copy_repo = Repo::open(&copy).unwrap();
    git(&copy, &["lfs", "install", "--local"]).unwrap();
    assert_eq!(copy_repo.lfs_state(), LfsState::Ready { not_downloaded: 1 });
    assert!(std::fs::metadata(copy.join("asset.bin")).unwrap().len() < 1000, "a placeholder");
    copy_repo.lfs_pull().unwrap();
    assert_eq!(std::fs::metadata(copy.join("asset.bin")).unwrap().len(), 400_000);
    assert_eq!(copy_repo.lfs_state(), LfsState::Ready { not_downloaded: 0 });
    let _ = std::fs::remove_dir_all(&root);
}
