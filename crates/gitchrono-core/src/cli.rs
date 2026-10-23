//! Write path (and the few reads where matching git byte-for-byte matters):
//! everything here shells out to the user's `git`, so hooks, credentials,
//! SSH config and signing behave exactly like the terminal.

use std::path::Path;
use std::process::{Command, Stdio};

use anyhow::{Result, bail};

use crate::diff::{ApplyMode, parse_patch};
use crate::model::*;
use crate::repo::Repo;

#[derive(Debug, thiserror::Error)]
#[error("git {command} failed: {stderr}")]
pub struct GitError {
    pub command: String,
    pub stderr: String,
    pub code: Option<i32>,
}

pub fn git(cwd: &Path, args: &[&str]) -> Result<String> {
    let output = Command::new("git")
        .args(args)
        .current_dir(cwd)
        .env("GIT_OPTIONAL_LOCKS", "0")
        .env("GIT_TERMINAL_PROMPT", "0")
        .stdin(Stdio::null())
        .output()?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(GitError { command: args.join(" "), stderr, code: output.status.code() }.into());
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

fn status_from_xy(c: u8) -> Option<FileStatus> {
    Some(match c {
        b'A' => FileStatus::Added,
        b'M' => FileStatus::Modified,
        b'D' => FileStatus::Deleted,
        b'R' => FileStatus::Renamed,
        b'C' => FileStatus::Copied,
        b'T' => FileStatus::TypeChange,
        _ => return None,
    })
}

impl Repo {
    fn git(&self, args: &[&str]) -> Result<String> {
        git(self.root(), args)
    }

    fn git_dir(&self) -> std::path::PathBuf {
        let dir = self.git(&["rev-parse", "--absolute-git-dir"]).unwrap_or_default();
        std::path::PathBuf::from(dir.trim())
    }

    pub fn status(&self) -> Result<Status> {
        let out = self.git(&["status", "--porcelain=v2", "--branch", "-z", "--untracked-files=all"])?;
        let mut status = Status::default();
        let mut fields = out.split('\0').filter(|s| !s.is_empty());
        while let Some(entry) = fields.next() {
            if let Some(header) = entry.strip_prefix("# ") {
                if let Some(head) = header.strip_prefix("branch.head ") {
                    status.branch = (head != "(detached)").then(|| head.to_string());
                } else if let Some(up) = header.strip_prefix("branch.upstream ") {
                    status.upstream = Some(up.to_string());
                } else if let Some(ab) = header.strip_prefix("branch.ab ") {
                    let mut parts = ab.split(' ');
                    status.ahead = parts.next().and_then(|a| a[1..].parse().ok()).unwrap_or(0);
                    status.behind = parts.next().and_then(|b| b[1..].parse().ok()).unwrap_or(0);
                }
                continue;
            }
            let kind = entry.as_bytes()[0];
            match kind {
                b'?' => status.unstaged.push(FileChange {
                    path: entry[2..].to_string(),
                    old_path: None,
                    status: FileStatus::Untracked,
                }),
                b'1' | b'2' => {
                    let parts: Vec<&str> = entry.splitn(if kind == b'1' { 9 } else { 10 }, ' ').collect();
                    let xy = parts[1].as_bytes();
                    let path = parts.last().copied().unwrap_or_default().to_string();
                    let old_path = (kind == b'2').then(|| fields.next().unwrap_or_default().to_string());
                    if let Some(s) = status_from_xy(xy[0]) {
                        status.staged.push(FileChange { path: path.clone(), old_path: old_path.clone(), status: s });
                    }
                    if let Some(s) = status_from_xy(xy[1]) {
                        status.unstaged.push(FileChange { path, old_path, status: s });
                    }
                }
                b'u' => {
                    let path = entry.splitn(11, ' ').last().unwrap_or_default().to_string();
                    status.conflicted.push(FileChange { path, old_path: None, status: FileStatus::Conflicted });
                }
                _ => {}
            }
        }

        let dir = self.git_dir();
        status.operation = if dir.join("rebase-merge").exists() || dir.join("rebase-apply").exists() {
            Operation::Rebase
        } else if dir.join("MERGE_HEAD").exists() {
            Operation::Merge
        } else if dir.join("CHERRY_PICK_HEAD").exists() {
            Operation::CherryPick
        } else if dir.join("REVERT_HEAD").exists() {
            Operation::Revert
        } else {
            Operation::None
        };
        Ok(status)
    }

    pub fn stashes(&self) -> Result<Vec<Stash>> {
        let out = self.git(&["stash", "list", "--format=%H%x1f%gs"])?;
        Ok(out
            .lines()
            .enumerate()
            .filter_map(|(index, line)| {
                let (target, message) = line.split_once('\x1f')?;
                Some(Stash { index, message: message.to_string(), target: target.to_string() })
            })
            .collect())
    }

    /// Fill in upstream + ahead/behind for local branches.
    pub fn tracking(&self, refs: &mut Refs) -> Result<()> {
        let out = self.git(&[
            "for-each-ref",
            "--format=%(refname:short)%1f%(upstream:short)%1f%(upstream:track,nobracket)",
            "refs/heads",
        ])?;
        for line in out.lines() {
            let mut parts = line.split('\x1f');
            let (Some(name), Some(up), Some(track)) = (parts.next(), parts.next(), parts.next()) else {
                continue;
            };
            let Some(branch) = refs.local.iter_mut().find(|b| b.name == name) else { continue };
            branch.upstream = (!up.is_empty()).then(|| up.to_string());
            for part in track.split(", ") {
                if let Some(n) = part.strip_prefix("ahead ") {
                    branch.ahead = n.parse().unwrap_or(0);
                } else if let Some(n) = part.strip_prefix("behind ") {
                    branch.behind = n.parse().unwrap_or(0);
                }
            }
        }
        Ok(())
    }

    pub fn commit_file_diff(&self, commit: &str, path: &str) -> Result<FileDiff> {
        let out = self.git(&["show", "--format=", "--no-color", "-M", "--first-parent", commit, "--", path])?;
        Ok(parse_patch(path, &out))
    }

    pub fn workdir_file_diff(&self, change: &FileChange, staged: bool) -> Result<FileDiff> {
        let out = if change.status == FileStatus::Untracked {
            // `--no-index` exits 1 when files differ, which is the normal case here.
            let output = Command::new("git")
                .args(["diff", "--no-color", "--no-index", "--", "/dev/null", &change.path])
                .current_dir(self.root())
                .output()?;
            String::from_utf8_lossy(&output.stdout).into_owned()
        } else if staged {
            self.git(&["diff", "--no-color", "--cached", "-M", "--", &change.path])?
        } else {
            self.git(&["diff", "--no-color", "--", &change.path])?
        };
        Ok(parse_patch(&change.path, &out))
    }

    pub fn stage(&self, paths: &[&str]) -> Result<()> {
        let mut args = vec!["add", "-A", "--"];
        args.extend_from_slice(paths);
        self.git(&args).map(drop)
    }

    pub fn stage_all(&self) -> Result<()> {
        self.git(&["add", "-A"]).map(drop)
    }

    pub fn unstage(&self, paths: &[&str]) -> Result<()> {
        let mut args = vec!["restore", "--staged", "--"];
        args.extend_from_slice(paths);
        self.git(&args).map(drop)
    }

    pub fn unstage_all(&self) -> Result<()> {
        self.git(&["restore", "--staged", ":/"]).map(drop)
    }

    pub fn discard(&self, change: &FileChange) -> Result<()> {
        if change.status == FileStatus::Untracked {
            self.git(&["clean", "-f", "--", &change.path]).map(drop)
        } else {
            self.git(&["restore", "--worktree", "--", &change.path]).map(drop)
        }
    }

    /// Apply a patch from [`crate::partial_patch`] to the index or working tree.
    pub fn apply_patch(&self, patch: &str, mode: ApplyMode) -> Result<()> {
        use std::io::Write as _;
        let mut args = vec!["apply", "--recount", "--whitespace=nowarn"];
        if mode != ApplyMode::Discard {
            args.push("--cached");
        }
        if mode.reverse() {
            args.push("-R");
        }
        args.push("-");
        let mut child = Command::new("git")
            .args(&args)
            .current_dir(self.root())
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()?;
        child.stdin.take().expect("piped stdin").write_all(patch.as_bytes())?;
        let output = child.wait_with_output()?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
            return Err(GitError { command: args.join(" "), stderr, code: output.status.code() }.into());
        }
        Ok(())
    }

    pub fn push_tag(&self, name: &str) -> Result<()> {
        self.git(&["push", "origin", &format!("refs/tags/{name}")]).map(drop)
    }

    pub fn delete_tag(&self, name: &str) -> Result<()> {
        self.git(&["tag", "-d", name]).map(drop)
    }

    pub fn rename_branch(&self, old: &str, new: &str) -> Result<()> {
        self.git(&["branch", "-m", old, new]).map(drop)
    }

    pub fn delete_remote_branch(&self, remote_branch: &str) -> Result<()> {
        let (remote, branch) = remote_branch
            .split_once('/')
            .ok_or_else(|| anyhow::anyhow!("not a remote branch: {remote_branch}"))?;
        self.git(&["push", remote, "--delete", branch]).map(drop)
    }

    pub fn set_upstream(&self, branch: &str, upstream: &str) -> Result<()> {
        self.git(&["branch", "--set-upstream-to", upstream, branch]).map(drop)
    }

    pub fn commit(&self, message: &str, amend: bool) -> Result<()> {
        if message.trim().is_empty() && !amend {
            bail!("commit message is empty");
        }
        let mut args = vec!["commit", "-m", message];
        if amend {
            args.push("--amend");
        }
        self.git(&args).map(drop)
    }

    pub fn checkout(&self, target: &str) -> Result<()> {
        self.git(&["checkout", target]).map(drop)
    }

    /// Check out a remote branch as a new tracking local branch.
    pub fn checkout_remote(&self, remote_branch: &str) -> Result<()> {
        self.git(&["checkout", "--track", remote_branch]).map(drop)
    }

    pub fn create_branch(&self, name: &str, start: Option<&str>, checkout: bool) -> Result<()> {
        let mut args = if checkout { vec!["checkout", "-b", name] } else { vec!["branch", name] };
        if let Some(start) = start {
            args.push(start);
        }
        self.git(&args).map(drop)
    }

    pub fn delete_branch(&self, name: &str, force: bool) -> Result<()> {
        self.git(&["branch", if force { "-D" } else { "-d" }, name]).map(drop)
    }

    pub fn create_tag(&self, name: &str, target: &str) -> Result<()> {
        self.git(&["tag", name, target]).map(drop)
    }

    pub fn fetch(&self) -> Result<()> {
        self.git(&["fetch", "--all", "--prune"]).map(drop)
    }

    pub fn pull(&self, rebase: bool) -> Result<()> {
        self.git(&["pull", if rebase { "--rebase" } else { "--no-rebase" }]).map(drop)
    }

    pub fn push(&self, force: bool) -> Result<()> {
        let status = self.status()?;
        let mut args = vec!["push"];
        if force {
            args.push("--force-with-lease");
        }
        let branch;
        if status.upstream.is_none() {
            branch = status.branch.ok_or_else(|| anyhow::anyhow!("cannot push a detached HEAD"))?;
            args.extend(["-u", "origin", branch.as_str()]);
        }
        self.git(&args).map(drop)
    }

    pub fn stash_push(&self, message: Option<&str>) -> Result<()> {
        let mut args = vec!["stash", "push", "--include-untracked"];
        if let Some(m) = message {
            args.extend(["-m", m]);
        }
        self.git(&args).map(drop)
    }

    pub fn stash_pop(&self, index: usize) -> Result<()> {
        self.git(&["stash", "pop", &format!("stash@{{{index}}}")]).map(drop)
    }

    pub fn stash_apply(&self, index: usize) -> Result<()> {
        self.git(&["stash", "apply", &format!("stash@{{{index}}}")]).map(drop)
    }

    pub fn stash_drop(&self, index: usize) -> Result<()> {
        self.git(&["stash", "drop", &format!("stash@{{{index}}}")]).map(drop)
    }

    pub fn merge(&self, target: &str) -> Result<()> {
        self.git(&["merge", "--no-edit", target]).map(drop)
    }

    pub fn rebase(&self, onto: &str) -> Result<()> {
        self.git(&["rebase", onto]).map(drop)
    }

    pub fn cherry_pick(&self, commit: &str) -> Result<()> {
        self.git(&["cherry-pick", commit]).map(drop)
    }

    pub fn revert(&self, commit: &str) -> Result<()> {
        self.git(&["revert", "--no-edit", commit]).map(drop)
    }

    pub fn reset(&self, commit: &str, mode: ResetMode) -> Result<()> {
        let flag = match mode {
            ResetMode::Soft => "--soft",
            ResetMode::Mixed => "--mixed",
            ResetMode::Hard => "--hard",
        };
        self.git(&["reset", flag, commit]).map(drop)
    }

    /// `--continue` or `--abort` the in-progress merge/rebase/cherry-pick/revert.
    pub fn resolve_operation(&self, op: Operation, abort: bool) -> Result<()> {
        let cmd = match op {
            Operation::None => return Ok(()),
            Operation::Merge => "merge",
            Operation::Rebase => "rebase",
            Operation::CherryPick => "cherry-pick",
            Operation::Revert => "revert",
        };
        let flag = if abort { "--abort" } else { "--continue" };
        let output = Command::new("git")
            .args([cmd, flag])
            .current_dir(self.root())
            .env("GIT_EDITOR", "true")
            .output()?;
        if !output.status.success() {
            bail!("{}", String::from_utf8_lossy(&output.stderr).trim());
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy)]
pub enum ResetMode {
    Soft,
    Mixed,
    Hard,
}
