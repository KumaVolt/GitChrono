//! Fetching or pulling many repositories at once (the home tab's Fetch all and
//! Pull all). One repository failing never stops the others, and a pull that
//! hits conflicts is undone: nobody is looking at that repository to finish it.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

use crate::cli::{GitError, NoUpstream, PullMode};
use crate::model::Operation;
use crate::repo::Repo;

/// Repositories worked on at the same time.
const PARALLEL: usize = 4;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SyncAction {
    Fetch,
    Pull(PullMode),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SyncOutcome {
    Done,
    /// A pull that found nothing new.
    UpToDate,
    /// Left alone, and why.
    Skipped(String),
    /// Tried without success, and why. The repository is as it was.
    Failed(String),
}

/// The line of a failure worth showing in a list.
fn reason(e: &anyhow::Error) -> String {
    let Some(git) = e.downcast_ref::<GitError>() else { return e.to_string() };
    let lines = || git.stderr.lines().map(str::trim).filter(|l| !l.is_empty());
    let line = lines().find_map(|l| l.strip_prefix("fatal: ").or(l.strip_prefix("error: "))).or(lines().next());
    line.unwrap_or("git failed").to_string()
}

fn pull(repo: &Repo, mode: PullMode) -> SyncOutcome {
    match repo.status() {
        Ok(status) if status.operation != Operation::None => return SyncOutcome::Skipped("an operation is in progress".into()),
        Ok(_) => {}
        Err(e) => return SyncOutcome::Failed(reason(&e)),
    }
    let head = || repo.ref_state().ok().and_then(|s| s.head);
    let before = head();
    let Err(e) = repo.pull(mode) else {
        return if head() == before { SyncOutcome::UpToDate } else { SyncOutcome::Done };
    };
    match e.downcast_ref::<NoUpstream>() {
        Some(NoUpstream::Detached) => return SyncOutcome::Skipped("not on a branch".into()),
        Some(NoUpstream::NoRemote) => return SyncOutcome::Skipped("no remote".into()),
        Some(_) => return SyncOutcome::Skipped("no upstream".into()),
        None => {}
    }
    match repo.status().map(|s| s.operation) {
        Ok(Operation::None) | Err(_) => SyncOutcome::Failed(reason(&e)),
        Ok(operation) => match repo.resolve_operation(operation, true) {
            Ok(()) => SyncOutcome::Failed("conflicts, so the pull was undone".into()),
            Err(abort) => SyncOutcome::Failed(format!("conflicts, and undoing the pull failed: {abort:#}")),
        },
    }
}

pub fn sync_repo(dir: &Path, action: SyncAction) -> SyncOutcome {
    let repo = match Repo::open(dir) {
        Ok(repo) => repo,
        Err(e) => return SyncOutcome::Failed(reason(&e)),
    };
    match repo.remotes() {
        Ok(remotes) if remotes.is_empty() => return SyncOutcome::Skipped("no remote".into()),
        Ok(_) => {}
        Err(e) => return SyncOutcome::Failed(reason(&e)),
    }
    match action {
        SyncAction::Fetch => repo.fetch().map_or_else(|e| SyncOutcome::Failed(reason(&e)), |()| SyncOutcome::Done),
        SyncAction::Pull(mode) => pull(&repo, mode),
    }
}

/// Run `action` on every repository, a few at a time, reporting each to `done`
/// as it finishes. Blocks until all are done.
pub fn sync_repos(dirs: &[PathBuf], action: SyncAction, done: impl Fn(&Path, SyncOutcome) + Sync) {
    let next = AtomicUsize::new(0);
    std::thread::scope(|scope| {
        for _ in 0..PARALLEL.min(dirs.len()) {
            scope.spawn(|| {
                while let Some(dir) = dirs.get(next.fetch_add(1, Ordering::Relaxed)) {
                    done(dir, sync_repo(dir, action));
                }
            });
        }
    });
}
