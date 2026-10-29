//! Undo / redo for any operation: snapshot the local refs before and after,
//! and move the repository from one snapshot back to the other.
//!
//! Stash messages are the exception: stashes aren't in the snapshot (undoing
//! a stash push would lose its changes), so a stash rename records its own
//! [`StashEdit`] instead.
//!
//! Only refs move. The working tree is never overwritten: undoing a commit is
//! a soft reset (changes come back staged) and every other HEAD move uses
//! `reset --keep` / `checkout`, which refuse to clobber local changes.

use std::collections::BTreeMap;

use anyhow::{Result, bail};

use crate::repo::Repo;

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct RefState {
    /// Checked-out branch; `None` when HEAD is detached (or unborn).
    pub head_branch: Option<String>,
    pub head: Option<String>,
    pub branches: BTreeMap<String, String>,
    pub tags: BTreeMap<String, String>,
}

impl RefState {
    /// The branch HEAD is on, or the commit it is detached at.
    fn head_target(&self) -> Option<&str> {
        self.head_branch.as_deref().or(self.head.as_deref())
    }
}

#[derive(Debug, Clone)]
pub struct UndoEntry {
    pub label: String,
    pub change: UndoChange,
}

#[derive(Debug, Clone)]
pub enum UndoChange {
    Refs {
        before: RefState,
        after: RefState,
        /// A commit or amend: undo is a soft reset, so its changes come back staged.
        soft: bool,
    },
    Stash(StashEdit),
}

/// A stash whose commit (and message) was swapped for another in place.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StashEdit {
    pub old_target: String,
    pub old_message: String,
    pub new_target: String,
    pub new_message: String,
}

impl UndoEntry {
    pub fn new(label: &str, before: RefState, after: RefState, soft: bool) -> Option<Self> {
        (before != after).then(|| Self { label: label.to_string(), change: UndoChange::Refs { before, after, soft } })
    }

    pub fn stash(label: &str, edit: StashEdit) -> Self {
        Self { label: label.to_string(), change: UndoChange::Stash(edit) }
    }
}

impl Repo {
    pub fn ref_state(&self) -> Result<RefState> {
        let mut state = RefState {
            head_branch: self
                .git(&["symbolic-ref", "-q", "--short", "HEAD"])
                .ok()
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty()),
            head: self.git(&["rev-parse", "-q", "--verify", "HEAD"]).ok().map(|s| s.trim().to_string()),
            ..Default::default()
        };
        let refs = self.git(&["for-each-ref", "--format=%(refname) %(objectname)", "refs/heads", "refs/tags"])?;
        for line in refs.lines() {
            let Some((name, sha)) = line.split_once(' ') else { continue };
            if let Some(branch) = name.strip_prefix("refs/heads/") {
                state.branches.insert(branch.to_string(), sha.to_string());
            } else if let Some(tag) = name.strip_prefix("refs/tags/") {
                state.tags.insert(tag.to_string(), sha.to_string());
            }
        }
        Ok(state)
    }

    pub fn undo(&self, entry: &UndoEntry) -> Result<()> {
        match &entry.change {
            UndoChange::Refs { before, after, soft } => self.move_refs(after, before, *soft),
            UndoChange::Stash(edit) => self.swap_stash(&edit.new_target, &edit.old_target, &edit.old_message),
        }
    }

    pub fn redo(&self, entry: &UndoEntry) -> Result<()> {
        match &entry.change {
            UndoChange::Refs { before, after, soft } => self.move_refs(before, after, *soft),
            UndoChange::Stash(edit) => self.swap_stash(&edit.old_target, &edit.new_target, &edit.new_message),
        }
    }

    /// Put `to` where the stash `from` is now, wherever it moved in the list.
    fn swap_stash(&self, from: &str, to: &str, message: &str) -> Result<()> {
        let Some(stash) = self.stashes()?.into_iter().find(|s| s.target == from) else {
            bail!("that stash was dropped or changed since, so it can't be undone");
        };
        self.replace_stash(stash.index, to, message)
    }

    /// Move refs from `from` (which must be the current state) to `to`.
    fn move_refs(&self, from: &RefState, to: &RefState, soft: bool) -> Result<()> {
        if self.ref_state()? != *from {
            bail!("the repository changed since this action, so it can't be undone safely");
        }
        let current = from.head_branch.as_deref();

        // 1. Every ref except the checked-out branch can move freely.
        for (name, sha) in &to.branches {
            if Some(name.as_str()) != current && from.branches.get(name) != Some(sha) {
                self.git(&["update-ref", &format!("refs/heads/{name}"), sha])?;
            }
        }
        for name in from.branches.keys() {
            if Some(name.as_str()) != current && !to.branches.contains_key(name) {
                self.git(&["update-ref", "-d", &format!("refs/heads/{name}")])?;
            }
        }
        for (name, sha) in &to.tags {
            if from.tags.get(name) != Some(sha) {
                self.git(&["update-ref", &format!("refs/tags/{name}"), sha])?;
            }
        }
        for name in from.tags.keys() {
            if !to.tags.contains_key(name) {
                self.git(&["update-ref", "-d", &format!("refs/tags/{name}")])?;
            }
        }

        // 2. HEAD: same branch → reset it; otherwise check out the target first.
        if from.head_branch.is_some() && from.head_branch == to.head_branch {
            if let Some(target) = to.head.as_deref().filter(|t| Some(*t) != from.head.as_deref()) {
                self.git(&["reset", if soft { "--soft" } else { "--keep" }, target])?;
            }
            return Ok(());
        }
        if let Some(target) = to.head_target().filter(|t| Some(*t) != from.head_target()) {
            let mut args = vec!["checkout", "-q"];
            if to.head_branch.is_none() {
                args.push("--detach");
            }
            args.push(target);
            self.git(&args)?;
        }

        // 3. The previously checked-out branch is free now.
        if let Some(name) = current {
            match to.branches.get(name) {
                Some(sha) if from.branches.get(name) != Some(sha) => {
                    self.git(&["update-ref", &format!("refs/heads/{name}"), sha])?;
                }
                None => {
                    self.git(&["update-ref", "-d", &format!("refs/heads/{name}")])?;
                }
                _ => {}
            }
        }
        Ok(())
    }
}
