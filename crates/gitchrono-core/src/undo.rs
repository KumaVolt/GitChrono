//! Undo / redo for any operation: snapshot the local refs before and after,
//! and move the repository from one snapshot back to the other.
//!
//! Only refs move (and the stash swap in `stash`, which checks the files
//! are exactly as it left them). The working tree is never overwritten: undoing a commit is
//! a soft reset (changes come back staged) and every other HEAD move uses
//! `reset --keep` / `checkout`, which refuse to clobber local changes.

use std::collections::BTreeMap;

use anyhow::{Result, bail};

use crate::repo::Repo;
use crate::stash::StashSwap;

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
    pub before: RefState,
    pub after: RefState,
    /// A commit or amend: undo is a soft reset, so its changes come back staged.
    pub soft: bool,
    /// Applying a stash that collided with uncommitted changes (no ref moves).
    pub stash: Option<StashSwap>,
}

impl UndoEntry {
    pub fn new(label: &str, before: RefState, after: RefState, soft: bool) -> Option<Self> {
        (before != after).then(|| Self { label: label.to_string(), before, after, soft, stash: None })
    }

    pub fn stash_swap(label: &str, swap: StashSwap) -> Self {
        let refs = swap.refs.clone();
        Self { label: label.to_string(), before: refs.clone(), after: refs, soft: false, stash: Some(swap) }
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
        if let Some(swap) = &entry.stash {
            return self.undo_stash_swap(swap);
        }
        self.move_refs(&entry.after, &entry.before, entry.soft)
    }

    pub fn redo(&self, entry: &UndoEntry) -> Result<()> {
        if let Some(swap) = &entry.stash {
            return self.redo_stash_swap(swap);
        }
        self.move_refs(&entry.before, &entry.after, entry.soft)
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
