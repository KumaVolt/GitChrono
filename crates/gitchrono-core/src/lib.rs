//! GitChrono's git engine.
//!
//! Reads that dominate on big repos (history walk, refs, commit trees) use
//! gitoxide; writes and porcelain-exact reads shell out to `git` (see `cli`).

mod cli;
mod conflict;
mod diff;
mod model;
mod rebase;
mod repo;
pub mod setup;
pub mod summary;
mod undo;

pub use conflict::{Chunk, Conflict, ConflictFile, Resolution};
pub use cli::{GitError, ResetMode, git};
pub use diff::{ApplyMode, parse_patch, partial_patch};
pub use model::*;
pub use rebase::{RebaseAction, RebaseStep};
pub use repo::Repo;
pub use undo::{RefState, UndoEntry};

/// Everything the main view needs, loaded in one go off the UI thread.
#[derive(Debug, Clone, Default)]
pub struct Snapshot {
    pub refs: Refs,
    pub commits: Vec<Commit>,
    pub status: Status,
    pub worktrees: Vec<Worktree>,
    pub submodules: Vec<Submodule>,
}

/// Which branches the graph walks. The checked-out branch always shows.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RefFilter {
    /// Branch names (`main`, `origin/main`) left out of the graph.
    pub hidden: std::collections::BTreeSet<String>,
    /// When non-empty, only these branches are walked (tags and stashes too are left out).
    pub solo: std::collections::BTreeSet<String>,
}

impl RefFilter {
    pub fn is_empty(&self) -> bool {
        self.hidden.is_empty() && self.solo.is_empty()
    }

    pub fn shows(&self, name: &str, current: bool) -> bool {
        current || if self.solo.is_empty() { !self.hidden.contains(name) } else { self.solo.contains(name) }
    }

    /// The refs the graph is built from.
    pub fn apply(&self, refs: &Refs) -> Refs {
        if self.is_empty() {
            return refs.clone();
        }
        let mut walked = refs.clone();
        walked.local.retain(|b| self.shows(&b.name, b.current));
        walked.remote.retain(|b| self.shows(&b.name, false));
        if !self.solo.is_empty() {
            walked.tags.clear();
            walked.stashes.clear();
        }
        walked
    }
}

impl Repo {
    pub fn snapshot(&self, limit: usize) -> anyhow::Result<Snapshot> {
        self.snapshot_filtered(limit, &RefFilter::default())
    }

    pub fn snapshot_filtered(&self, limit: usize, filter: &RefFilter) -> anyhow::Result<Snapshot> {
        let mut refs = self.refs()?;
        refs.stashes = self.stashes().unwrap_or_default();
        self.tracking(&mut refs).ok();
        let commits = self.log(&filter.apply(&refs), limit)?;
        let status = self.status()?;
        let worktrees = self.worktrees().unwrap_or_default();
        let submodules = self.submodules().unwrap_or_default();
        Ok(Snapshot { refs, commits, status, worktrees, submodules })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn branch(name: &str, current: bool) -> Branch {
        Branch { name: name.into(), full_name: format!("refs/heads/{name}"), target: "0".repeat(40), current, upstream: None, ahead: 0, behind: 0 }
    }

    #[test]
    fn filter_keeps_current_branch() {
        let refs = Refs { local: vec![branch("main", true), branch("wip", false), branch("old", false)], ..Default::default() };
        let mut filter = RefFilter::default();
        filter.hidden.insert("old".into());
        filter.hidden.insert("main".into());
        let names: Vec<String> = filter.apply(&refs).local.into_iter().map(|b| b.name).collect();
        assert_eq!(names, ["main", "wip"]);

        let mut solo = RefFilter::default();
        solo.solo.insert("old".into());
        let names: Vec<String> = solo.apply(&refs).local.into_iter().map(|b| b.name).collect();
        assert_eq!(names, ["main", "old"]);
    }
}
