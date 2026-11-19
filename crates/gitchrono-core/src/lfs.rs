//! Git LFS. Commands go through the real `git lfs`, so its filters, hooks
//! and credentials behave as in the terminal; this module only finds out
//! where a repository stands and wraps the few commands the app offers.

use std::sync::RwLock;

use anyhow::Result;

use crate::repo::Repo;

/// Where a repository stands with Git LFS.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum LfsState {
    /// No `.gitattributes` sends files to LFS.
    #[default]
    NotUsed,
    /// The repository uses LFS but `git-lfs` isn't installed: its files are
    /// placeholder (pointer) files, and pushing new ones fails.
    Missing,
    /// `git-lfs` is installed but not set up for this repository (no
    /// filters configured), so files aren't downloaded or uploaded.
    NotSetUp,
    /// Working; `not_downloaded` files are still placeholders.
    Ready { not_downloaded: usize },
}

impl LfsState {
    pub fn installed(&self) -> bool {
        matches!(self, Self::NotSetUp | Self::Ready { .. })
    }
}

static INSTALLED: RwLock<Option<bool>> = RwLock::new(None);

/// Whether `git lfs` runs. Remembered; `recheck` forgets it (after the
/// user installs it).
pub fn installed() -> bool {
    if let Some(known) = INSTALLED.read().ok().and_then(|k| *k) {
        return known;
    }
    let ok = crate::gitbin::git_command()
        .args(["lfs", "version"])
        .stdin(std::process::Stdio::null())
        .output()
        .is_ok_and(|o| o.status.success());
    if let Ok(mut k) = INSTALLED.write() {
        *k = Some(ok);
    }
    ok
}

pub fn recheck() {
    if let Ok(mut k) = INSTALLED.write() {
        *k = None;
    }
}

/// An LFS pointer: what git stores in place of the file's contents.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Pointer {
    pub size: u64,
}

/// Reads a pointer file ("version https://git-lfs…", "oid sha256:…", "size N").
pub fn parse_pointer<'a>(lines: impl IntoIterator<Item = &'a str>) -> Option<Pointer> {
    let mut lines = lines.into_iter().map(str::trim).filter(|l| !l.is_empty());
    if !lines.next()?.starts_with("version https://git-lfs.github.com/spec/") {
        return None;
    }
    let mut size = None;
    for line in lines {
        if let Some(n) = line.strip_prefix("size ") {
            size = n.parse().ok();
        }
    }
    size.map(|size| Pointer { size })
}

/// Whether any of `.gitattributes`' lines sends files to LFS.
fn attributes_use_lfs(text: &str) -> bool {
    text.lines().map(str::trim).filter(|l| !l.starts_with('#')).any(|l| l.split_whitespace().any(|a| a == "filter=lfs"))
}

impl Repo {
    /// Whether this repository sends any files to LFS (a tracked or new
    /// `.gitattributes` says `filter=lfs`).
    pub fn uses_lfs(&self) -> bool {
        let listed = self.git(&["ls-files", "--cached", "--others", "--exclude-standard", "-z", "--", ":(glob)**/.gitattributes"]).unwrap_or_default();
        listed
            .split('\0')
            .filter(|p| !p.is_empty())
            .any(|p| std::fs::read_to_string(self.root().join(p)).is_ok_and(|t| attributes_use_lfs(&t)))
    }

    pub fn lfs_state(&self) -> LfsState {
        if !self.uses_lfs() {
            return LfsState::NotUsed;
        }
        if !installed() {
            return LfsState::Missing;
        }
        if !self.lfs_configured() {
            return LfsState::NotSetUp;
        }
        // "<oid> - path" is a placeholder still, "<oid> * path" the real file.
        let listed = self.git(&["lfs", "ls-files"]).unwrap_or_default();
        let not_downloaded = listed.lines().filter(|l| l.split_whitespace().nth(1) == Some("-")).count();
        LfsState::Ready { not_downloaded }
    }

    /// Sets LFS up for this repository only (filters and hooks in its own
    /// config, nothing global), then downloads its files.
    pub fn lfs_setup(&self) -> Result<()> {
        self.git(&["lfs", "install", "--local"])?;
        self.lfs_pull()
    }

    /// Downloads the LFS files of the checked-out commit and puts them in
    /// place of their placeholders.
    pub fn lfs_pull(&self) -> Result<()> {
        self.git(&["lfs", "pull"]).map(drop)
    }

    /// Deletes local copies of LFS files that older commits used and the
    /// remote has (`git lfs prune` never deletes unpushed ones).
    pub fn lfs_prune(&self) -> Result<()> {
        self.git(&["lfs", "prune"]).map(drop)
    }

    /// Sends files matching `pattern` to LFS from now on (in `.gitattributes`).
    /// Sets LFS up for this repository first when it isn't yet.
    pub fn lfs_track(&self, pattern: &str) -> Result<()> {
        if !self.lfs_configured() {
            self.git(&["lfs", "install", "--local"])?;
        }
        self.git(&["lfs", "track", "--", pattern]).map(drop)
    }

    /// `git lfs install` (globally, or `--local` here) sets these filters.
    fn lfs_configured(&self) -> bool {
        ["filter.lfs.process", "filter.lfs.smudge"].iter().any(|key| self.git(&["config", "--get", key]).is_ok_and(|v| !v.trim().is_empty()))
    }

    /// Whether `path` goes to LFS (its `filter` attribute is `lfs`).
    pub fn is_lfs_path(&self, path: &str) -> bool {
        self.git(&["check-attr", "filter", "--", path]).is_ok_and(|out| out.trim_end().ends_with(": filter: lfs"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_pointer_files() {
        let pointer = "version https://git-lfs.github.com/spec/v1\noid sha256:4d7a214614ab2935c943f9e0ff69d22eadbb8f32b1258daaa5e2ca24d17e2393\nsize 12345\n";
        assert_eq!(parse_pointer(pointer.lines()), Some(Pointer { size: 12345 }));
        assert_eq!(parse_pointer("hello\nsize 3".lines()), None);
        assert_eq!(parse_pointer("".lines()), None);
    }

    #[test]
    fn finds_lfs_in_gitattributes() {
        assert!(attributes_use_lfs("*.psd filter=lfs diff=lfs merge=lfs -text\n"));
        assert!(!attributes_use_lfs("# *.psd filter=lfs\n*.rs text eol=lf\n"));
    }
}
