//! Interactive rebase driven by a prepared todo list (no editor round-trips).

use std::process::Stdio;

use anyhow::{Context as _, Result, bail};

use crate::model::Commit;
use crate::repo::Repo;

/// What can be done to a commit by rewriting the current branch, going by the
/// graph's commits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rewritable {
    pub drop: bool,
    pub move_up: bool,
    pub move_down: bool,
}

/// `None` unless `id` is on HEAD with no merges from it up to HEAD, which a
/// rewrite would flatten. `commits` go children before parents.
pub fn rewritable(commits: &[Commit], head: &str, id: &str) -> Option<Rewritable> {
    let mut reachable = std::collections::HashSet::from([head]);
    for commit in commits {
        if !reachable.contains(commit.id.as_str()) {
            continue;
        }
        if commit.parents.len() > 1 {
            return None;
        }
        if commit.id == id {
            let newest = commit.id == head;
            let parent = commit.parents.first().and_then(|p| commits.iter().find(|c| c.id == *p));
            return Some(Rewritable {
                drop: !newest || !commit.parents.is_empty(),
                move_up: !newest,
                move_down: parent.is_some_and(|p| p.parents.len() < 2),
            });
        }
        reachable.extend(commit.parents.iter().map(String::as_str));
    }
    None
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RebaseAction {
    Pick,
    /// Keep the commit with a new message.
    Reword,
    /// Meld into the previous commit, keeping both messages.
    Squash,
    /// Meld into the previous commit, discarding this message.
    Fixup,
    Drop,
}

impl RebaseAction {
    pub const ALL: [RebaseAction; 5] = [Self::Pick, Self::Reword, Self::Squash, Self::Fixup, Self::Drop];

    pub fn label(self) -> &'static str {
        match self {
            Self::Pick => "pick",
            Self::Reword => "reword",
            Self::Squash => "squash",
            Self::Fixup => "fixup",
            Self::Drop => "drop",
        }
    }
}

#[derive(Debug, Clone)]
pub struct RebaseStep {
    pub id: String,
    pub summary: String,
    pub action: RebaseAction,
    /// New full message for [`RebaseAction::Reword`].
    pub message: String,
}

impl Repo {
    /// Commits on HEAD that `base` (a commit or branch) doesn't have, oldest
    /// first, ready to edit; an empty `base` means the whole history (`--root`).
    /// Fails when the range contains merges (they'd be flattened).
    pub fn rebase_plan(&self, base: &str) -> Result<Vec<RebaseStep>> {
        let range = if base.is_empty() { "HEAD".to_string() } else { format!("{base}..HEAD") };
        if !self.git(&["rev-list", "--merges", &range])?.trim().is_empty() {
            let shown = if base.len() == 40 { base.get(..7).unwrap_or(base) } else { base };
            bail!("the commits after {shown} include merges, which interactive rebase would flatten");
        }
        let out = self.git(&["log", "--reverse", "--format=%H%x00%s", &range])?;
        Ok(out
            .lines()
            .filter_map(|l| l.split_once('\0'))
            .map(|(id, summary)| RebaseStep {
                id: id.to_string(),
                summary: summary.to_string(),
                action: RebaseAction::Pick,
                message: summary.to_string(),
            })
            .collect())
    }

    pub fn interactive_rebase(&self, base: &str, steps: &[RebaseStep]) -> Result<()> {
        if steps.first().is_some_and(|s| matches!(s.action, RebaseAction::Squash | RebaseAction::Fixup)) {
            bail!("the first commit can't be squashed: there is nothing before it to meld into");
        }
        let dir = std::env::temp_dir().join(format!("gitchrono-rebase-{}-{}", std::process::id(), base.replace('/', "-")));
        std::fs::create_dir_all(&dir)?;
        let mut todo = String::new();
        for (i, step) in steps.iter().enumerate() {
            match step.action {
                RebaseAction::Drop => todo.push_str(&format!("drop {}\n", step.id)),
                RebaseAction::Pick => todo.push_str(&format!("pick {}\n", step.id)),
                RebaseAction::Squash => todo.push_str(&format!("squash {}\n", step.id)),
                RebaseAction::Fixup => todo.push_str(&format!("fixup {}\n", step.id)),
                RebaseAction::Reword => {
                    let message = dir.join(format!("msg-{i}"));
                    std::fs::write(&message, &step.message)?;
                    todo.push_str(&format!("pick {}\n", step.id));
                    todo.push_str(&format!("exec git commit --amend --only --allow-empty -F {}\n", shell_quote(&message.to_string_lossy())));
                }
            }
        }
        let todo_path = dir.join("todo");
        std::fs::write(&todo_path, todo)?;
        let output = crate::gitbin::git_command()
            .args(["rebase", "-i", "--autostash", if base.is_empty() { "--root" } else { base }])
            .current_dir(self.root())
            // git runs `$GIT_SEQUENCE_EDITOR <todo file>`: overwrite it with ours.
            .env("GIT_SEQUENCE_EDITOR", format!("cp {}", shell_quote(&todo_path.to_string_lossy())))
            // Squash messages: accept git's combined message as is.
            .env("GIT_EDITOR", "true")
            .env("GIT_TERMINAL_PROMPT", "0")
            .stdin(Stdio::null())
            .output()?;
        let _ = std::fs::remove_dir_all(&dir);
        if !output.status.success() {
            bail!("rebase stopped: {}", String::from_utf8_lossy(&output.stderr).trim());
        }
        Ok(())
    }

