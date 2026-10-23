//! Repository window state and the actions that mutate it. Rendering lives in
//! `views`, `diff_view` and `menus`.

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use gitchrono_core::{ApplyMode, Commit, FileChange, FileDiff, FileStatus, Repo, Snapshot, Status};
use gitchrono_graph::Graph;
use gpui_kit::component::input::{InputEvent, InputState, TextareaState};
use gpui_kit::*;

use crate::watcher::{self, Change};

const LOG_LIMIT: usize = 50_000;

pub type Op = Box<dyn FnOnce(&Repo) -> Result<()> + Send>;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Selection {
    None,
    Wip,
    Commit(usize),
}

#[derive(Clone)]
pub enum DiffSource {
    Commit { id: String, path: String },
    Workdir { change: FileChange, staged: bool },
}

impl DiffSource {
    /// `Some(staged)` when hunks/lines can be staged, unstaged or discarded.
    pub fn partial(&self) -> Option<bool> {
        match self {
            DiffSource::Workdir { change, staged } if change.status != FileStatus::Untracked => Some(*staged),
            _ => None,
        }
    }
}

pub struct DiffView {
    pub title: SharedString,
    pub source: DiffSource,
    pub diff: FileDiff,
    /// `(hunk, line)` pairs picked for line-level staging.
    pub selected: HashSet<(usize, usize)>,
}

pub struct Prompt {
    pub title: SharedString,
    pub message: Option<SharedString>,
    pub input: Option<Entity<InputState>>,
    pub confirm: SharedString,
    pub danger: bool,
    action: Option<Box<dyn FnOnce(String) -> (String, Op)>>,
    _subscription: Option<Subscription>,
}

pub struct Workspace {
    pub repo: Option<Arc<Repo>>,
    pub snap: Arc<Snapshot>,
    pub status: Status,
    pub graph: Arc<Graph>,
    pub selection: Selection,
    pub commit_files: Vec<FileChange>,
    pub diff: Option<DiffView>,
    pub prompt: Option<Prompt>,
    pub busy: Option<SharedString>,
    pub error: Option<SharedString>,
    pub summary: Entity<InputState>,
    pub description: Entity<TextareaState>,
    pub amend: bool,
    pub list_scroll: UniformListScrollHandle,
    /// `GITCHRONO_SELECT=<row>[:diff]`, applied once after the first load (for headless screenshots).
    debug_select: Option<(usize, bool)>,
    load_task: Option<Task<()>>,
    status_task: Option<Task<()>>,
    _watcher: Option<(notify::RecommendedWatcher, Task<()>)>,
}

impl Workspace {
    pub fn new(path: Option<PathBuf>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let summary = cx.new(|cx| InputState::new(window, cx).placeholder("Commit summary"));
        let description = cx.new(|cx| TextareaState::new(window, cx).placeholder("Description"));
        let mut this = Self {
            repo: None,
            snap: Arc::default(),
            status: Status::default(),
            graph: Arc::default(),
            selection: Selection::None,
            commit_files: Vec::new(),
            diff: None,
            prompt: None,
            busy: None,
            error: None,
            summary,
            description,
            amend: false,
            list_scroll: UniformListScrollHandle::new(),
            debug_select: std::env::var("GITCHRONO_SELECT").ok().and_then(|v| {
                let (row, diff) = v.split_once(':').map_or((v.as_str(), false), |(r, _)| (r, true));
                Some((row.parse().ok()?, diff))
            }),
            load_task: None,
            status_task: None,
            _watcher: None,
        };
        if let Some(path) = path {
            this.open(path, cx);
        }
        this
    }

    pub fn open(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        match Repo::open(&path) {
            Ok(repo) => {
                let repo = Arc::new(repo);
                self._watcher = self.start_watcher(&repo, cx);
                self.repo = Some(repo);
                self.selection = Selection::None;
                self.diff = None;
                self.reload(cx);
            }
            Err(e) => self.error = Some(format!("{e:#}").into()),
        }
        cx.notify();
    }

