//! Applying a stash on top of uncommitted changes. When the two touch
//! different files the stash simply joins the changes. When they share a
//! file, the changes are stashed first so nothing gets merged into (or
//! overwrites) work in progress; that swap is undoable like a ref move.

use std::collections::BTreeSet;
use std::process::Stdio;

use anyhow::{Result, bail};

use crate::cli::GitError;
use crate::repo::Repo;
use crate::undo::RefState;

const SAVED_MESSAGE: &str = "Changes saved before applying a stash";

/// Index and working tree (untracked files included) as tree ids, to tell
/// whether anything changed between an operation and its undo.
#[derive(Debug, Clone, PartialEq, Eq)]
struct WorkState {
    index: String,
    files: String,
}

/// What `apply_stash` did when it had to set the current changes aside.
#[derive(Debug, Clone)]
pub struct StashSwap {
    /// Files both the stash and the current changes touch.
    pub overlap: Vec<String>,
    /// The applied stash: commit, list message and position.
    applied: String,
    applied_message: String,
    index: usize,
    pop: bool,
    /// The stash holding the changes that were set aside (`stash@{0}` after).
    saved: String,
    saved_message: String,
    pub(crate) refs: RefState,
    before: WorkState,
    after: WorkState,
}

impl Repo {
    /// Apply (or pop) `stash@{index}`. Uncommitted changes in other files are
    /// kept as they are. If the stash touches a file that has changes, all
    /// changes are stashed first and the returned swap says so.
    pub fn apply_stash(&self, index: usize, pop: bool) -> Result<Option<StashSwap>> {
        let (applied, applied_message) = self.stash_entry(index)?;
        let overlap = self.stash_overlap(&applied)?;
        if overlap.is_empty() {
            self.git(&["stash", if pop { "pop" } else { "apply" }, &format!("stash@{{{index}}}")])?;
            return Ok(None);
        }

        let refs = self.ref_state()?;
        let before = self.work_state()?;
        self.git(&["stash", "push", "--include-untracked", "-m", SAVED_MESSAGE])?;
        let swap = (|| {
            let (saved, saved_message) = self.stash_entry(0)?;
            // The saved changes pushed the chosen stash down one.
            self.git(&["stash", if pop { "pop" } else { "apply" }, &format!("stash@{{{}}}", index + 1)])?;
            let after = self.work_state()?;
            Ok(StashSwap { overlap, applied, applied_message, index, pop, saved, saved_message, refs, before, after })
        })();
        swap.map(Some).map_err(|e: anyhow::Error| e.context("your changes were saved as stash@{0}, but applying the stash then failed"))
    }

    pub(crate) fn undo_stash_swap(&self, swap: &StashSwap) -> Result<()> {
        self.check_unchanged(swap, &swap.after)?;
        if self.stash_entry(0).ok().map(|(id, _)| id) != Some(swap.saved.clone()) {
            bail!("the stash list changed since this action, so it can't be undone safely");
        }
        // Everything here came out of the applied stash, so this loses nothing.
        self.git(&["reset", "-q", "--hard"])?;
        self.git(&["clean", "-q", "-fd"])?;
        self.git(&["stash", "pop", "-q", "--index"])?;
        if swap.pop {
            self.stash_insert(swap.index, &swap.applied, &swap.applied_message)?;
        }
        Ok(())
    }

    pub(crate) fn redo_stash_swap(&self, swap: &StashSwap) -> Result<()> {
        self.check_unchanged(swap, &swap.before)?;
        if swap.pop && self.stash_entry(swap.index).ok().map(|(id, _)| id) != Some(swap.applied.clone()) {
            bail!("the stash list changed since this action, so it can't be redone safely");
        }
        // Same as `stash push`, but reusing the commit made the first time.
        self.git(&["stash", "store", "-m", &swap.saved_message, &swap.saved])?;
        self.git(&["reset", "-q", "--hard"])?;
        self.git(&["clean", "-q", "-fd"])?;
        if swap.pop {
            self.git(&["stash", "pop", "-q", &format!("stash@{{{}}}", swap.index + 1)])?;
        } else {
            self.git(&["stash", "apply", "-q", &swap.applied])?;
        }
        Ok(())
    }

