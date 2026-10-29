//! Right-click menus for commits, refs and files.

use gitchrono_core::{FileChange, FileStatus, Operation, RefKind, ResetMode};
use gpui_kit::component::menu::{PopupMenu, PopupMenuItem};
use gpui_kit::prelude::*;
use gpui_kit::*;

use crate::watcher::Change;
use crate::workspace::{Op, Workspace};

type Ws = WeakEntity<Workspace>;

fn item(
    label: impl Into<SharedString>,
    ws: &Ws,
    f: impl Fn(&mut Workspace, &mut Window, &mut Context<Workspace>) + 'static,
) -> PopupMenuItem {
    let ws = ws.clone();
    PopupMenuItem::new(label).on_click(move |_, window, cx| {
        ws.update(cx, |this, cx| f(this, window, cx)).ok();
    })
}

/// Item that runs a git operation built from cloned `arg`.
fn op_item(
    label: impl Into<SharedString>,
    busy: &'static str,
    refresh: Change,
    ws: &Ws,
    arg: String,
    op: fn(&gitchrono_core::Repo, &str) -> anyhow::Result<()>,
) -> PopupMenuItem {
    item(label, ws, move |this, _, cx| {
        let arg = arg.clone();
        this.run(busy, refresh, cx, move |r| op(r, &arg));
    })
}

fn copy_item(label: &str, ws: &Ws, text: String) -> PopupMenuItem {
    item(label.to_string(), ws, move |_, _, cx| cx.write_to_clipboard(ClipboardItem::new_string(text.clone())))
}

pub fn commit_menu(menu: PopupMenu, ws: &Ws, id: &str, current: Option<&str>) -> PopupMenu {
    let short = id[..7].to_string();
    let id = id.to_string();
    let target = current.unwrap_or("HEAD").to_string();
    let reset = |mode: ResetMode, label: &str| {
        let id = id.clone();
        let target = target.clone();
        let short = short.clone();
        item(label.to_string(), ws, move |this, window, cx| {
            let id = id.clone();
            match mode {
                ResetMode::Hard => this.confirm(
                    "Hard reset",
                    format!("Reset {target} to {short} and throw away all uncommitted changes?"),
                    "Reset",
                    "Resetting",
                    window,
                    cx,
                    move |r| r.reset(&id, mode),
                ),
                _ => this.run("Resetting", Change::Refs, cx, move |r| r.reset(&id, mode)),
            }
        })
    };

    menu.item(op_item(format!("Checkout {short}"), "Checking out", Change::Refs, ws, id.clone(), |r, id| r.checkout(id)))
        .item({
            let id = id.clone();
            item("Create branch here…", ws, move |this, window, cx| {
                let id = id.clone();
                this.open_prompt("Create branch", None, Some(""), "Create & checkout", false, window, cx, move |name| {
                    ("Creating branch".into(), Box::new(move |r: &gitchrono_core::Repo| r.create_branch(&name, Some(&id), true)) as Op)
                });
            })
        })
        .item({
            let id = id.clone();
            item("Create tag here…", ws, move |this, window, cx| {
                let id = id.clone();
                this.open_prompt("Create tag", None, Some(""), "Create tag", false, window, cx, move |name| {
                    ("Creating tag".into(), Box::new(move |r: &gitchrono_core::Repo| r.create_tag(&name, &id)) as Op)
                });
            })
        })
        .item({
            let id = id.clone();
            item("Edit message…", ws, move |this, window, cx| {
                this.select_commit_id(&id, cx);
                this.edit_message(window, cx);
            })
        })
        .separator()
        .item(op_item("Cherry-pick commit", "Cherry-picking", Change::Refs, ws, id.clone(), |r, id| r.cherry_pick(id)))
        .item(op_item("Revert commit", "Reverting", Change::Refs, ws, id.clone(), |r, id| r.revert(id)))
        .item({
            let base = id.clone();
            item("Interactive rebase from here…", ws, move |this, window, cx| this.open_rebase_editor(base.clone(), window, cx))
        })
        .separator()
        .item(reset(ResetMode::Soft, &format!("Reset {target} here — soft")))
        .item(reset(ResetMode::Mixed, &format!("Reset {target} here — mixed")))
        .item(reset(ResetMode::Hard, &format!("Reset {target} here — hard…")))
        .separator()
        .item(copy_item("Copy commit SHA", ws, id.clone()))
}

