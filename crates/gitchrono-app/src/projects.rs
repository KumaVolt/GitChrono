//! Remembered tabs, recent repositories and project folders, plus discovery of
//! the repositories inside a folder.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

const MAX_RECENT: usize = 15;
/// How deep below a project folder we look for repositories.
const SCAN_DEPTH: usize = 4;
const SCAN_LIMIT: usize = 1000;
const SKIP_DIRS: &[&str] = &["node_modules", "target", "vendor", "dist", "build", "Library", "Pods", "venv", "__pycache__"];

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Session {
    #[serde(default)]
    pub tabs: Vec<PathBuf>,
    #[serde(default)]
    pub active: usize,
    #[serde(default)]
    pub recent: Vec<PathBuf>,
    #[serde(default)]
    pub folders: Vec<PathBuf>,
    /// Home-screen groups the user collapsed: project folders, or "" for Recent.
    #[serde(default)]
    pub collapsed: Vec<PathBuf>,
    /// Repositories removed from a project folder's list, by folder. They stay
    /// on disk; rescans skip them. Saved as `"hidden"`, its name from when they
    /// could be shown again, so older versions agree on what's removed.
    #[serde(default, rename = "hidden")]
    pub removed: std::collections::BTreeMap<PathBuf, Vec<PathBuf>>,
    /// Show authors' profile pictures (Gravatar, which gets hashed emails);
    /// `None` means on. A new key: sessions saved before it was on by default
    /// all carry `"gravatar": false`, which is dropped.
    #[serde(default)]
    pub profile_pictures: Option<bool>,
    /// Check GitHub for new releases; `None` means on.
    #[serde(default)]
    pub auto_update: Option<bool>,
    /// Folder the last clone went into, offered for the next one.
    #[serde(default)]
    pub clone_dir: Option<PathBuf>,
}

impl Session {
    fn path() -> Option<PathBuf> {
        Some(dirs::config_dir()?.join("gitchrono").join("session.json"))
    }

    pub fn load() -> Self {
        Self::path()
            .and_then(|p| std::fs::read_to_string(p).ok())
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or_default()
    }

    pub fn save(&self) {
        let Some(path) = Self::path() else { return };
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).ok();
        }
        if let Ok(text) = serde_json::to_string_pretty(self) {
            std::fs::write(path, text).ok();
        }
    }

    pub fn gravatar_enabled(&self) -> bool {
        self.profile_pictures.unwrap_or(true)
    }

    pub fn touch_recent(&mut self, repo: &Path) {
        self.recent.retain(|p| p != repo);
        self.recent.insert(0, repo.to_path_buf());
        self.recent.truncate(MAX_RECENT);
    }

    pub fn forget_recent(&mut self, repo: &Path) {
        self.recent.retain(|p| p != repo);
    }

    pub fn is_removed(&self, folder: &Path, repo: &Path) -> bool {
        self.removed.get(folder).is_some_and(|r| r.iter().any(|p| p == repo))
    }

    /// Drops a repository from a project folder's list; nothing on disk changes.
    pub fn remove_from_folder(&mut self, folder: &Path, repo: &Path) {
        let list = self.removed.entry(folder.to_path_buf()).or_default();
        if !list.iter().any(|p| p == repo) {
            list.push(repo.to_path_buf());
        }
    }

    pub fn add_folder(&mut self, folder: &Path) {
        if !self.folders.iter().any(|f| f == folder) {
            self.folders.push(folder.to_path_buf());
        }
    }
}

/// Per-repository graph filters, keyed by repository root.
#[derive(Default, Serialize, Deserialize)]
struct RepoPrefs {
    #[serde(default)]
    hidden: std::collections::BTreeMap<String, Vec<String>>,
    #[serde(default)]
    solo: std::collections::BTreeMap<String, Vec<String>>,
    /// Sidebar sections the user collapsed ("local", "remote", "tags", …).
    #[serde(default)]
    collapsed: std::collections::BTreeMap<String, Vec<String>>,
}

fn prefs_path() -> Option<PathBuf> {
    Some(dirs::config_dir()?.join("gitchrono").join("repos.json"))
}

fn load_prefs() -> RepoPrefs {
    prefs_path().and_then(|p| std::fs::read_to_string(p).ok()).and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default()
}

pub fn load_ref_filter(root: &Path) -> gitchrono_core::RefFilter {
    let prefs = load_prefs();
    let key = root.to_string_lossy().into_owned();
    gitchrono_core::RefFilter {
        hidden: prefs.hidden.get(&key).cloned().unwrap_or_default().into_iter().collect(),
        solo: prefs.solo.get(&key).cloned().unwrap_or_default().into_iter().collect(),
    }
}

pub fn load_collapsed_sections(root: &Path) -> std::collections::BTreeSet<String> {
    load_prefs().collapsed.remove(&*root.to_string_lossy()).unwrap_or_default().into_iter().collect()
}

pub fn save_collapsed_sections(root: &Path, sections: &std::collections::BTreeSet<String>) {
    let mut prefs = load_prefs();
    let key = root.to_string_lossy().into_owned();
    if sections.is_empty() {
        prefs.collapsed.remove(&key);
    } else {
        prefs.collapsed.insert(key, sections.iter().cloned().collect());
    }
    write_prefs(&prefs);
}