    fn check_unchanged(&self, swap: &StashSwap, work: &WorkState) -> Result<()> {
        if self.ref_state()? != swap.refs {
            bail!("the repository changed since this action, so it can't be undone safely");
        }
        if self.work_state()? != *work {
            bail!("files changed since this action; commit or stash those edits first");
        }
        Ok(())
    }

    /// Commit and list message of `stash@{index}`.
    fn stash_entry(&self, index: usize) -> Result<(String, String)> {
        match self.stashes()?.into_iter().nth(index) {
            Some(s) => Ok((s.target, s.message)),
            None => bail!("there is no stash@{{{index}}}"),
        }
    }

    /// Put a dropped stash back at its old position in the list.
    fn stash_insert(&self, index: usize, commit: &str, message: &str) -> Result<()> {
        let above: Vec<_> = self.stashes()?.into_iter().take(index).collect();
        for _ in &above {
            self.git(&["stash", "drop", "-q", "stash@{0}"])?;
        }
        self.git(&["stash", "store", "-m", message, commit])?;
        for s in above.iter().rev() {
            self.git(&["stash", "store", "-m", &s.message, &s.target])?;
        }
        Ok(())
    }

    /// Files the stash changes that also have uncommitted changes (staged,
    /// unstaged or untracked).
    fn stash_overlap(&self, stash: &str) -> Result<Vec<String>> {
        let paths = |out: String| out.split('\0').filter(|p| !p.is_empty()).map(str::to_string).collect::<BTreeSet<_>>();
        let mut touched = paths(self.git(&["diff", "-z", "--name-only", "--no-renames", &format!("{stash}^1"), stash])?);
        let untracked = format!("{stash}^3");
        if self.git(&["rev-parse", "-q", "--verify", &untracked]).is_ok() {
            touched.extend(paths(self.git(&["ls-tree", "-r", "-z", "--name-only", &untracked])?));
        }
        let mut local = paths(self.git(&["diff", "-z", "--name-only", "--no-renames", "HEAD"])?);
        local.extend(paths(self.git(&["diff", "-z", "--name-only", "--no-renames", "--cached"])?));
        local.extend(paths(self.git(&["ls-files", "-z", "--others", "--exclude-standard"])?));
        Ok(touched.intersection(&local).cloned().collect())
    }

    fn work_state(&self) -> Result<WorkState> {
        let index = self.git(&["write-tree"])?.trim().to_string();
        // Stage everything into a copy of the index (keeping its stat cache,
        // so only changed files get hashed) and write that as a tree.
        let real = self.root().join(self.git(&["rev-parse", "--git-path", "index"])?.trim());
        let copy = real.with_file_name("gitchrono-work-index");
        if real.exists() {
            std::fs::copy(&real, &copy)?;
        }
        let files = self.git_with_index(&copy, &["add", "-A"]).and_then(|_| self.git_with_index(&copy, &["write-tree"]));
        let _ = std::fs::remove_file(&copy);
        Ok(WorkState { index, files: files?.trim().to_string() })
    }

    fn git_with_index(&self, index: &std::path::Path, args: &[&str]) -> Result<String> {
        let output = crate::gitbin::git_command()
            .args(args)
            .current_dir(self.root())
            .env("GIT_INDEX_FILE", index)
            .env("GIT_OPTIONAL_LOCKS", "0")
            .stdin(Stdio::null())
            .output()?;
        if !output.status.success() {
            return Err(GitError::from_output(args.join(" "), &output).into());
        }
        Ok(String::from_utf8_lossy(&output.stdout).into_owned())
    }
}
