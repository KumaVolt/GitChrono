//! Pull requests via the GitHub CLI (`gh`), when it's installed and signed in.

use std::path::Path;
use std::process::{Command, Stdio};

use anyhow::{Result, bail};
use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PullRequest {
    pub number: u64,
    pub title: String,
    pub head_ref_name: String,
    pub url: String,
    #[serde(default)]
    pub is_draft: bool,
    #[serde(default)]
    pub author: Author,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct Author {
    #[serde(default)]
    pub login: String,
}

fn gh(root: &Path, args: &[&str]) -> Result<String> {
    let output = Command::new("gh").args(args).current_dir(root).stdin(Stdio::null()).env("GH_PROMPT_DISABLED", "1").output()?;
    if !output.status.success() {
        bail!("{}", String::from_utf8_lossy(&output.stderr).trim());
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

/// Open pull requests, or `None` when `gh` is missing, signed out, or the
/// repository isn't on GitHub.
pub fn open_pull_requests(root: &Path) -> Option<Vec<PullRequest>> {
    let out = gh(root, &["pr", "list", "--state", "open", "--limit", "50", "--json", "number,title,headRefName,url,isDraft,author"]).ok()?;
    serde_json::from_str(&out).ok()
}

pub fn checkout(root: &Path, number: u64) -> Result<()> {
    gh(root, &["pr", "checkout", &number.to_string()]).map(drop)
}

/// Start a pull request for the current branch in the browser.
pub fn create_in_browser(root: &Path) -> Result<()> {
    gh(root, &["pr", "create", "--web"]).map(drop)
}

#[cfg(test)]
mod tests {
    use super::PullRequest;

    #[test]
    fn parses_gh_json() {
        let json = r#"[{"number":7,"title":"Add search","headRefName":"feature/search","url":"https://github.com/o/r/pull/7","isDraft":true,"author":{"login":"mira"}}]"#;
        let prs: Vec<PullRequest> = serde_json::from_str(json).unwrap();
        assert_eq!((prs[0].number, prs[0].author.login.as_str(), prs[0].is_draft), (7, "mira", true));
    }
}
