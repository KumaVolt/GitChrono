//! A one-line overview of a repository for lists (the new-tab page):
//! branch, ahead/behind and counts of uncommitted changes, from a single
//! `git status --porcelain=v2 --branch`, plus the owner from `origin`.

use std::path::Path;

use anyhow::Result;

use crate::cli::git;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RepoSummary {
    /// Checked-out branch; `None` when HEAD is detached (see `detached_at`).
    pub branch: Option<String>,
    pub detached_at: Option<String>,
    /// Commits ahead of / behind the upstream; `None` without an upstream.
    pub ahead_behind: Option<(u32, u32)>,
    pub modified: u32,
    /// Newly added files, staged or untracked.
    pub added: u32,
    pub deleted: u32,
    pub conflicted: u32,
    /// "KumaVolt" for github.com/KumaVolt/GitChrono, from `origin`.
    pub owner: Option<String>,
}

impl RepoSummary {
    pub fn is_clean(&self) -> bool {
        self.modified + self.added + self.deleted + self.conflicted == 0
    }
}

pub fn repo_summary(dir: &Path) -> Result<RepoSummary> {
    let status = git(dir, &["status", "--porcelain=v2", "--branch", "--untracked-files=normal", "--ignore-submodules=dirty"])?;
    let mut summary = parse_status(&status);
    summary.owner = git(dir, &["config", "--get", "remote.origin.url"]).ok().and_then(|url| owner_from_url(url.trim()));
    Ok(summary)
}

pub fn parse_status(out: &str) -> RepoSummary {
    let mut s = RepoSummary::default();
    let mut oid = None;
    for line in out.lines() {
        if let Some(rest) = line.strip_prefix("# branch.oid ") {
            // "(initial)" before the first commit.
            oid = (!rest.starts_with('(')).then(|| rest.chars().take(7).collect::<String>());
        } else if let Some(rest) = line.strip_prefix("# branch.head ") {
            if rest != "(detached)" {
                s.branch = Some(rest.to_string());
            }
        } else if let Some(rest) = line.strip_prefix("# branch.ab ") {
            let mut parts = rest.split_whitespace().map(|p| p.trim_start_matches(['+', '-']).parse::<u32>().unwrap_or(0));
            s.ahead_behind = Some((parts.next().unwrap_or(0), parts.next().unwrap_or(0)));
        } else if line.starts_with("? ") {
            s.added += 1;
        } else if line.starts_with("u ") {
            s.conflicted += 1;
        } else if line.starts_with("1 ") || line.starts_with("2 ") {
            // `1 XY …` / `2 XY …` (renamed or copied); X is the index, Y the worktree.
            let xy = line.split(' ').nth(1).unwrap_or("..");
            if xy.contains('A') {
                s.added += 1;
            } else if xy.contains('D') {
                s.deleted += 1;
            } else {
                s.modified += 1;
            }
        }
    }
    if s.branch.is_none() {
        s.detached_at = oid;
    }
    s
}

/// The account or organisation in a remote URL: `https://github.com/o/r.git`,
/// `git@github.com:o/r.git`, `ssh://git@host:22/o/r`. `None` for local paths.
pub fn owner_from_url(url: &str) -> Option<String> {
    let path = if let Some((_, rest)) = url.split_once("://") {
        rest.split_once('/')?.1
    } else if let Some((host, rest)) = url.split_once(':') {
        if host.contains('/') || host.len() == 1 {
            return None; // a local path, or a Windows drive letter
        }
        rest
    } else {
        return None;
    };
    let mut parts = path.trim_matches('/').split('/').filter(|p| !p.is_empty());
    let owner = parts.next()?;
    parts.next()?; // needs a repository after the owner
    Some(owner.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_changes_and_reads_the_branch() {
        let out = "# branch.oid 1234567890abcdef\n# branch.head main\n# branch.upstream origin/main\n# branch.ab +1 -8\n\
                   1 .M N... 100644 100644 100644 a b src/a.rs\n\
                   1 M. N... 100644 100644 100644 a b src/b.rs\n\
                   1 A. N... 000000 100644 100644 a b new.rs\n\
                   1 .D N... 100644 100644 000000 a b gone.rs\n\
                   2 R. N... 100644 100644 100644 a b R100 to.rs\tfrom.rs\n\
                   u UU N... 100644 100644 100644 100644 a b c clash.rs\n\
                   ? untracked.txt\n";
        let s = parse_status(out);
        assert_eq!(s.branch.as_deref(), Some("main"));
        assert_eq!(s.ahead_behind, Some((1, 8)));
        assert_eq!((s.modified, s.added, s.deleted, s.conflicted), (3, 2, 1, 1));
        assert!(!s.is_clean());
    }

    #[test]
    fn detached_and_no_upstream() {
        let s = parse_status("# branch.oid abcdef1234\n# branch.head (detached)\n");
        assert_eq!((s.branch.as_deref(), s.detached_at.as_deref(), s.ahead_behind), (None, Some("abcdef1"), None));
        assert!(s.is_clean());
    }

    #[test]
    fn owners_from_remote_urls() {
        for (url, owner) in [
            ("https://github.com/KumaVolt/GitChrono.git", Some("KumaVolt")),
            ("git@github.com:godotengine/godot.git", Some("godotengine")),
            ("ssh://git@gitlab.example.com:2222/team/app", Some("team")),
            ("/srv/git/project.git", None),
            ("C:/repos/x", None),
            ("https://example.com/single", None),
        ] {
            assert_eq!(owner_from_url(url).as_deref(), owner, "{url}");
        }
    }
}
