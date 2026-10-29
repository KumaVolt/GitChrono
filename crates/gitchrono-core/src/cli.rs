//! Write path (and the few reads where matching git byte-for-byte matters):
//! everything here shells out to the user's `git`, so hooks, credentials,
//! SSH config and signing behave exactly like the terminal.

use std::path::Path;
use std::process::Stdio;

use anyhow::{Result, bail};

use crate::diff::{ApplyMode, parse_patch};
use crate::model::*;
use crate::repo::Repo;
use crate::undo::StashEdit;

#[derive(Debug, thiserror::Error)]
#[error("git {command} failed: {stderr}")]
pub struct GitError {
    pub command: String,
    pub stderr: String,
    pub code: Option<i32>,
}

impl GitError {
    /// A push the remote refused because it has commits we don't (not a
    /// permission or network problem).
    pub fn is_push_rejected(&self) -> bool {
        // "[remote rejected]" (hooks, protected branches) needs a different fix.
        self.command.starts_with("push") && ["[rejected]", "fetch first", "non-fast-forward"].iter().any(|s| self.stderr.contains(s))
    }

    /// A pull with nothing to pull from: no upstream set, or not on a branch.
    pub fn is_no_upstream(&self) -> bool {
        self.command.starts_with("pull")
            && ["no tracking information", "not currently on a branch"].iter().any(|s| self.stderr.contains(s))
    }
}

/// Why the current branch has nothing to pull from, and what fixes it. Pull
/// and push return this as their error, so the caller can offer the fix.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NoUpstream {
    Detached,
    /// Remote branches with the same name, best first: track one, then pull.
    Track { branch: String, candidates: Vec<String> },
    /// Remotes (best first), none of which has this branch: publish it.
    Publish { branch: String, remotes: Vec<String> },
    NoRemote,
}

impl std::error::Error for NoUpstream {}

impl std::fmt::Display for NoUpstream {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Detached => write!(f, "You're not on a branch, so there's nothing to pull into. Check out a branch first."),
            Self::Track { branch, candidates } => {
                let exist = if candidates.len() == 1 { "exists" } else { "exist" };
                write!(f, "{branch} isn't linked to a remote branch yet, but {} {exist}.", candidates.join(" and "))
            }
            Self::Publish { branch, remotes } => {
                let on = if remotes.len() == 1 { remotes[0].as_str() } else { "any remote" };
                write!(f, "{branch} isn't on {on} yet, so there's nothing to pull. Publish it to share it.")
            }
            Self::NoRemote => write!(f, "This repository has no remote yet. Add one in the terminal with `git remote add origin <url>`, then push."),
        }
    }
}

