//! A Fetch all or Pull all from the home tab: how far along it is, and how
//! each repository went.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use gitchrono_core::sync::SyncOutcome;

use crate::theme;

pub struct BulkSync {
    pull: bool,
    /// Every repository in the run; `None` until it finishes.
    results: HashMap<PathBuf, Option<SyncOutcome>>,
}

impl BulkSync {
    pub fn new(paths: &[PathBuf], pull: bool) -> Self {
        Self { pull, results: paths.iter().map(|p| (p.clone(), None)).collect() }
    }

    pub fn finish(&mut self, path: PathBuf, outcome: SyncOutcome) {
        self.results.insert(path, Some(outcome));
    }

    pub fn running(&self) -> bool {
        self.results.values().any(Option::is_none)
    }

    fn count(&self, matches: impl Fn(&SyncOutcome) -> bool) -> usize {
        self.results.values().flatten().filter(|o| matches(o)).count()
    }

    pub fn failed(&self) -> bool {
        self.count(|o| matches!(o, SyncOutcome::Failed(_))) > 0
    }

    /// One line on the whole run: "Pulling… 3 of 14" or "Pulled 9 · 3 skipped · 1 failed".
    pub fn summary(&self) -> String {
        let total = self.results.len();
        let finished = self.results.values().flatten().count();
        if self.running() {
            return format!("{}… {finished} of {total}", if self.pull { "Pulling" } else { "Fetching" });
        }
        let done = self.count(|o| *o == SyncOutcome::Done);
        let mut parts = vec![format!("{} {done}", if self.pull { "Pulled" } else { "Fetched" })];
        let counts = [
            (self.count(|o| *o == SyncOutcome::UpToDate), "up to date"),
            (self.count(|o| matches!(o, SyncOutcome::Skipped(_))), "skipped"),
            (self.count(|o| matches!(o, SyncOutcome::Failed(_))), "failed"),
        ];
        for (n, what) in counts {
            if n > 0 {
                parts.push(format!("{n} {what}"));
            }
        }
        parts.join(" · ")
    }

    /// What a repository's row says about this run, and in which color.
    pub fn row(&self, path: &Path) -> Option<(String, u32)> {
        Some(match self.results.get(path)? {
            None => ((if self.pull { "Pulling…" } else { "Fetching…" }).to_string(), theme::MUTED),
            Some(SyncOutcome::Done) => ((if self.pull { "✓ pulled" } else { "✓ fetched" }).to_string(), theme::ADD),
            Some(SyncOutcome::UpToDate) => ("up to date".to_string(), theme::FAINT),
            Some(SyncOutcome::Skipped(why)) => (why.clone(), theme::FAINT),
            Some(SyncOutcome::Failed(why)) => (format!("⚠ {why}"), theme::DEL),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_as_repositories_finish() {
        let (a, b, c) = (PathBuf::from("/a"), PathBuf::from("/b"), PathBuf::from("/c"));
        let mut sync = BulkSync::new(&[a.clone(), b.clone(), c.clone()], true);
        assert!(sync.running());
        assert_eq!(sync.summary(), "Pulling… 0 of 3");
        assert_eq!(sync.row(&a), Some(("Pulling…".to_string(), theme::MUTED)));
        assert_eq!(sync.row(Path::new("/elsewhere")), None);

        sync.finish(a.clone(), SyncOutcome::Done);
        sync.finish(b.clone(), SyncOutcome::UpToDate);
        assert_eq!(sync.summary(), "Pulling… 2 of 3");
        assert!(!sync.failed());

        sync.finish(c.clone(), SyncOutcome::Failed("conflicts".into()));
        assert!(!sync.running() && sync.failed());
        assert_eq!(sync.summary(), "Pulled 1 · 1 up to date · 1 failed");
        assert_eq!(sync.row(&c), Some(("⚠ conflicts".to_string(), theme::DEL)));
    }

    #[test]
    fn a_clean_fetch_reads_short() {
        let a = PathBuf::from("/a");
        let mut sync = BulkSync::new(std::slice::from_ref(&a), false);
        sync.finish(a, SyncOutcome::Done);
        assert_eq!(sync.summary(), "Fetched 1");
    }
}