/// `filter`: (hidden, soloed) state of a branch in the graph.
pub fn ref_menu(menu: PopupMenu, ws: &Ws, name: &str, kind: RefKind, current: Option<&str>, index: usize, filter: (bool, bool)) -> PopupMenu {
    let name = name.to_string();
    let is_branch = matches!(kind, RefKind::Local | RefKind::Remote) && current != Some(name.as_str());
    let menu = if is_branch {
        let (h, s) = (name.clone(), name.clone());
        menu.item(item(if filter.0 { "Show in graph" } else { "Hide in graph" }, ws, move |this, _, cx| this.toggle_hidden(&h, cx)))
            .item(item(if filter.1 { "Stop soloing" } else { "Solo in graph" }, ws, move |this, _, cx| this.toggle_solo(&s, cx)))
            .separator()
    } else {
        menu
    };
    let cur = current.unwrap_or("HEAD").to_string();
    match kind {
        RefKind::Local | RefKind::Head => {
            let is_current = current == Some(name.as_str());
            let mut menu = menu;
            if !is_current {
                menu = menu
                    .item(op_item(format!("Checkout {name}"), "Checking out", Change::Refs, ws, name.clone(), |r, n| r.checkout(n)))
                    .item(op_item(format!("Merge {name} into {cur}"), "Merging", Change::Refs, ws, name.clone(), |r, n| r.merge(n)))
                    .item(op_item(format!("Rebase {cur} onto {name}"), "Rebasing", Change::Refs, ws, name.clone(), |r, n| r.rebase(n)))
                    .separator();
            } else {
                menu = menu
                    .item(item("Push", ws, |this, _, cx| this.push(cx)))
                    .item(item("Force push (with lease)…", ws, |this, window, cx| {
                        this.confirm("Force push", "Overwrite the remote branch with your local history?".into(), "Force push", "Pushing", window, cx, |r| r.push(true));
                    }))
                    .item(item("Pull", ws, |this, _, cx| this.run("Pulling", Change::Refs, cx, |r| r.pull(None))))
                    .separator();
            }
            menu.when(!is_current, |menu| {
                let name = name.clone();
                menu.item(item(format!("Create worktree for {name}…"), ws, move |this, window, cx| {
                    this.prompt_worktree_for_branch(name.clone(), false, window, cx)
                }))
            })
            .item({
                let name = name.clone();
                item("Rename…", ws, move |this, window, cx| {
                    let old = name.clone();
                    this.open_prompt(format!("Rename {old}"), None, Some(&old.clone()), "Rename", false, window, cx, move |new| {
                        ("Renaming".into(), Box::new(move |r: &gitchrono_core::Repo| r.rename_branch(&old, &new)) as Op)
                    });
                })
            })
            .when(!is_current, |menu| {
                let name = name.clone();
                menu.item(item(format!("Delete {name}…"), ws, move |this, window, cx| {
                    let n = name.clone();
                    this.confirm("Delete branch", format!("Delete local branch {n}? Unmerged commits need a force delete."), "Delete", "Deleting", window, cx, move |r| {
                        r.delete_branch(&n, false)
                    });
                }))
            })
            .separator()
            .item(copy_item("Copy branch name", ws, name))
        }
        RefKind::Remote => menu
            .item(op_item(format!("Checkout {name}"), "Checking out", Change::Refs, ws, name.clone(), |r, n| r.checkout_remote(n)))
            .item(op_item(format!("Merge {name} into {cur}"), "Merging", Change::Refs, ws, name.clone(), |r, n| r.merge(n)))
            .item(op_item(format!("Rebase {cur} onto {name}"), "Rebasing", Change::Refs, ws, name.clone(), |r, n| r.rebase(n)))
            .item({
                let name = name.clone();
                item(format!("Create worktree for {name}…"), ws, move |this, window, cx| {
                    this.prompt_worktree_for_branch(name.clone(), true, window, cx)
                })
            })
            .separator()
            .item({
                let name = name.clone();
                item(format!("Delete {name} from remote…"), ws, move |this, window, cx| {
                    let n = name.clone();
                    this.confirm("Delete remote branch", format!("Delete {n} on the remote for everyone?"), "Delete", "Deleting", window, cx, move |r| {
                        r.delete_remote_branch(&n)
                    });
                })
            })
            .item(copy_item("Copy branch name", ws, name)),
        RefKind::Tag => menu
            .item(op_item(format!("Checkout {name}"), "Checking out", Change::Refs, ws, name.clone(), |r, n| r.checkout(n)))
            .item(op_item("Push tag to origin", "Pushing tag", Change::Refs, ws, name.clone(), |r, n| r.push_tag(n)))
            .separator()
            .item({
                let name = name.clone();
                item(format!("Delete tag {name}…"), ws, move |this, window, cx| {
                    let n = name.clone();
                    this.confirm("Delete tag", format!("Delete local tag {n}?"), "Delete", "Deleting", window, cx, move |r| r.delete_tag(&n));
                })
            })
            .item(copy_item("Copy tag name", ws, name)),
        RefKind::Stash => menu
            .item(item("Apply stash", ws, move |this, _, cx| this.run("Applying stash", Change::Refs, cx, move |r| r.stash_apply(index))))
            .item(item("Pop stash", ws, move |this, _, cx| this.run("Popping stash", Change::Refs, cx, move |r| r.stash_pop(index))))
            .item(item("Rename stash…", ws, move |this, window, cx| this.edit_stash_message(index, window, cx)))
            .separator()
            .item(item("Drop stash…", ws, move |this, window, cx| {
                this.confirm("Drop stash", format!("Permanently drop stash@{{{index}}}?"), "Drop", "Dropping", window, cx, move |r| {
                    r.stash_drop(index)
                });
            })),
    }
}

