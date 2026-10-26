//! Which `git` GitChrono runs. The one on PATH wins, so the user's own setup
//! (credential manager, config) applies; Windows builds also ship MinGit in
//! `mingit/` next to the executable as a fallback. Resolved once at startup,
//! and again when the user asks to check after installing git.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::RwLock;

/// Oldest git GitChrono supports.
pub const MIN_VERSION: (u32, u32) = (2, 40);

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GitCheck {
    Ok { path: PathBuf, version: String, bundled: bool },
    TooOld { path: PathBuf, version: String },
    Missing,
}

static RESOLVED: RwLock<Option<PathBuf>> = RwLock::new(None);

/// A `Command` for the resolved git (plain `git` before `check` has run).
pub fn git_command() -> Command {
    let path = RESOLVED.read().ok().and_then(|p| p.clone());
    #[cfg_attr(not(windows), allow(unused_mut))]
    let mut cmd = Command::new(path.as_deref().unwrap_or(Path::new("git")));
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW: no console flashing up
    }
    cmd
}

/// Looks for a usable git and makes `git_command` use it.
pub fn check() -> GitCheck {
    let mut too_old = None;
    for (path, bundled) in candidates() {
        let Some(version) = version_of(&path) else { continue };
        if parse_version(&version).is_some_and(|v| v >= MIN_VERSION) {
            if let Ok(mut r) = RESOLVED.write() {
                *r = Some(path.clone());
            }
            return GitCheck::Ok { path, version, bundled };
        }
        too_old.get_or_insert(GitCheck::TooOld { path, version });
    }
    too_old.unwrap_or(GitCheck::Missing)
}

fn candidates() -> Vec<(PathBuf, bool)> {
    let mut out: Vec<(PathBuf, bool)> = Vec::new();
    // macOS apps opened from Finder get a minimal PATH; Homebrew's git
    // lives outside it.
    let extra: &[&str] = if cfg!(target_os = "macos") { &["/opt/homebrew/bin", "/usr/local/bin"] } else { &[] };
    let name = if cfg!(windows) { "git.exe" } else { "git" };
    let path = std::env::var_os("PATH").unwrap_or_default();
    for dir in std::env::split_paths(&path).chain(extra.iter().map(PathBuf::from)) {
        let candidate = dir.join(name);
        if candidate.is_file() && !out.iter().any(|(p, _)| *p == candidate) {
            out.push((candidate, false));
        }
    }
    if let Some(bundled) = bundled_git() {
        out.push((bundled, true));
    }
    out
}

/// `mingit/cmd/git.exe` next to the executable (Windows builds).
fn bundled_git() -> Option<PathBuf> {
    let dir = std::env::current_exe().ok()?.parent()?.to_path_buf();
    let git = dir.join("mingit").join("cmd").join(if cfg!(windows) { "git.exe" } else { "git" });
    git.is_file().then_some(git)
}

fn version_of(path: &Path) -> Option<String> {
    // macOS without the Command Line Tools: /usr/bin/git is a stub that
    // fails here (and would open the installer), so it counts as missing.
    let out = Command::new(path).arg("--version").stdin(Stdio::null()).stderr(Stdio::null()).output().ok()?;
    out.status.success().then(|| String::from_utf8_lossy(&out.stdout).trim().to_string()).filter(|v| v.starts_with("git version"))
}

/// `git version 2.50.1 (Apple Git-155)` → (2, 50); `2.45.2.windows.1` works too.
pub fn parse_version(text: &str) -> Option<(u32, u32)> {
    let number = text.split_whitespace().find(|w| w.chars().next().is_some_and(|c| c.is_ascii_digit()))?;
    let mut parts = number.split('.').map(|p| p.parse::<u32>());
    Some((parts.next()?.ok()?, parts.next()?.ok()?))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_versions() {
        assert_eq!(parse_version("git version 2.50.1 (Apple Git-155)"), Some((2, 50)));
        assert_eq!(parse_version("git version 2.45.2.windows.1"), Some((2, 45)));
        assert_eq!(parse_version("git version 2.39.5"), Some((2, 39)));
        assert_eq!(parse_version("nonsense"), None);
    }

    #[test]
    fn finds_the_installed_git() {
        match check() {
            GitCheck::Ok { version, bundled, .. } => assert!(version.starts_with("git version") && !bundled),
            other => panic!("tests need git {MIN_VERSION:?}+ on PATH, got {other:?}"),
        }
    }
}