fn write_prefs(prefs: &RepoPrefs) {
    let Some(path) = prefs_path() else { return };
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).ok();
    }
    if let Ok(text) = serde_json::to_string_pretty(prefs) {
        std::fs::write(path, text).ok();
    }
}

pub fn save_ref_filter(root: &Path, filter: &gitchrono_core::RefFilter) {
    let mut prefs = load_prefs();
    let key = root.to_string_lossy().into_owned();
    let set = |map: &mut std::collections::BTreeMap<String, Vec<String>>, values: &std::collections::BTreeSet<String>| {
        if values.is_empty() {
            map.remove(&key);
        } else {
            map.insert(key.clone(), values.iter().cloned().collect());
        }
    };
    set(&mut prefs.hidden, &filter.hidden);
    set(&mut prefs.solo, &filter.solo);
    write_prefs(&prefs);
}

/// A repository root: has a `.git` directory, or a `.git` file (worktrees, submodules).
pub fn is_repo(path: &Path) -> bool {
    path.join(".git").exists()
}

/// Repositories at or below `root`, sorted by path. Doesn't descend into a
/// repository once found, nor into hidden or dependency directories.
pub fn scan_repos(root: &Path) -> Vec<PathBuf> {
    let mut found = Vec::new();
    walk(root, 0, &mut found);
    found.sort_by_key(|p| p.to_string_lossy().to_lowercase());
    found
}

fn walk(dir: &Path, depth: usize, found: &mut Vec<PathBuf>) {
    if found.len() >= SCAN_LIMIT {
        return;
    }
    if is_repo(dir) {
        found.push(dir.to_path_buf());
        return;
    }
    if depth >= SCAN_DEPTH {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    let mut dirs: Vec<PathBuf> = entries
        .filter_map(Result::ok)
        .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
        .filter(|e| {
            let name = e.file_name();
            let name = name.to_string_lossy();
            !name.starts_with('.') && !SKIP_DIRS.contains(&name.as_ref())
        })
        .map(|e| e.path())
        .collect();
    dirs.sort();
    for d in dirs {
        walk(&d, depth + 1, found);
    }
}

pub fn display_name(path: &Path) -> String {
    path.file_name().map_or_else(|| path.to_string_lossy().into_owned(), |n| n.to_string_lossy().into_owned())
}

/// `~/…` for paths under the home directory.
pub fn short_path(path: &Path) -> String {
    match dirs::home_dir().and_then(|home| path.strip_prefix(home).ok().map(Path::to_path_buf)) {
        Some(rest) => format!("~/{}", rest.display()),
        None => path.display().to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_nested_repos_and_skips_noise() {
        let root = std::env::temp_dir().join(format!("gitchrono-scan-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        for dir in ["a/.git", "group/b/.git", "group/b/inner/.git", "node_modules/c/.git", ".hidden/d/.git", "empty"] {
            std::fs::create_dir_all(root.join(dir)).unwrap();
        }
        std::fs::create_dir_all(root.join("wt")).unwrap();
        std::fs::write(root.join("wt/.git"), "gitdir: elsewhere").unwrap();

        let found: Vec<String> =
            scan_repos(&root).iter().map(|p| p.strip_prefix(&root).unwrap().to_string_lossy().replace('\\', "/")).collect();
        assert_eq!(found, ["a", "group/b", "wt"]);
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn profile_pictures_are_on_unless_turned_off() {
        let old: Session = serde_json::from_str(r#"{"gravatar": false}"#).unwrap();
        assert!(old.gravatar_enabled());
        let off: Session = serde_json::from_str(r#"{"profile_pictures": false}"#).unwrap();
        assert!(!off.gravatar_enabled());
    }

    #[test]
    fn removed_repos_are_per_folder_and_survive_a_round_trip() {
        let mut s = Session::default();
        s.remove_from_folder(Path::new("/f"), Path::new("/f/a"));
        s.remove_from_folder(Path::new("/f"), Path::new("/f/a"));
        assert_eq!(s.removed[Path::new("/f")].len(), 1);
        assert!(s.is_removed(Path::new("/f"), Path::new("/f/a")));
        assert!(!s.is_removed(Path::new("/g"), Path::new("/f/a")));
        let back: Session = serde_json::from_str(&serde_json::to_string(&s).unwrap()).unwrap();
        assert_eq!(back, s);
    }

    #[test]
    fn repos_hidden_by_older_versions_stay_removed() {
        let old: Session = serde_json::from_str(r#"{"hidden": {"/f": ["/f/a"]}}"#).unwrap();
        assert!(old.is_removed(Path::new("/f"), Path::new("/f/a")));
        assert!(serde_json::to_string(&old).unwrap().contains(r#""hidden":{"/f":["/f/a"]}"#));
    }

    #[test]
    fn recent_is_deduplicated_and_capped() {
        let mut s = Session::default();
        for i in 0..20 {
            s.touch_recent(Path::new(&format!("/r{i}")));
        }
        s.touch_recent(Path::new("/r5"));
        assert_eq!(s.recent.len(), MAX_RECENT);
        assert_eq!(s.recent[0], Path::new("/r5"));
        assert_eq!(s.recent.iter().filter(|p| *p == Path::new("/r5")).count(), 1);
    }
}
