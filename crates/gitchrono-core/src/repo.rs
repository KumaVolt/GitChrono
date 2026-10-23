//! Read path: history, refs and commit file lists come straight from gix.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result};
use gix::traverse::commit::topo;

use crate::model::*;

pub struct Repo {
    root: PathBuf,
    git: gix::ThreadSafeRepository,
}

impl Repo {
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let git = gix::discover(path.as_ref())
            .with_context(|| format!("not a git repository: {}", path.as_ref().display()))?
            .into_sync();
        let root = git
            .work_dir()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| git.git_dir().to_path_buf());
        Ok(Self { root, git })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn name(&self) -> String {
        self.root
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| self.root.display().to_string())
    }

    fn local(&self) -> gix::Repository {
        self.git.to_thread_local()
    }

    pub fn refs(&self) -> Result<Refs> {
        let repo = self.local();
        let mut out = Refs {
            head_branch: repo
                .head_name()?
                .map(|n| n.shorten().to_string()),
            head_id: repo.head_id().ok().map(|id| id.to_string()),
            ..Default::default()
        };

        let platform = repo.references()?;
        for reference in platform.all()? {
            let Ok(mut reference) = reference else { continue };
            let full = reference.name().as_bstr().to_string();
            let Ok(id) = reference.peel_to_id() else { continue };
            let target = id.to_string();
            if let Some(name) = full.strip_prefix("refs/heads/") {
                let current = out.head_branch.as_deref() == Some(name);
                out.local.push(Branch {
                    name: name.to_string(),
                    full_name: full.clone(),
                    target,
                    current,
                    upstream: None,
                    ahead: 0,
                    behind: 0,
                });
            } else if let Some(name) = full.strip_prefix("refs/remotes/") {
                if name.ends_with("/HEAD") {
                    continue;
                }
                out.remote.push(Branch {
                    name: name.to_string(),
                    full_name: full.clone(),
                    target,
                    current: false,
                    upstream: None,
                    ahead: 0,
                    behind: 0,
                });
            } else if let Some(name) = full.strip_prefix("refs/tags/") {
                out.tags.push(Tag { name: name.to_string(), target });
            }
        }
        out.tags.sort_by(|a, b| b.name.cmp(&a.name));
        Ok(out)
    }

    /// Walk every ref in `--topo-order` (children before parents), up to `limit` commits.
    pub fn log(&self, refs: &Refs, limit: usize) -> Result<Vec<Commit>> {
        let repo = self.local();

        let mut labels: HashMap<String, Vec<RefLabel>> = HashMap::new();
        let mut tips: Vec<gix::ObjectId> = Vec::new();
        let mut add = |target: &str, label: RefLabel| {
            if let Ok(id) = gix::ObjectId::from_hex(target.as_bytes()) {
                tips.push(id);
            }
            labels.entry(target.to_string()).or_default().push(label);
        };
        if let Some(head) = &refs.head_id {
            if refs.head_branch.is_none() {
                add(head, RefLabel { name: "HEAD".into(), kind: RefKind::Head, current: true });
            }
        }
        for b in &refs.local {
            add(&b.target, RefLabel { name: b.name.clone(), kind: RefKind::Local, current: b.current });
        }
        for b in &refs.remote {
            add(&b.target, RefLabel { name: b.name.clone(), kind: RefKind::Remote, current: false });
        }
        for t in &refs.tags {
            add(&t.target, RefLabel { name: t.name.clone(), kind: RefKind::Tag, current: false });
        }
        for s in &refs.stashes {
            add(&s.target, RefLabel { name: format!("stash@{{{}}}", s.index), kind: RefKind::Stash, current: false });
        }
        tips.sort();
        tips.dedup();
        // Tags may point at blobs or trees (git.git has one); only commits can be walked.
        tips.retain(|id| repo.find_header(*id).is_ok_and(|h| h.kind() == gix::object::Kind::Commit));
        if tips.is_empty() {
            return Ok(Vec::new());
        }

        let walk = topo::Builder::new(&repo.objects)
            .with_tips(tips)
            .sorting(topo::Sorting::TopoOrder)
            .with_commit_graph(repo.commit_graph_if_enabled().ok().flatten())
            .build()?;

        let mut commits = Vec::with_capacity(limit.min(4096));
        // A stash is a merge of HEAD with helper commits (index, untracked files).
        // Show it as a single node on its base: drop the helpers and their edges.
        let stash_ids: std::collections::HashSet<&str> = refs.stashes.iter().map(|s| s.target.as_str()).collect();
        let mut hidden: std::collections::HashSet<gix::ObjectId> = Default::default();

        for info in walk {
            if commits.len() >= limit {
                break;
            }
            let info = info?;
            if hidden.remove(&info.id) {
                continue;
            }
            let id = info.id.to_string();
            let mut parents: Vec<String> = info.parent_ids.iter().map(|p| p.to_string()).collect();
            if stash_ids.contains(id.as_str()) {
                hidden.extend(info.parent_ids.iter().skip(1).copied());
                parents.truncate(1);
            }
            let commit = repo.find_commit(info.id)?;
            let decoded = commit.decode()?;
            let author = decoded.author()?;
            let message = decoded.message();
            commits.push(Commit {
                short_id: id[..7].to_string(),
                parents,
                author_name: author.name.to_string(),
                author_email: author.email.to_string(),
                time: author.time()?.seconds,
                summary: message.summary().to_string(),
                body: message.body.map(|b| b.to_string()).unwrap_or_default(),
                refs: labels.remove(&id).unwrap_or_default(),
                id,
            });
        }
        Ok(commits)
    }

    /// Files changed by `commit` relative to its first parent.
    pub fn commit_files(&self, commit: &str) -> Result<Vec<FileChange>> {
        use gix::object::tree::diff::Change;

        let repo = self.local();
        let commit = repo.find_commit(gix::ObjectId::from_hex(commit.as_bytes())?)?;
        let tree = commit.tree()?;
        let parent_tree = match commit.parent_ids().next() {
            Some(p) => p.object()?.into_commit().tree()?,
            None => repo.empty_tree(),
        };

        let mut files = Vec::new();
        parent_tree
            .changes()?
            .options(|o| {
                o.track_rewrites(Some(Default::default()));
            })
            .for_each_to_obtain_tree(&tree, |change| {
                if !change.entry_mode().is_blob_or_symlink() {
                    return Ok(std::ops::ControlFlow::Continue(()));
                }
                let path = change.location().to_string();
                files.push(match change {
                    Change::Addition { .. } => FileChange { path, old_path: None, status: FileStatus::Added },
                    Change::Deletion { .. } => FileChange { path, old_path: None, status: FileStatus::Deleted },
                    Change::Modification { .. } => FileChange { path, old_path: None, status: FileStatus::Modified },
                    Change::Rewrite { source_location, copy, .. } => FileChange {
                        path,
                        old_path: Some(source_location.to_string()),
                        status: if copy { FileStatus::Copied } else { FileStatus::Renamed },
                    },
                });
                Ok(std::ops::ControlFlow::Continue(()))
            })?;
        files.sort_by(|a, b| a.path.cmp(&b.path));
        Ok(files)
    }
}
