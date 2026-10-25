//! Machine setup: git identity and defaults (global or per repository), SSH
//! keys and commit signing. Like `cli`, everything goes through the user's
//! own `git`, `ssh` and `ssh-keygen` so the result matches the terminal.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use anyhow::{Result, bail};

use crate::cli::git;

/// Where settings are read from and written to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Scope {
    /// `~/.gitconfig`: every repository on this machine.
    Global,
    /// `.git/config` of one repository; empty values inherit the global ones.
    Local(PathBuf),
    /// Read-only: what git actually uses in this directory, from every config
    /// file (system, global, `~/.config/git/config`, includes, local).
    Effective(PathBuf),
}

/// The settings shown in the setup dialog. Empty strings and `None` mean
/// "not set in this scope".
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct GitSetup {
    pub name: String,
    pub email: String,
    pub default_branch: String,
    pub pull_rebase: Option<bool>,
    pub sign_commits: Option<bool>,
    /// `gpg.format`: "openpgp" (default when unset), "ssh" or "x509".
    pub signing_format: String,
    pub signing_key: String,
}

const NAME: &str = "user.name";
const EMAIL: &str = "user.email";
const DEFAULT_BRANCH: &str = "init.defaultBranch";
const PULL_REBASE: &str = "pull.rebase";
const SIGN: &str = "commit.gpgsign";
const FORMAT: &str = "gpg.format";
const SIGNING_KEY: &str = "user.signingkey";

impl Scope {
    fn cwd(&self) -> PathBuf {
        match self {
            Scope::Global => std::env::temp_dir(),
            Scope::Local(repo) | Scope::Effective(repo) => repo.clone(),
        }
    }

    fn flags(&self) -> &'static [&'static str] {
        match self {
            Scope::Global => &["config", "--global"],
            Scope::Local(_) => &["config", "--local"],
            Scope::Effective(_) => &["config"],
        }
    }

    fn config(&self, args: &[&str]) -> Result<String> {
        if matches!(self, Scope::Effective(_)) && !args.contains(&"--get") {
            bail!("the effective configuration is read-only");
        }
        let all: Vec<&str> = self.flags().iter().chain(args).copied().collect();
        git(&self.cwd(), &all)
    }

    fn get(&self, key: &str) -> String {
        // Exit code 1 means "not set"; anything else unreadable is treated the same.
        self.config(&["--get", key]).map(|v| v.trim().to_string()).unwrap_or_default()
    }

    fn get_bool(&self, key: &str) -> Option<bool> {
        let out = self.config(&["--type=bool", "--get", key]).ok()?;
        Some(out.trim() == "true")
    }

    fn set(&self, key: &str, value: &str) -> Result<()> {
        if value.is_empty() {
            return self.unset(key);
        }
        self.config(&[key, value])?;
        Ok(())
    }

    fn set_bool(&self, key: &str, value: Option<bool>) -> Result<()> {
        match value {
            Some(v) => self.set(key, if v { "true" } else { "false" }),
            None => self.unset(key),
        }
    }

    fn unset(&self, key: &str) -> Result<()> {
        match self.config(&["--unset-all", key]) {
            // Exit code 5: the key wasn't set, which is what we wanted.
            Err(e) if e.downcast_ref::<crate::GitError>().is_some_and(|g| g.code == Some(5)) => Ok(()),
            other => other.map(|_| ()),
        }
    }
}

pub fn read_setup(scope: &Scope) -> GitSetup {
    GitSetup {
        name: scope.get(NAME),
        email: scope.get(EMAIL),
        default_branch: scope.get(DEFAULT_BRANCH),
        pull_rebase: scope.get_bool(PULL_REBASE),
        sign_commits: scope.get_bool(SIGN),
        signing_format: scope.get(FORMAT),
        signing_key: scope.get(SIGNING_KEY),
    }
}