pub fn git(cwd: &Path, args: &[&str]) -> Result<String> {
    let output = crate::gitbin::git_command()
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

fn parse_file_history(out: &str, path: &str) -> Vec<FileCommit> {
    let mut list = Vec::new();
    for record in out.split('\x1e').filter(|r| !r.trim().is_empty()) {
        let mut lines = record.lines();
        let header: Vec<&str> = lines.next().unwrap_or_default().split('\0').collect();
        if header.len() < 4 {
            continue;
        }
        // `--name-status`: "M\tpath" or "R100\told\tnew"; the last field is the path here.
        let file = lines
            .filter(|l| !l.is_empty())
            .find_map(|l| l.split('\t').next_back().map(str::to_string))
            .unwrap_or_else(|| path.to_string());
        list.push(FileCommit {
            id: header[0].to_string(),
            author_name: header[1].to_string(),
            time: header[2].parse().unwrap_or(0),
            summary: header[3].to_string(),
            path: file,
        });
    }
    list
}

fn parse_blame(out: &str) -> Vec<BlameLine> {
    struct Info {
        author: String,
        time: i64,
        summary: String,
    }
    let mut infos: std::collections::HashMap<String, Info> = std::collections::HashMap::new();
    let mut lines = Vec::new();
    let mut current: Option<(String, usize)> = None;
    for line in out.lines() {
        if let Some(text) = line.strip_prefix('\t') {
            if let Some((id, line_no)) = current.take() {
                let info = infos.get(&id);
                lines.push(BlameLine {
                    author_name: info.map(|i| i.author.clone()).unwrap_or_default(),
                    time: info.map_or(0, |i| i.time),
                    summary: info.map(|i| i.summary.clone()).unwrap_or_default(),
                    id,
                    line_no,
                    text: text.to_string(),
                });
            }
            continue;
        }
        let mut parts = line.split(' ');
        let first = parts.next().unwrap_or_default();
        if first.len() == 40 && first.bytes().all(|b| b.is_ascii_hexdigit()) {
            let _orig = parts.next();
            let final_no = parts.next().and_then(|n| n.parse().ok()).unwrap_or(0);
            infos.entry(first.to_string()).or_insert(Info { author: String::new(), time: 0, summary: String::new() });
            current = Some((first.to_string(), final_no));
            continue;
        }
        let Some((id, _)) = &current else { continue };
        let Some(info) = infos.get_mut(id) else { continue };
        let rest = line.split_once(' ').map_or("", |(_, r)| r);
        match first {
            "author" => info.author = rest.to_string(),
            "author-time" => info.time = rest.parse().unwrap_or(0),
            "summary" => info.summary = rest.to_string(),
            _ => {}
        }
    }
    lines
}

fn parse_worktrees(out: &str) -> Vec<Worktree> {
    let mut list = Vec::new();
    for block in out.split("\n\n").filter(|b| !b.trim().is_empty()) {
        let mut wt = Worktree { path: Default::default(), head: None, branch: None, main: list.is_empty(), locked: false, prunable: false };
        for line in block.lines() {
            let (key, value) = line.split_once(' ').unwrap_or((line, ""));
            match key {
                "worktree" => wt.path = value.into(),
                "HEAD" => wt.head = Some(value.to_string()),
                "branch" => wt.branch = Some(value.strip_prefix("refs/heads/").unwrap_or(value).to_string()),
                "locked" => wt.locked = true,
                "prunable" => wt.prunable = true,
                _ => {}
            }
        }
        if !wt.path.as_os_str().is_empty() {
            list.push(wt);
        }
    }
    list
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
    /// `git init` in `path` (created if missing) and open it. A folder that
    /// already holds a repository is opened as it is. New repositories start
    /// on `main` unless `init.defaultBranch` names another branch.
    pub fn init(path: &Path) -> Result<Self> {
        if !path.join(".git").exists() {
            std::fs::create_dir_all(path)?;
            let configured = git(path, &["config", "--get", "init.defaultBranch"]).is_ok();
            git(path, if configured { &["init", "-q"] } else { &["init", "-q", "-b", "main"] })?;
        }
        Self::open(path)
    }

    pub(crate) fn git(&self, args: &[&str]) -> Result<String> {
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
            let output = crate::gitbin::git_command()
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

    /// The whole staged patch (what the next commit contains).
    pub fn staged_patch(&self) -> Result<String> {
        self.git(&["diff", "--no-color", "--cached", "-M"])
    }

    /// What a commit changed; merges against their first parent (what they brought in).
    pub fn commit_patch(&self, id: &str) -> Result<String> {
        self.git(&["show", "--no-color", "-M", "--format=", "--diff-merges=first-parent", id])
    }

    /// Every working-tree change against HEAD, plus the names of untracked files.
    pub fn workdir_patch(&self) -> Result<String> {
        let mut out = self.git(&["diff", "--no-color", "-M", "HEAD"]).or_else(|_| self.git(&["diff", "--no-color", "--cached"]))?;
        let untracked = self.git(&["ls-files", "--others", "--exclude-standard"])?;
        for path in untracked.lines().filter(|l| !l.is_empty()) {
            out.push_str(&format!("diff --git a/{path} b/{path}\nnew file (untracked)\n"));
        }
        Ok(out)
    }

    /// Commits touching `path` (newest first), following renames.
    pub fn file_history(&self, path: &str, limit: usize) -> Result<Vec<FileCommit>> {
        let limit = format!("-{limit}");
        let out = self.git(&["log", &limit, "--follow", "-M", "--format=%x1e%H%x00%an%x00%at%x00%s", "--name-status", "--", path])?;
        Ok(parse_file_history(&out, path))
    }

    /// `git blame` of `path` at `rev` (the working tree when `None`).
    pub fn blame(&self, path: &str, rev: Option<&str>) -> Result<Vec<BlameLine>> {
        let mut args = vec!["blame", "--porcelain"];
        args.extend(rev);
        args.extend(["--", path]);
        Ok(parse_blame(&self.git(&args)?))
    }

    /// Signature status of a commit; `None` when unsigned.
    pub fn signature(&self, id: &str) -> Result<Option<Signature>> {
        let out = self.git(&["log", "-1", "--format=%G?%x00%GS%x00%GK", id])?;
        let mut parts = out.trim_end_matches('\n').split('\0');
        let status = parts.next().and_then(|s| s.chars().next()).unwrap_or('N');
        if status == 'N' {
            return Ok(None);
        }
        Ok(Some(Signature {
            status,
            signer: parts.next().unwrap_or_default().to_string(),
            key: parts.next().unwrap_or_default().to_string(),
        }))
    }

    pub fn submodules(&self) -> Result<Vec<Submodule>> {
        if !self.root().join(".gitmodules").exists() {
            return Ok(Vec::new());
        }
        let out = self.git(&["submodule", "status"])?;
        Ok(out
            .lines()
            .filter(|l| l.len() > 41)
            .map(|l| {
                let state = l.chars().next().unwrap_or(' ');
                let rest = &l[1..];
                let (sha, rest) = rest.split_once(' ').unwrap_or((rest, ""));
                let path = rest.split(" (").next().unwrap_or(rest).to_string();
                Submodule { path, sha: sha.to_string(), state }
            })
            .collect())
    }

    pub fn update_submodule(&self, path: &str) -> Result<()> {
        self.git(&["submodule", "update", "--init", "--recursive", "--", path]).map(drop)
    }

    pub fn worktrees(&self) -> Result<Vec<Worktree>> {
        Ok(parse_worktrees(&self.git(&["worktree", "list", "--porcelain"])?))
    }

    /// `git worktree add`: check out `start` at `path`, creating `new_branch` if given.
    pub fn add_worktree(&self, path: &Path, new_branch: Option<&str>, start: Option<&str>) -> Result<()> {
        let path = path.to_string_lossy();
        let mut args = vec!["worktree", "add"];
        if let Some(branch) = new_branch {
            args.extend(["-b", branch]);
        }
        args.push(&path);
        args.extend(start);
        self.git(&args).map(drop)
    }

    pub fn remove_worktree(&self, path: &Path, force: bool) -> Result<()> {
        let path = path.to_string_lossy();
        let mut args = vec!["worktree", "remove"];
        if force {
            args.push("--force");
        }
        args.push(&path);
        self.git(&args).map(drop)
    }

    pub fn stage(&self, paths: &[&str]) -> Result<()> {
        let mut args = vec!["add", "-A", "--"];
        args.extend_from_slice(paths);
        self.git(&args).map(drop)
    }

    pub fn stage_all(&self) -> Result<()> {
        self.git(&["add", "-A"]).map(drop)
    }

    /// Before the first commit there is no HEAD to restore the index from.
    fn unborn(&self) -> bool {
        self.git(&["rev-parse", "-q", "--verify", "HEAD"]).is_err()
    }

    pub fn unstage(&self, paths: &[&str]) -> Result<()> {
        let mut args = if self.unborn() { vec!["rm", "--cached", "-r", "-q", "--"] } else { vec!["restore", "--staged", "--"] };
        args.extend_from_slice(paths);
        self.git(&args).map(drop)
    }

    pub fn unstage_all(&self) -> Result<()> {
        if self.unborn() {
            return self.git(&["rm", "--cached", "-r", "-q", "--ignore-unmatch", ":/"]).map(drop);
        }
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
        let mut child = crate::gitbin::git_command()
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

    pub fn remotes(&self) -> Result<Vec<String>> {
        Ok(self.git(&["remote"])?.lines().map(str::to_string).collect())
    }

    /// Pushes tag `name` to `remote`, never forced: a remote that already has
    /// the tag on another commit refuses it, and the error says so.
    pub fn push_tag(&self, remote: &str, name: &str) -> Result<()> {
        match self.git(&["push", remote, &format!("refs/tags/{name}")]) {
            Ok(_) => Ok(()),
            Err(e) if e.downcast_ref::<GitError>().is_some_and(|g| g.stderr.contains("already exists")) => {
                bail!("{remote} already has a tag {name} on a different commit. Delete it there or rename yours to push it.")
            }
            Err(e) => Err(e),
        }
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

    /// Check out a remote branch. If a local branch of the same name exists,
    /// switch to it and fast-forward it when it's only behind; otherwise
    /// create a tracking branch.
    pub fn checkout_remote(&self, remote_branch: &str) -> Result<()> {
        let remotes = self.git(&["remote"])?;
        // The longest matching remote wins, as remote names may contain '/'.
        let local = remotes
            .lines()
            .filter_map(|r| remote_branch.strip_prefix(r).and_then(|rest| rest.strip_prefix('/')).map(|rest| (r.len(), rest)))
            .max_by_key(|(len, _)| *len)
            .map(|(_, rest)| rest);
        let Some(local) = local.filter(|l| self.git(&["rev-parse", "--verify", "--quiet", &format!("refs/heads/{l}")]).is_ok()) else {
            return self.git(&["checkout", "--track", remote_branch]).map(drop);
        };
        self.git(&["checkout", local])?;
        if self.git(&["merge-base", "--is-ancestor", local, remote_branch]).is_ok() {
            self.git(&["merge", "--ff-only", "--quiet", remote_branch])?;
        }
        Ok(())
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

    /// `rebase: None` follows the user's `pull.rebase` setting, merging when
    /// it isn't set (plain `git pull` refuses diverged branches then).
    /// Fails with [`NoUpstream`] when there's nothing to pull from.
    pub fn pull(&self, rebase: Option<bool>) -> Result<()> {
        if let Some(why) = self.missing_upstream()? {
            return Err(why.into());
        }
        let rebase = rebase.or_else(|| self.git(&["config", "--type=bool", "--get", "pull.rebase"]).ok().map(|v| v.trim() == "true"));
        self.git(&["pull", if rebase == Some(true) { "--rebase" } else { "--no-rebase" }]).map(drop).map_err(|e| {
            // Whatever the check above missed (say, an upstream with no remote).
            match e.downcast_ref::<GitError>().filter(|g| g.is_no_upstream()).and_then(|_| self.missing_upstream().ok().flatten()) {
                Some(why) => why.into(),
                None => e,
            }
        })
    }

    /// Why the current branch has nothing to pull from, or `None` when it has
    /// an upstream. Fetches first if no remote branch of the same name is known.
    pub fn missing_upstream(&self) -> Result<Option<NoUpstream>> {
        let status = self.status()?;
        if status.upstream.is_some() {
            return Ok(None);
        }
        let Some(branch) = status.branch else { return Ok(Some(NoUpstream::Detached)) };
        let remotes = self.remotes_by_preference()?;
        if remotes.is_empty() {
            return Ok(Some(NoUpstream::NoRemote));
        }
        let candidates = || {
            remotes
                .iter()
                .map(|r| format!("{r}/{branch}"))
                .filter(|c| self.git(&["rev-parse", "--verify", "--quiet", &format!("refs/remotes/{c}")]).is_ok())
                .collect::<Vec<_>>()
        };
        let mut found = candidates();
        if found.is_empty() {
            // The remote may have it without our knowing; offline, go by what we have.
            if self.git(&["fetch", "--all", "--quiet"]).is_ok() {
                found = candidates();
            }
        }
        Ok(Some(if found.is_empty() {
            NoUpstream::Publish { branch, remotes }
        } else {
            NoUpstream::Track { branch, candidates: found }
        }))
    }

    /// Remote names, the one to push to by default first: `remote.pushDefault`,
    /// then `origin`, then the rest.
    fn remotes_by_preference(&self) -> Result<Vec<String>> {
        let mut remotes: Vec<String> = self.git(&["remote"])?.lines().map(str::to_string).collect();
        let preferred = self.git(&["config", "--get", "remote.pushDefault"]).map(|s| s.trim().to_string()).unwrap_or_default();
        remotes.sort_by_key(|r| (*r != preferred, r.as_str() != "origin"));
        Ok(remotes)
    }

    /// Sets the current branch to track `upstream` (`origin/main`), then pulls.
    pub fn track_and_pull(&self, upstream: &str) -> Result<()> {
        let branch = self.status()?.branch.ok_or(NoUpstream::Detached)?;
        self.set_upstream(&branch, upstream)?;
        self.pull(None)
    }

    /// Pushes the current branch to `remote` and tracks it there.
    pub fn publish(&self, remote: &str) -> Result<()> {
        let branch = self.status()?.branch.ok_or_else(|| anyhow::anyhow!("cannot push a detached HEAD"))?;
        self.git(&["push", "-u", remote, &branch]).map(drop)
    }

    /// Pulls (as `pull(None)`), then pushes: for when the remote moved on.
    pub fn pull_then_push(&self) -> Result<()> {
        self.pull(None)?;
        self.push(false)
    }

    pub fn push(&self, force: bool) -> Result<()> {
        let status = self.status()?;
        let mut args = vec!["push"];
        if force {
            args.push("--force-with-lease");
        }
        let (branch, remote);
        if status.upstream.is_none() {
            branch = status.branch.ok_or_else(|| anyhow::anyhow!("cannot push a detached HEAD"))?;
            remote = self.remotes_by_preference()?.into_iter().next().ok_or(NoUpstream::NoRemote)?;
            args.extend(["-u", remote.as_str(), branch.as_str()]);
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

    /// Give stash@{index} a new message, keeping its place in the list. Git
    /// has no command for this: the stash commit is rewritten with the new
    /// message (same tree, parents and dates) and put back where it was.
    pub fn rename_stash(&self, index: usize, message: &str) -> Result<StashEdit> {
        let message = message.trim();
        if message.is_empty() {
            bail!("stash message is empty");
        }
        let stash = self.stashes()?.into_iter().nth(index).ok_or_else(|| anyhow::anyhow!("there is no stash@{{{index}}}"))?;
        let raw = self.git(&["cat-file", "commit", &stash.target])?;
        let headers = raw.split_once("\n\n").map_or(raw.as_str(), |(h, _)| h);
        let file = std::env::temp_dir().join(format!("gitchrono-stash-{}-{}", std::process::id(), stash.target));
        std::fs::write(&file, format!("{headers}\n\n{message}\n"))?;
        let written = self.git(&["hash-object", "-t", "commit", "-w", &file.to_string_lossy()]);
        let _ = std::fs::remove_file(&file);
        let edit = StashEdit {
            old_target: stash.target,
            old_message: stash.message,
            new_target: written?.trim().to_string(),
            // The reflog holds one line: what `git stash list` shows.
            new_message: message.lines().next().unwrap_or_default().to_string(),
        };
        self.replace_stash(index, &edit.new_target, &edit.new_message)?;
        Ok(edit)
    }

    /// Swap the stash at `index` for `target`, keeping the order: the entries
    /// above it are dropped with it and stored back, oldest first.
    pub(crate) fn replace_stash(&self, index: usize, target: &str, message: &str) -> Result<()> {
        let stashes = self.stashes()?;
        if index >= stashes.len() {
            bail!("there is no stash@{{{index}}}");
        }
        let store = |s: &Stash| {
            let (target, message) = if s.index == index { (target, message) } else { (s.target.as_str(), s.message.as_str()) };
            self.git(&["stash", "store", "-m", message, target])
        };
        for dropped in 0..=index {
            if let Err(e) = self.git(&["stash", "drop", "-q", "stash@{0}"]) {
                // Put back what's gone already, as it was.
                for s in stashes[..dropped].iter().rev() {
                    self.git(&["stash", "store", "-m", &s.message, &s.target])?;
                }
                return Err(e);
            }
        }
        stashes[..=index].iter().rev().try_for_each(|s| store(s).map(drop))
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
        let output = crate::gitbin::git_command()
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