    /// Give `commit` a new message. HEAD is amended in place (staged changes
    /// stay staged, not committed); an older commit on the current branch is
    /// reworded by rebasing the commits after it, which get new ids.
    /// Returns the reworded commit's new id.
    pub fn reword(&self, commit: &str, message: &str) -> Result<String> {
        let message = message.trim();
        if message.is_empty() {
            bail!("commit message is empty");
        }
        let id = self.git(&["rev-parse", "--verify", &format!("{commit}^{{commit}}")])?.trim().to_string();
        let short = &id[..id.len().min(7)];
        // `<id> <parent>...`
        let parents: Vec<String> = self.git(&["rev-list", "--parents", "-n1", &id])?.split_whitespace().skip(1).map(str::to_string).collect();
        if parents.len() > 1 {
            bail!("{short} is a merge commit; rewording it would flatten the merge");
        }
        let head = self.git(&["rev-parse", "-q", "--verify", "HEAD"])?.trim().to_string();
        if id == head {
            self.git(&["commit", "--amend", "--only", "--allow-empty", "-m", message])?;
            return Ok(self.git(&["rev-parse", "HEAD"])?.trim().to_string());
        }
        if self.git(&["merge-base", "--is-ancestor", &id, "HEAD"]).is_err() {
            bail!("{short} isn't on the current branch; check it out to reword it");
        }
        let base = parents.first().map_or("", String::as_str);
        let mut plan = self.rebase_plan(base)?;
        let step = plan.iter_mut().find(|s| s.id == id).context("commit not found in the rebase plan")?;
        step.action = RebaseAction::Reword;
        step.message = message.to_string();
        let after = plan.iter().rev().position(|s| s.id == id).unwrap_or_default();
        self.interactive_rebase(base, &plan)?;
        Ok(self.git(&["rev-parse", &format!("HEAD~{after}")])?.trim().to_string())
    }

    /// Remove `commit` from the current branch; the commits after it get new ids.
    pub fn drop_commit(&self, commit: &str) -> Result<()> {
        let (id, parent) = self.commit_on_head(commit)?;
        let mut plan = self.rebase_plan(&parent)?;
        if plan.len() == 1 && parent.is_empty() {
            bail!("{} is the only commit, so there would be nothing left", &id[..7]);
        }
        plan.iter_mut().find(|s| s.id == id).context("commit not found in the rebase plan")?.action = RebaseAction::Drop;
        self.interactive_rebase(&parent, &plan)
    }

    /// Swap `commit` with the commit after it (`up`) or before it.
    pub fn move_commit(&self, commit: &str, up: bool) -> Result<()> {
        let (id, parent) = self.commit_on_head(commit)?;
        let older = if up { id.clone() } else { parent };
        if older.is_empty() {
            bail!("{} is the first commit, so there is nothing before it", &id[..7]);
        }
        let (_, base) = self.commit_on_head(&older)?;
        let mut plan = self.rebase_plan(&base)?;
        let at = plan.iter().position(|s| s.id == older).context("commit not found in the rebase plan")?;
        if at + 1 == plan.len() {
            bail!("{} is the latest commit, so there is nothing after it", &id[..7]);
        }
        plan.swap(at, at + 1);
        self.interactive_rebase(&base, &plan)
    }

    /// The full id of `commit`, which must be on HEAD and no merge, and its
    /// parent's (empty for a root commit).
    fn commit_on_head(&self, commit: &str) -> Result<(String, String)> {
        let id = self.git(&["rev-parse", "--verify", &format!("{commit}^{{commit}}")])?.trim().to_string();
        let parents: Vec<String> = self.git(&["rev-list", "--parents", "-n1", &id])?.split_whitespace().skip(1).map(str::to_string).collect();
        if parents.len() > 1 {
            bail!("{} is a merge commit; rewriting it would flatten the merge", &id[..7]);
        }
        if self.git(&["merge-base", "--is-ancestor", &id, "HEAD"]).is_err() {
            bail!("{} isn't on the current branch; check it out first", &id[..7]);
        }
        Ok((id, parents.into_iter().next().unwrap_or_default()))
    }

    /// Whether a remote-tracking branch already contains `commit`, so
    /// rewriting it means a force push.
    pub fn is_pushed(&self, commit: &str) -> Result<bool> {
        Ok(!self.git(&["for-each-ref", "--contains", commit, "--format=%(refname)", "refs/remotes"])?.trim().is_empty())
    }
}

fn shell_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', r"'\''"))
}