/// Writes only the values that differ from `before`, so keys the dialog
/// didn't touch (or that git stores in another spelling) stay as they are.
pub fn write_setup(scope: &Scope, before: &GitSetup, after: &GitSetup) -> Result<()> {
    if after.email.contains(char::is_whitespace) {
        bail!("Email can't contain spaces");
    }
    for (key, old, new) in [
        (NAME, &before.name, &after.name),
        (EMAIL, &before.email, &after.email),
        (DEFAULT_BRANCH, &before.default_branch, &after.default_branch),
        (FORMAT, &before.signing_format, &after.signing_format),
        (SIGNING_KEY, &before.signing_key, &after.signing_key),
    ] {
        if old != new {
            scope.set(key, new.trim())?;
        }
    }
    if before.pull_rebase != after.pull_rebase {
        scope.set_bool(PULL_REBASE, after.pull_rebase)?;
    }
    if before.sign_commits != after.sign_commits {
        scope.set_bool(SIGN, after.sign_commits)?;
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SshKey {
    pub private: PathBuf,
    pub public: PathBuf,
    /// "ED25519", "RSA", …
    pub kind: String,
    pub bits: u32,
    /// "SHA256:…"
    pub fingerprint: String,
    pub comment: String,
}

/// Key pairs in `dir` (normally `~/.ssh`): every `*.pub` that `ssh-keygen`
/// can read. Sorted by file name.
pub fn ssh_keys(dir: &Path) -> Vec<SshKey> {
    let Ok(entries) = std::fs::read_dir(dir) else { return Vec::new() };
    let mut pubs: Vec<PathBuf> = entries.flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|e| e == "pub")).collect();
    pubs.sort();
    pubs.into_iter()
        .filter_map(|public| {
            let out = Command::new("ssh-keygen").arg("-lf").arg(&public).stdin(Stdio::null()).output().ok()?;
            if !out.status.success() {
                return None;
            }
            let mut key = parse_fingerprint(String::from_utf8_lossy(&out.stdout).trim())?;
            key.private = public.with_extension("");
            key.public = public;
            Some(key)
        })
        .collect()
}

/// `256 SHA256:abc… comment words (ED25519)`
fn parse_fingerprint(line: &str) -> Option<SshKey> {
    let (bits, rest) = line.split_once(' ')?;
    let (fingerprint, rest) = rest.split_once(' ').unwrap_or((rest, ""));
    let (comment, kind) = match rest.rfind(" (") {
        Some(i) if rest.ends_with(')') => (&rest[..i], &rest[i + 2..rest.len() - 1]),
        _ if rest.starts_with('(') && rest.ends_with(')') => ("", &rest[1..rest.len() - 1]),
        _ => (rest, ""),
    };
    Some(SshKey {
        private: PathBuf::new(),
        public: PathBuf::new(),
        kind: kind.to_string(),
        bits: bits.parse().ok()?,
        fingerprint: fingerprint.to_string(),
        comment: if comment == "no comment" { String::new() } else { comment.to_string() },
    })
}

/// A file name in `dir` that doesn't exist yet: `id_ed25519`, then `id_ed25519_2`, …
pub fn free_key_path(dir: &Path) -> PathBuf {
    (1..)
        .map(|n| dir.join(if n == 1 { "id_ed25519".to_string() } else { format!("id_ed25519_{n}") }))
        .find(|p| !p.exists() && !p.with_extension("pub").exists())
        .expect("unbounded range")
}

/// Creates an Ed25519 key pair at `path` (and `path.pub`).
pub fn generate_ssh_key(path: &Path, comment: &str, passphrase: &str) -> Result<SshKey> {
    if path.exists() {
        bail!("{} already exists", path.display());
    }
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))?;
        }
    }
    let out = Command::new("ssh-keygen")
        .args(["-q", "-t", "ed25519", "-C", comment, "-N", passphrase, "-f"])
        .arg(path)
        .stdin(Stdio::null())
        .output()?;
    if !out.status.success() {
        bail!("ssh-keygen failed: {}", String::from_utf8_lossy(&out.stderr).trim());
    }
    let dir = path.parent().unwrap_or(Path::new("."));
    ssh_keys(dir).into_iter().find(|k| k.private == path).ok_or_else(|| anyhow::anyhow!("key was created but can't be read"))
}

/// `ssh -T git@host`. Hosts answer with a greeting and a non-zero exit even
/// when the key works, so success is judged by the text.
pub fn test_ssh(host: &str) -> Result<String> {
    let out = Command::new("ssh")
        .args(["-T", "-o", "BatchMode=yes", "-o", "ConnectTimeout=10", "-o", "StrictHostKeyChecking=accept-new"])
        .arg(format!("git@{host}"))
        .stdin(Stdio::null())
        .output()?;
    let text = format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    let text = text.trim().to_string();
    let lower = text.to_lowercase();
    if lower.contains("successfully authenticated") || lower.contains("welcome to gitlab") || lower.contains("authenticated via") {
        Ok(text)
    } else {
        bail!("{}", if text.is_empty() { format!("ssh exited with {}", out.status) } else { text })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_fingerprints() {
        let k = parse_fingerprint("256 SHA256:abcDEF me@laptop (ED25519)").unwrap();
        assert_eq!((k.bits, k.fingerprint.as_str(), k.comment.as_str(), k.kind.as_str()), (256, "SHA256:abcDEF", "me@laptop", "ED25519"));
        let k = parse_fingerprint("3072 SHA256:xyz work key (2024) (RSA)").unwrap();
        assert_eq!((k.comment.as_str(), k.kind.as_str()), ("work key (2024)", "RSA"));
        let k = parse_fingerprint("256 SHA256:xyz no comment (ED25519)").unwrap();
        assert_eq!(k.comment, "");
    }
}
