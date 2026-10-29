//! Cloning a remote into a new folder, with git's progress lines passed on.

use std::io::Read as _;
use std::path::Path;
use std::process::Stdio;

use anyhow::{Result, bail};

use crate::cli::GitError;

/// The folder name `git clone` would pick: the URL's last path part without
/// `.git` (`git@github.com:o/repo.git` → `repo`).
pub fn repo_name_from_url(url: &str) -> Option<String> {
    let url = url.trim().trim_end_matches(['/', '\\']);
    let url = url.strip_suffix(".git").unwrap_or(url).trim_end_matches(['/', '\\']);
    let last = url.rsplit(['/', '\\', ':']).next()?;
    (!last.is_empty()).then(|| last.to_string())
}

/// Clones `url` into `dest`, which must not exist yet (or be empty). Calls
/// `progress` with each progress line git prints ("Receiving objects: 42%…").
pub fn clone_repo(url: &str, dest: &Path, mut progress: impl FnMut(&str)) -> Result<()> {
    let (Some(parent), Some(name)) = (dest.parent(), dest.file_name()) else { bail!("not a folder path: {}", dest.display()) };
    if dest.exists() && std::fs::read_dir(dest).map_or(true, |mut d| d.next().is_some()) {
        bail!("{} already exists", dest.display());
    }
    std::fs::create_dir_all(parent)?;
    let mut child = crate::gitbin::git_command()
        .args(["clone", "--progress", "--", url.trim()])
        .arg(name)
        .current_dir(parent)
        .env("GIT_TERMINAL_PROMPT", "0")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()?;
    // Progress lines end in \r while they update in place, \n when done.
    let mut stderr = child.stderr.take().expect("piped");
    let mut all = String::new();
    let mut line = Vec::new();
    let mut buf = [0u8; 4096];
    loop {
        let n = stderr.read(&mut buf)?;
        if n == 0 {
            break;
        }
        for &b in &buf[..n] {
            if b == b'\r' || b == b'\n' {
                let text = String::from_utf8_lossy(&line);
                let text = text.trim();
                if !text.is_empty() {
                    progress(text);
                    if b == b'\n' {
                        all.push_str(text);
                        all.push('\n');
                    }
                }
                line.clear();
            } else {
                line.push(b);
            }
        }
    }
    all.push_str(String::from_utf8_lossy(&line).trim());
    let status = child.wait()?;
    if !status.success() {
        // Keep only what explains the failure, not the "Cloning into…" chatter.
        let stderr = all.lines().filter(|l| !l.starts_with("Cloning into")).collect::<Vec<_>>().join("\n").trim().to_string();
        return Err(GitError { command: format!("clone {}", url.trim()), stderr, code: status.code() }.into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_from_urls() {
        assert_eq!(repo_name_from_url("https://github.com/owner/repo.git").as_deref(), Some("repo"));
        assert_eq!(repo_name_from_url("https://github.com/owner/repo/").as_deref(), Some("repo"));
        assert_eq!(repo_name_from_url("git@github.com:owner/repo.git").as_deref(), Some("repo"));
        assert_eq!(repo_name_from_url("host:repo.git").as_deref(), Some("repo"));
        assert_eq!(repo_name_from_url("/srv/git/project.git/").as_deref(), Some("project"));
        assert_eq!(repo_name_from_url(r"C:\repos\thing").as_deref(), Some("thing"));
        assert_eq!(repo_name_from_url("  "), None);
    }
}