/// Several files picked in one staging list.
pub fn files_menu(menu: PopupMenu, ws: &Ws, files: &[FileChange], staged: bool) -> PopupMenu {
    let n = files.len();
    let (pick, discard) = (files.to_vec(), files.to_vec());
    menu.item(item(&format!("{} {n} files", if staged { "Unstage" } else { "Stage" }), ws, move |this, _, cx| {
        this.stage_files(&pick, staged, cx)
    }))
    .when(!staged && files.iter().all(|f| f.status != FileStatus::Conflicted), |menu| {
        menu.item(item(&format!("Discard changes to {n} files…"), ws, move |this, window, cx| {
            let files = discard.clone();
            let untracked = files.iter().filter(|f| f.status == FileStatus::Untracked).count();
            let note = if untracked > 0 { format!(" {untracked} untracked file(s) will be deleted.") } else { String::new() };
            this.confirm("Discard changes", format!("Discard all changes to {n} files?{note} This cannot be undone."), "Discard", "Discarding", window, cx, move |r| {
                files.iter().try_for_each(|f| r.discard(f))
            });
        }))
    })
    .separator()
    .item(copy_item("Copy paths", ws, files.iter().map(|f| f.path.as_str()).collect::<Vec<_>>().join("\n")))
}

pub fn file_menu(menu: PopupMenu, ws: &Ws, change: &FileChange, staged: bool) -> PopupMenu {
    let path = change.path.clone();
    let c1 = change.clone();
    let c2 = change.clone();
    menu.item(item(if staged { "Unstage file" } else { "Stage file" }, ws, move |this, _, cx| {
        let path = c1.path.clone();
        if staged {
            this.run("Unstaging", Change::Status, cx, move |r| r.unstage(&[&path]));
        } else {
            this.run("Staging", Change::Status, cx, move |r| r.stage(&[&path]));
        }
    }))
    .when(change.status == FileStatus::Conflicted, |menu| {
        let (o, t, e) = (path.clone(), path.clone(), path.clone());
        menu.item(item("Resolve in editor", ws, move |this, _, cx| this.open_conflict(e.clone(), cx)))
            .item(item("Take ours (whole file)", ws, move |this, _, cx| {
                let p = o.clone();
                this.run("Taking ours", Change::Status, cx, move |r| r.resolve_whole_file(&p, true))
            }))
            .item(item("Take theirs (whole file)", ws, move |this, _, cx| {
                let p = t.clone();
                this.run("Taking theirs", Change::Status, cx, move |r| r.resolve_whole_file(&p, false))
            }))
            .separator()
    })
    .when(!staged && change.status != FileStatus::Conflicted, |menu| {
        menu.item(item("Discard changes…", ws, move |this, window, cx| {
            let change = c2.clone();
            let what = if change.status == FileStatus::Untracked { "Delete untracked file" } else { "Discard all changes to" };
            this.confirm("Discard changes", format!("{what} {}? This cannot be undone.", change.path), "Discard", "Discarding", window, cx, move |r| {
                r.discard(&change)
            });
        }))
    })
    .separator()
    .when(change.status != FileStatus::Untracked && change.status != FileStatus::Added, |menu| {
        history_items(menu, ws, &path)
    })
    .item(copy_item("Copy path", ws, path))
}

