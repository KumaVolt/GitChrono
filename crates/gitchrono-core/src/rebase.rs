//! Interactive rebase driven by a prepared todo list (no editor round-trips).

use std::process::Stdio;

use anyhow::{Result, bail};

use crate::repo::Repo;

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
    /// Commits after `base` up to HEAD, oldest first, ready to edit.
    /// Fails when the range contains merges (they'd be flattened).
    pub fn rebase_plan(&self, base: &str) -> Result<Vec<RebaseStep>> {
        let range = format!("{base}..HEAD");
        if !self.git(&["rev-list", "--merges", &range])?.trim().is_empty() {
            bail!("the commits after {} include merges, which interactive rebase would flatten", &base[..base.len().min(7)]);
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
        let dir = std::env::temp_dir().join(format!("gitchrono-rebase-{}-{}", std::process::id(), base));
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
            .args(["rebase", "-i", "--autostash", base])
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
}

fn shell_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', r"'\''"))
}
