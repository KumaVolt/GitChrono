//! GitChrono's git engine.
//!
//! Reads that dominate on big repos (history walk, refs, commit trees) use
//! gitoxide; writes and porcelain-exact reads shell out to `git` (see `cli`).

mod cli;
mod diff;
mod model;
mod rebase;
mod repo;
mod undo;

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
}

impl Repo {
    pub fn snapshot(&self, limit: usize) -> anyhow::Result<Snapshot> {
        let mut refs = self.refs()?;
        refs.stashes = self.stashes().unwrap_or_default();
        self.tracking(&mut refs).ok();
        let commits = self.log(&refs, limit)?;
        let status = self.status()?;
        let worktrees = self.worktrees().unwrap_or_default();
        Ok(Snapshot { refs, commits, status, worktrees })
    }
}