fn history_items(menu: PopupMenu, ws: &Ws, path: &str) -> PopupMenu {
    let h = path.to_string();
    let b = path.to_string();
    menu.item(item("File history", ws, move |this, _, cx| this.show_file_history(h.clone(), cx)))
        .item(item("Blame", ws, move |this, _, cx| this.show_blame(b.clone(), cx)))
        .separator()
}

/// Menu for a file in a commit's changed-file list.
pub fn commit_file_menu(menu: PopupMenu, ws: &Ws, path: &str) -> PopupMenu {
    history_items(menu, ws, path).item(copy_item("Copy path", ws, path.to_string()))
}

pub fn operation_name(op: Operation) -> &'static str {
    match op {
        Operation::Merge => "Merge",
        Operation::Rebase => "Rebase",
        Operation::CherryPick => "Cherry-pick",
        Operation::Revert => "Revert",
        Operation::None => "",
    }
}

pub fn worktree_menu(menu: PopupMenu, ws: &Ws, path: std::path::PathBuf, removable: bool) -> PopupMenu {
    let open = path.clone();
    let remove = path.clone();
    menu.item(item("Open in tab", ws, move |_, _, cx| cx.emit(crate::workspace::WorkspaceEvent::OpenPath(open.clone()))))
        .when(removable, |menu| {
            menu.item(item("Remove worktree…", ws, move |this, window, cx| this.remove_worktree(remove.clone(), window, cx)))
        })
        .separator()
        .item(copy_item("Copy path", ws, path.to_string_lossy().into_owned()))
}

pub fn submodule_menu(menu: PopupMenu, ws: &Ws, path: std::path::PathBuf, rel: String, copy: String) -> PopupMenu {
    menu.item(item("Open in tab", ws, move |_, _, cx| cx.emit(crate::workspace::WorkspaceEvent::OpenPath(path.clone()))))
        .item(item("Update / initialize", ws, move |this, _, cx| {
            let p = rel.clone();
            this.run("Updating submodule", Change::Refs, cx, move |r| r.update_submodule(&p))
        }))
        .separator()
        .item(copy_item("Copy path", ws, copy))
}

pub fn pr_menu(menu: PopupMenu, ws: &Ws, number: u64, url: String) -> PopupMenu {
    let open = url.clone();
    menu.item(item("Open on GitHub", ws, move |_, _, cx| cx.open_url(&open)))
        .item(item(format!("Check out #{number}"), ws, move |this, _, cx| {
            let Some(root) = this.repo.as_ref().map(|r| r.root().to_path_buf()) else { return };
            this.run("Checking out pull request", Change::Refs, cx, move |_| crate::github::checkout(&root, number))
        }))
        .separator()
        .item(copy_item("Copy link", ws, url))
}
