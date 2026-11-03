//! One file at a time: ignoring it, stashing it, saving its changes as a patch.

use std::process::Stdio;

use anyhow::{Result, bail};

use crate::model::{FileChange, FileStatus};
use crate::repo::Repo;

/// A `.gitignore` pattern, and how to describe it in a menu.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IgnoreChoice {
    pub label: String,
    pub pattern: String,
}

/// `text` as literal `.gitignore` text: wildcards, a leading `#` or `!`, and
/// trailing spaces would otherwise mean something.
fn escape(text: &str) -> String {
    let mut out = String::new();
    for (i, ch) in text.chars().enumerate() {
        if matches!(ch, '*' | '?' | '[' | '\\') || (i == 0 && matches!(ch, '#' | '!')) {
            out.push('\\');
        }
        out.push(ch);
    }
    let trailing = out.len() - out.trim_end_matches(' ').len();
    if trailing > 0 {
        out.truncate(out.len() - trailing);
        out.push_str(&"\\ ".repeat(trailing));
    }
    out
}

/// Ways to ignore `path`: the file itself, every file with its extension,
/// everything in its folder. Patterns are anchored at the repository root.
pub fn ignore_choices(path: &str) -> Vec<IgnoreChoice> {
    let (folder, name) = path.rsplit_once('/').map_or(("", path), |(f, n)| (f, n));
    let mut choices = vec![IgnoreChoice { label: format!("Ignore '{name}'"), pattern: format!("/{}", escape(path)) }];
    // `.env` has no extension: the dot starts the name.
    if let Some((_, ext)) = name.rsplit_once('.').filter(|(stem, ext)| !stem.is_empty() && !ext.is_empty()) {
        choices.push(IgnoreChoice { label: format!("All files with the extension '.{ext}'"), pattern: format!("*.{}", escape(ext)) });
    }
    if !folder.is_empty() {
        choices.push(IgnoreChoice { label: format!("All files in '{folder}/'"), pattern: format!("/{}/", escape(folder)) });
    }
    choices
}

impl Repo {
    /// Add `pattern` to the repository's top `.gitignore`, unless it's there.
    pub fn ignore(&self, pattern: &str) -> Result<()> {
        let file = self.root().join(".gitignore");
        let mut text = std::fs::read_to_string(&file).unwrap_or_default();
        if text.lines().any(|line| line.trim_end() == pattern) {
            return Ok(());
        }
        if !text.is_empty() && !text.ends_with('\n') {
            text.push('\n');
        }
        text.push_str(pattern);
        text.push('\n');
        std::fs::write(file, text)?;
        Ok(())
    }

    /// Stop tracking `path` but keep it on disk (`git rm --cached`): the next
    /// commit removes it from the repository. Refuses when its staged changes
    /// are in neither the last commit nor the file, which would lose them.
    pub fn stop_tracking(&self, path: &str) -> Result<()> {
        self.git(&["rm", "--cached", "--quiet", "--", path]).map(drop)
    }

    /// Stash just these files (untracked ones too), leaving everything else.
    pub fn stash_files(&self, paths: &[&str]) -> Result<()> {
        let message = match paths {
            [one] => one.to_string(),
            _ => format!("{} files", paths.len()),
        };
        let mut args = vec!["stash", "push", "--include-untracked", "-m", &message, "--"];
        args.extend(paths);
        self.git(&args).map(drop)
    }

    /// The file's changes as a patch `git apply` takes: staged ones, or the
    /// unstaged ones (an untracked file as all new).
    pub fn file_patch(&self, change: &FileChange, staged: bool) -> Result<String> {
        if !staged && change.status == FileStatus::Untracked {
            // `--no-index` exits 1 when the files differ, which they always do here.
            let output = crate::gitbin::git_command()
                .args(["diff", "--no-index", "--binary", "--", "/dev/null", &change.path])
                .current_dir(self.root())
                .stdin(Stdio::null())
                .output()?;
            if output.status.code() != Some(1) {
                bail!("git diff failed: {}", String::from_utf8_lossy(&output.stderr).trim());
            }
            return Ok(String::from_utf8_lossy(&output.stdout).into_owned());
        }
        let mut args = vec!["diff", "--binary", "-M"];
        if staged {
            args.push("--cached");
        }
        args.push("--");
        args.extend(change.old_path.as_deref());
        args.push(&change.path);
        self.git(&args)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn offers_the_file_its_extension_and_its_folder() {
        let choices = ignore_choices(".agents/skills/horizon/SKILL.md");
        let pairs: Vec<(&str, &str)> = choices.iter().map(|c| (c.label.as_str(), c.pattern.as_str())).collect();
        assert_eq!(
            pairs,
            [
                ("Ignore 'SKILL.md'", "/.agents/skills/horizon/SKILL.md"),
                ("All files with the extension '.md'", "*.md"),
                ("All files in '.agents/skills/horizon/'", "/.agents/skills/horizon/"),
            ]
        );
    }

    #[test]
    fn dotfiles_and_top_level_files_get_fewer_choices() {
        assert_eq!(ignore_choices(".env").len(), 1, "no extension, no folder");
        assert_eq!(ignore_choices("notes.txt").len(), 2, "no folder");
    }

    #[test]
    fn special_characters_are_taken_literally() {
        assert_eq!(ignore_choices("a[1]*.log")[0].pattern, "/a\\[1]\\*.log");
        assert_eq!(escape("#notes"), "\\#notes");
        assert_eq!(escape("!keep"), "\\!keep");
        assert_eq!(escape("odd "), "odd\\ ");
    }
}
