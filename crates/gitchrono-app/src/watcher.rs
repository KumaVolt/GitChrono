//! Filesystem watcher that tells the workspace what kind of refresh it needs.

use std::path::{Component, Path};

use notify::{EventKind, RecommendedWatcher, RecursiveMode, Watcher as _};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Change {
    /// Working tree or index changed: re-run `git status` only.
    Status,
    /// HEAD, refs or the reflog moved: reload history too.
    Refs,
}

fn classify(path: &Path, root: &Path) -> Option<Change> {
    let rel = path.strip_prefix(root).ok()?;
    let mut parts = rel.components().filter_map(|c| match c {
        Component::Normal(s) => s.to_str(),
        _ => None,
    });
    match parts.next()? {
        ".git" => match parts.next()? {
            "objects" | "lfs" | "fsmonitor--daemon" => None,
            name if name.ends_with(".lock") => None,
            "index" => Some(Change::Status),
            "HEAD" | "refs" | "packed-refs" | "logs" | "FETCH_HEAD" | "ORIG_HEAD" | "MERGE_HEAD"
            | "rebase-merge" | "rebase-apply" | "CHERRY_PICK_HEAD" | "REVERT_HEAD" => Some(Change::Refs),
            _ => None,
        },
        // Build output churns constantly and is almost always ignored.
        "target" | "node_modules" => None,
        _ => Some(Change::Status),
    }
}

/// Watch `root` recursively; the receiver yields one classified change per event.
pub fn watch(root: &Path) -> notify::Result<(RecommendedWatcher, smol::channel::Receiver<Change>)> {
    let (tx, rx) = smol::channel::unbounded();
    let root_owned = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
    let mut watcher = notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
        let Ok(event) = res else { return };
        if matches!(event.kind, EventKind::Access(_)) {
            return;
        }
        if let Some(change) = event.paths.iter().filter_map(|p| classify(p, &root_owned)).max() {
            tx.try_send(change).ok();
        }
    })?;
    watcher.watch(root, RecursiveMode::Recursive)?;
    Ok((watcher, rx))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_paths() {
        let root = Path::new("/r");
        assert_eq!(classify(Path::new("/r/src/main.rs"), root), Some(Change::Status));
        assert_eq!(classify(Path::new("/r/.git/index"), root), Some(Change::Status));
        assert_eq!(classify(Path::new("/r/.git/refs/heads/main"), root), Some(Change::Refs));
        assert_eq!(classify(Path::new("/r/.git/objects/ab/cd"), root), None);
        assert_eq!(classify(Path::new("/r/.git/index.lock"), root), None);
        assert_eq!(classify(Path::new("/r/target/debug/x"), root), None);
    }
}