    fn start_watcher(&self, repo: &Repo, cx: &mut Context<Self>) -> Option<(notify::RecommendedWatcher, Task<()>)> {
        let (watcher, rx) = watcher::watch(repo.root()).ok()?;
        let task = cx.spawn(async move |this, cx| {
            while let Ok(first) = rx.recv().await {
                // Coalesce bursts (checkouts, builds) into one refresh.
                cx.background_executor().timer(Duration::from_millis(250)).await;
                let mut change = first;
                while let Ok(next) = rx.try_recv() {
                    change = change.max(next);
                }
                let alive = this
                    .update(cx, |this, cx| {
                        if this.busy.is_none() {
                            match change {
                                Change::Status => this.refresh_status(cx),
                                Change::Refs => this.reload(cx),
                            }
                        }
                    })
                    .is_ok();
                if !alive {
                    break;
                }
            }
        });
        Some((watcher, task))
    }

    pub fn prompt_open(&mut self, cx: &mut Context<Self>) {
        let paths = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some("Open Repository".into()),
        });
        cx.spawn(async move |this, cx| {
            if let Ok(Ok(Some(mut paths))) = paths.await {
                if let Some(path) = paths.pop() {
                    this.update(cx, |this, cx| this.open(path, cx)).ok();
                }
            }
        })
        .detach();
    }

    pub fn has_wip(&self) -> bool {
        !self.status.is_clean()
    }

    pub fn row_count(&self) -> usize {
        self.snap.commits.len() + usize::from(self.has_wip())
    }

    /// Reload refs, history and status off the main thread, keeping the selection.
    pub fn reload(&mut self, cx: &mut Context<Self>) {
        let Some(repo) = self.repo.clone() else { return };
        let selected_id = match self.selection {
            Selection::Commit(i) => self.snap.commits.get(i).map(|c| c.id.clone()),
            _ => None,
        };
        let task = cx.background_spawn(async move {
            let snap = repo.snapshot(LOG_LIMIT)?;
            let graph = gitchrono_graph::layout(
                snap.commits.iter().map(|c| (c.id.as_str(), c.parents.iter().map(String::as_str))),
            );
            anyhow::Ok((snap, graph))
        });
        self.load_task = Some(cx.spawn(async move |this, cx| {
            let result = task.await;
            this.update(cx, |this, cx| {
                match result {
                    Ok((mut snap, graph)) => {
                        this.status = std::mem::take(&mut snap.status);
                        this.snap = Arc::new(snap);
                        this.graph = Arc::new(graph);
                        this.selection = match selected_id {
                            Some(id) => this
                                .snap
                                .commits
                                .iter()
                                .position(|c| c.id == id)
                                .map_or(Selection::None, Selection::Commit),
                            None if this.selection == Selection::Wip && !this.has_wip() => Selection::None,
                            None => this.selection,
                        };
                        if let Selection::Commit(i) = this.selection {
                            this.load_commit_files(i, cx);
                        }
                        if let Some((row, diff)) = this.debug_select.take() {
                            this.select_row(row, cx);
                            if diff {
                                this.debug_open_first_diff(cx);
                            }
                        }
                    }
                    Err(e) => this.error = Some(format!("{e:#}").into()),
                }
                cx.notify();
            })
            .ok();
        }));
    }

    /// Cheap refresh for working-tree edits: `git status` only, plus the open diff.
    pub fn refresh_status(&mut self, cx: &mut Context<Self>) {
        let Some(repo) = self.repo.clone() else { return };
        let diff_source = self.diff.as_ref().map(|d| d.source.clone());
        let task = cx.background_spawn(async move {
            let status = repo.status()?;
            let diff = match diff_source {
                Some(DiffSource::Workdir { change, staged }) => Some(repo.workdir_file_diff(&change, staged)),
                _ => None,
            };
            anyhow::Ok((status, diff))
        });
        self.status_task = Some(cx.spawn(async move |this, cx| {
            let result = task.await;
            this.update(cx, |this, cx| {
                match result {
                    Ok((status, diff)) => {
                        this.status = status;
                        if this.selection == Selection::Wip && !this.has_wip() {
                            this.selection = Selection::None;
                        }
                        if let Some(Ok(diff)) = diff {
                            if diff.hunks.is_empty() && !diff.binary {
                                this.diff = None;
                            } else if let Some(view) = this.diff.as_mut() {
                                view.diff = diff;
                                view.selected.clear();
                            }
                        }
                    }
                    Err(e) => this.error = Some(format!("{e:#}").into()),
                }
                cx.notify();
            })
            .ok();
        }));
    }

    /// Run a git operation in the background, then refresh.
    pub fn run(
        &mut self,
        label: impl Into<String>,
        refresh: Change,
        cx: &mut Context<Self>,
        op: impl FnOnce(&Repo) -> Result<()> + Send + 'static,
    ) {
        let Some(repo) = self.repo.clone() else { return };
        if self.busy.is_some() {
            return;
        }
        self.busy = Some(format!("{}…", label.into()).into());
        self.error = None;
        cx.notify();
        let task = cx.background_spawn(async move { op(&repo) });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            this.update(cx, |this, cx| {
                this.busy = None;
                if let Err(e) = result {
                    this.error = Some(format!("{e:#}").into());
                }
                match refresh {
                    Change::Status => this.refresh_status(cx),
                    Change::Refs => this.reload(cx),
                }
            })
            .ok();
        })
        .detach();
    }

    pub fn select_row(&mut self, row: usize, cx: &mut Context<Self>) {
        let wip = self.has_wip();
        self.selection = if wip && row == 0 { Selection::Wip } else { Selection::Commit(row - usize::from(wip)) };
        self.diff = None;
        if let Selection::Commit(i) = self.selection {
            self.load_commit_files(i, cx);
        }
        cx.notify();
    }

    pub fn select_commit_id(&mut self, id: &str, cx: &mut Context<Self>) {
        if let Some(i) = self.snap.commits.iter().position(|c| c.id == id) {
            let row = i + usize::from(self.has_wip());
            self.select_row(row, cx);
            self.list_scroll.scroll_to_item(row, ScrollStrategy::Center);
        }
    }

    fn load_commit_files(&mut self, index: usize, cx: &mut Context<Self>) {
        let (Some(repo), Some(commit)) = (self.repo.clone(), self.snap.commits.get(index)) else {
            return;
        };
        let id = commit.id.clone();
        let task = cx.background_spawn(async move { repo.commit_files(&id) });
        cx.spawn(async move |this, cx| {
            let files = task.await;
            this.update(cx, |this, cx| {
                if this.selection == Selection::Commit(index) {
                    this.commit_files = files.unwrap_or_default();
                    cx.notify();
                }
            })
            .ok();
        })
        .detach();
    }

    pub fn show_diff(&mut self, title: String, source: DiffSource, cx: &mut Context<Self>) {
        let Some(repo) = self.repo.clone() else { return };
        let load = source.clone();
        let task = cx.background_spawn(async move {
            match load {
                DiffSource::Commit { id, path } => repo.commit_file_diff(&id, &path),
                DiffSource::Workdir { change, staged } => repo.workdir_file_diff(&change, staged),
            }
        });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            this.update(cx, |this, cx| {
                match result {
                    Ok(diff) => this.diff = Some(DiffView { title: title.into(), source, diff, selected: HashSet::new() }),
                    Err(e) => this.error = Some(format!("{e:#}").into()),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    pub fn show_workdir_diff(&mut self, change: FileChange, staged: bool, cx: &mut Context<Self>) {
        let title = format!("{} ({})", change.path, if staged { "staged" } else { "unstaged" });
        self.show_diff(title, DiffSource::Workdir { change, staged }, cx);
    }

    /// Stage, unstage or discard `hunks` of the open diff, limited to the
    /// selected lines when `lines_only`.
    pub fn apply_partial(&mut self, hunks: Vec<usize>, lines_only: bool, discard: bool, cx: &mut Context<Self>) {
        let Some(view) = &self.diff else { return };
        let Some(staged) = view.source.partial() else { return };
        let mode = match (staged, discard) {
            (_, true) => ApplyMode::Discard,
            (true, false) => ApplyMode::Unstage,
            (false, false) => ApplyMode::Stage,
        };
        let selected = lines_only.then_some(&view.selected);
        let Some(patch) = gitchrono_core::partial_patch(&view.diff, &hunks, selected, mode) else { return };
        let label = match mode {
            ApplyMode::Stage => "Staging",
            ApplyMode::Unstage => "Unstaging",
            ApplyMode::Discard => "Discarding",
        };
        self.run(label, Change::Status, cx, move |r| r.apply_patch(&patch, mode));
    }

    pub fn toggle_line(&mut self, hunk: usize, line: usize, cx: &mut Context<Self>) {
        if let Some(view) = &mut self.diff {
            if !view.selected.remove(&(hunk, line)) {
                view.selected.insert((hunk, line));
            }
            cx.notify();
        }
    }

    pub fn commit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let summary = self.summary.read(cx).value().trim().to_string();
        let body = self.description.read(cx).value().trim().to_string();
        let message = if body.is_empty() { summary } else { format!("{summary}\n\n{body}") };
        let amend = self.amend;
        self.summary.update(cx, |s, cx| s.set_value("", window, cx));
        self.description.update(cx, |s, cx| s.set_value("", window, cx));
        self.amend = false;
        self.run("Committing", Change::Refs, cx, move |repo| repo.commit(&message, amend));
    }

    pub fn selected_commit(&self) -> Option<&Commit> {
        match self.selection {
            Selection::Commit(i) => self.snap.commits.get(i),
            _ => None,
        }
    }

    // ------------------------------------------------------------ prompts

    /// Ask for a name (when `default` is `Some`) or just a confirmation, then
    /// run the operation `action` builds from the entered text.
    #[allow(clippy::too_many_arguments)]
    pub fn open_prompt(
        &mut self,
        title: impl Into<SharedString>,
        message: Option<String>,
        default: Option<&str>,
        confirm: &str,
        danger: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
        action: impl FnOnce(String) -> (String, Op) + 'static,
    ) {
        let (input, subscription) = match default {
            Some(default) => {
                let default = default.to_string();
                let input = cx.new(|cx| InputState::new(window, cx).default_value(default));
                let sub = cx.subscribe_in(&input, window, |this, _, ev: &InputEvent, window, cx| {
                    if let InputEvent::PressEnter { .. } = ev {
                        this.submit_prompt(window, cx);
                    }
                });
                input.update(cx, |i, cx| i.focus(window, cx));
                (Some(input), Some(sub))
            }
            None => (None, None),
        };
        self.prompt = Some(Prompt {
            title: title.into(),
            message: message.map(Into::into),
            input,
            confirm: confirm.to_string().into(),
            danger,
            action: Some(Box::new(action)),
            _subscription: subscription,
        });
        cx.notify();
    }

    /// A yes/no confirmation before a destructive operation.
    #[allow(clippy::too_many_arguments)]
    pub fn confirm(
        &mut self,
        title: &str,
        message: String,
        confirm: &str,
        label: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
        op: impl FnOnce(&Repo) -> Result<()> + Send + 'static,
    ) {
        let label = label.to_string();
        self.open_prompt(title.to_string(), Some(message), None, confirm, true, window, cx, move |_| {
            (label, Box::new(op) as Op)
        });
    }

    pub fn submit_prompt(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        let Some(mut prompt) = self.prompt.take() else { return };
        let text = prompt.input.as_ref().map(|i| i.read(cx).value().trim().to_string()).unwrap_or_default();
        if prompt.input.is_some() && text.is_empty() {
            self.prompt = Some(prompt);
            return;
        }
        if let Some(action) = prompt.action.take() {
            let (label, op) = action(text);
            self.run(label, Change::Refs, cx, op);
        }
        cx.notify();
    }

    pub fn cancel_prompt(&mut self, cx: &mut Context<Self>) {
        self.prompt = None;
        cx.notify();
    }

    fn debug_open_first_diff(&mut self, cx: &mut Context<Self>) {
        match self.selection {
            Selection::Wip => {
                let st = &self.status;
                let (change, staged) = match st.unstaged.iter().find(|c| c.status != FileStatus::Untracked) {
                    Some(c) => (c.clone(), false),
                    None => match st.staged.first() {
                        Some(c) => (c.clone(), true),
                        None => return,
                    },
                };
                self.show_workdir_diff(change, staged, cx);
            }
            Selection::Commit(i) => {
                let Some(id) = self.snap.commits.get(i).map(|c| c.id.clone()) else { return };
                let Some(repo) = self.repo.clone() else { return };
                let path = repo.commit_files(&id).ok().and_then(|f| f.first().map(|f| f.path.clone())).unwrap_or_default();
                self.show_diff(format!("{path} @ {}", &id[..7]), DiffSource::Commit { id, path }, cx);
            }
            Selection::None => {}
        }
    }
}
