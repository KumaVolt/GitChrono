//! Repository window state and the actions that mutate it. Rendering lives in
//! `views`, `diff_view` and `menus`.

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

/// Quiet period after a filesystem event before refreshing.
const DEBOUNCE: Duration = Duration::from_millis(400);
/// Minimum gap after a full history reload before handling more events.
const RELOAD_COOLDOWN: Duration = Duration::from_secs(2);

use anyhow::Result;
use gitchrono_core::{ApplyMode, Commit, FileChange, FileDiff, FileStatus, Repo, Snapshot, Status, UndoEntry};
use gitchrono_graph::Graph;
use gpui_kit::component::input::{InputEvent, InputState, TextareaState};
use gpui_kit::*;

use crate::terminal::Terminal;
use crate::watcher::{self, Change};

const LOG_LIMIT: usize = 50_000;
const UNDO_LIMIT: usize = 50;

actions!(workspace, [CloseSearch]);

pub fn init(cx: &mut App) {
    cx.bind_keys([KeyBinding::new("escape", CloseSearch, Some("GraphSearch > Input"))]);
}

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

/// What an AI "Suggest" button in a prompt fills in.
#[derive(Clone, Copy)]
pub enum Suggest {
    BranchName,
    StashMessage,
}

pub struct Prompt {
    pub title: SharedString,
    pub message: Option<SharedString>,
    pub input: Option<Entity<InputState>>,
    pub confirm: SharedString,
    pub danger: bool,
    pub suggest: Option<Suggest>,
    /// Keep the text as typed (and allow it empty) instead of treating it as a ref name.
    pub raw: bool,
    action: Option<Box<dyn FnOnce(String) -> (String, Op)>>,
    _subscription: Option<Subscription>,
}

pub enum WorkspaceEvent {
    /// The user asked to open another repository (handled by the tab shell).
    OpenRequested,
}

impl EventEmitter<WorkspaceEvent> for Workspace {}

/// The graph's search bar: matches dim everything else and Enter walks them.
pub struct GraphSearch {
    pub input: Entity<InputState>,
    /// Commit indices that match, newest first.
    pub matches: Vec<usize>,
    pub current: usize,
    pub hit: HashSet<usize>,
    _sub: Subscription,
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
    pub ai: Option<gitchrono_ai::AiConfig>,
    pub ai_settings: Option<crate::ai::AiSettings>,
    pub ai_busy: Option<SharedString>,
    /// Token usage of the last AI request, shown so costs stay visible.
    pub ai_usage: Option<SharedString>,
    pub(crate) ai_task: Option<Task<()>>,
    /// `GITCHRONO_SELECT=<row>[:diff]`, applied once after the first load (for headless screenshots).
    debug_select: Option<(usize, bool)>,
    load_task: Option<Task<()>>,
    status_task: Option<Task<()>>,
    _watcher: Option<(notify::RecommendedWatcher, Task<()>)>,
    /// Whether our window has focus. In the background we only record that
    /// something changed and refresh once on re-activation.
    active: bool,
    pending: Option<Change>,
    /// Whether this repository's tab is the one shown.
    visible: bool,
    pub terminals: Vec<Entity<Terminal>>,
    pub terminal_index: usize,
    pub terminal_open: bool,
    pub search: Option<GraphSearch>,
    pub undo_stack: Vec<UndoEntry>,
    pub redo_stack: Vec<UndoEntry>,
    pub sidebar_filter: Entity<InputState>,
    _sidebar_filter_sub: Subscription,
    _activation: Subscription,
}

impl Workspace {
    pub fn new(path: Option<PathBuf>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let summary = cx.new(|cx| InputState::new(window, cx).placeholder("Commit summary"));
        let description = cx.new(|cx| TextareaState::new(window, cx).placeholder("Description"));
        let activation = cx.observe_window_activation(window, |this, window, cx| {
            this.active = window.is_window_active();
            if this.active && this.visible {
                if let Some(change) = this.pending.take() {
                    this.apply_change(change, cx);
                }
            }
        });
        let sidebar_filter = cx.new(|cx| InputState::new(window, cx).placeholder("Filter refs…").autocorrect(false));
        let sidebar_filter_sub = cx.observe(&sidebar_filter, |_, _, cx| cx.notify());
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
            ai: gitchrono_ai::AiConfig::load().filter(|c| c.enabled),
            ai_settings: None,
            ai_busy: None,
            ai_usage: None,
            ai_task: None,
            debug_select: std::env::var("GITCHRONO_SELECT").ok().and_then(|v| {
                let (row, diff) = v.split_once(':').map_or((v.as_str(), false), |(r, _)| (r, true));
                Some((row.parse().ok()?, diff))
            }),
            load_task: None,
            status_task: None,
            _watcher: None,
            active: true,
            pending: None,
            visible: true,
            terminals: Vec::new(),
            terminal_index: 0,
            terminal_open: false,
            search: None,
            undo_stack: Vec::new(),
            redo_stack: Vec::new(),
            sidebar_filter,
            _sidebar_filter_sub: sidebar_filter_sub,
            _activation: activation,
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
                cx.background_executor().timer(DEBOUNCE).await;
                let mut change = first;
                while let Ok(next) = rx.try_recv() {
                    change = change.max(next);
                }
                let Ok(reloaded) = this.update(cx, |this, cx| {
                    if this.active && this.visible && this.busy.is_none() {
                        this.apply_change(change, cx);
                        change == Change::Refs
                    } else {
                        // Background (or mid-operation): remember, refresh later.
                        this.pending = this.pending.max(Some(change));
                        false
                    }
                }) else {
                    break;
                };
                if reloaded {
                    cx.background_executor().timer(RELOAD_COOLDOWN).await;
                }
            }
        });
        Some((watcher, task))
    }

    fn apply_change(&mut self, change: Change, cx: &mut Context<Self>) {
        match change {
            Change::Status => self.refresh_status(cx),
            Change::Refs => self.reload(cx),
        }
    }

    pub fn prompt_open(&mut self, cx: &mut Context<Self>) {
        cx.emit(WorkspaceEvent::OpenRequested);
    }

    /// Tabs in the background skip refreshes and catch up when shown again.
    pub fn set_visible(&mut self, visible: bool, cx: &mut Context<Self>) {
        self.visible = visible;
        if visible && self.active {
            if let Some(change) = self.pending.take() {
                self.apply_change(change, cx);
            }
        }
    }

    pub fn prompt_create_branch(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.open_prompt("Create branch", Some("Branches from HEAD and checks it out.".into()), Some(""), "Create & checkout", false, window, cx, |name| {
            ("Creating branch".into(), Box::new(move |r: &Repo| r.create_branch(&name, None, true)) as Op)
        });
        if self.has_wip()
            && let Some(p) = &mut self.prompt
        {
            p.suggest = Some(Suggest::BranchName);
        }
    }

    pub fn prompt_stash(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.status.is_clean() {
            return;
        }
        self.open_prompt("Stash changes", Some("Message (optional)".into()), Some(""), "Stash", false, window, cx, |message| {
            let op = move |r: &Repo| r.stash_push(Some(message.as_str()).filter(|m| !m.is_empty()));
            ("Stashing".into(), Box::new(op) as Op)
        });
        if let Some(p) = &mut self.prompt {
            p.raw = true;
            p.suggest = Some(Suggest::StashMessage);
        }
    }

    // ------------------------------------------------------------ search

    pub fn open_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(search) = &self.search {
            let input = search.input.clone();
            input.update(cx, |i, cx| i.focus(window, cx));
            return;
        }
        let input = cx.new(|cx| InputState::new(window, cx).placeholder("Search message, author or SHA").autocorrect(false));
        let sub = cx.subscribe_in(&input, window, |this, _, event: &InputEvent, _, cx| match event {
            InputEvent::Change => this.update_search(cx),
            InputEvent::PressEnter { shift, .. } => this.step_search(if *shift { -1 } else { 1 }, cx),
            _ => {}
        });
        input.update(cx, |i, cx| i.focus(window, cx));
        self.search = Some(GraphSearch { input, matches: Vec::new(), current: 0, hit: HashSet::new(), _sub: sub });
        cx.notify();
    }

    pub fn close_search(&mut self, cx: &mut Context<Self>) {
        self.search = None;
        cx.notify();
    }

    fn update_search(&mut self, cx: &mut Context<Self>) {
        let Some(search) = &mut self.search else { return };
        let query = search.input.read(cx).value().trim().to_lowercase();
        search.matches = if query.is_empty() {
            Vec::new()
        } else {
            self.snap
                .commits
                .iter()
                .enumerate()
                .filter(|(_, c)| commit_matches(c, &query))
                .map(|(i, _)| i)
                .collect()
        };
        search.hit = search.matches.iter().copied().collect();
        search.current = 0;
        if let Some(&first) = search.matches.first() {
            let id = self.snap.commits[first].id.clone();
            self.select_commit_id(&id, cx);
        }
        cx.notify();
    }

    pub fn step_search(&mut self, delta: isize, cx: &mut Context<Self>) {
        let Some(search) = &mut self.search else { return };
        if search.matches.is_empty() {
            return;
        }
        let n = search.matches.len() as isize;
        search.current = (search.current as isize + delta).rem_euclid(n) as usize;
        let id = self.snap.commits[search.matches[search.current]].id.clone();
        self.select_commit_id(&id, cx);
    }

    // ------------------------------------------------------------ terminal

    pub fn toggle_terminal(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.terminal_open {
            self.terminal_open = false;
        } else if self.terminals.is_empty() {
            self.new_terminal(window, cx);
        } else {
            self.terminal_open = true;
            self.focus_terminal(window, cx);
        }
        cx.notify();
    }

    pub fn new_terminal(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(root) = self.repo.as_ref().map(|r| r.root().to_path_buf()) else { return };
        let spawned = match Terminal::spawn(&root) {
            Ok(spawned) => spawned,
            Err(e) => {
                self.error = Some(format!("Terminal: {e:#}").into());
                cx.notify();
                return;
            }
        };
        let terminal = cx.new(|cx| Terminal::new(spawned, cx));
        self.terminals.push(terminal);
        self.terminal_index = self.terminals.len() - 1;
        self.terminal_open = true;
        self.focus_terminal(window, cx);
        cx.notify();
    }

    pub fn select_terminal(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        self.terminal_index = index.min(self.terminals.len().saturating_sub(1));
        self.focus_terminal(window, cx);
        cx.notify();
    }

    pub fn close_terminal(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        if index < self.terminals.len() {
            self.terminals.remove(index);
        }
        if self.terminals.is_empty() {
            self.terminal_open = false;
        } else {
            self.terminal_index = self.terminal_index.min(self.terminals.len() - 1);
            self.focus_terminal(window, cx);
        }
        cx.notify();
    }

    fn focus_terminal(&self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(t) = self.terminals.get(self.terminal_index) {
            let focus = t.read(cx).focus.clone();
            focus.focus(window, cx);
        }
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

    /// Run a git operation in the background, then refresh. Operations that
    /// move local refs become undoable.
    pub fn run(
        &mut self,
        label: impl Into<String>,
        refresh: Change,
        cx: &mut Context<Self>,
        op: impl FnOnce(&Repo) -> Result<()> + Send + 'static,
    ) {
        self.run_inner(label.into(), refresh, true, cx, op, |_, _| {});
    }

    fn run_inner(
        &mut self,
        label: String,
        refresh: Change,
        record: bool,
        cx: &mut Context<Self>,
        op: impl FnOnce(&Repo) -> Result<()> + Send + 'static,
        done: impl FnOnce(&mut Self, bool) + 'static,
    ) {
        let Some(repo) = self.repo.clone() else { return };
        if self.busy.is_some() {
            return;
        }
        self.busy = Some(format!("{label}…").into());
        self.error = None;
        cx.notify();
        let soft = label == "Committing";
        let task = cx.background_spawn(async move {
            let before = if record { repo.ref_state().ok() } else { None };
            let result = op(&repo);
            // Half-finished merges/rebases aren't a state worth returning to.
            let settled = || repo.status().is_ok_and(|s| s.operation == gitchrono_core::Operation::None);
            let entry = match (&result, before) {
                (Ok(()), Some(before)) if settled() => {
                    repo.ref_state().ok().and_then(|after| UndoEntry::new(&label, before, after, soft))
                }
                _ => None,
            };
            (result, entry)
        });
        cx.spawn(async move |this, cx| {
            let (result, entry) = task.await;
            this.update(cx, |this, cx| {
                this.busy = None;
                let ok = result.is_ok();
                if let Err(e) = result {
                    this.error = Some(format!("{e:#}").into());
                }
                if let Some(entry) = entry {
                    this.undo_stack.push(entry);
                    if this.undo_stack.len() > UNDO_LIMIT {
                        this.undo_stack.remove(0);
                    }
                    this.redo_stack.clear();
                }
                done(this, ok);
                // Our own refresh covers whatever the watcher saw meanwhile.
                let refresh = this.pending.take().map_or(refresh, |p| p.max(refresh));
                this.apply_change(refresh, cx);
            })
            .ok();
        })
        .detach();
    }

    pub fn undo(&mut self, cx: &mut Context<Self>) {
        let Some(entry) = self.undo_stack.pop() else { return };
        let label = format!("Undoing {}", entry.label.to_lowercase());
        let back = entry.clone();
        self.run_inner(label, Change::Refs, false, cx, move |r| r.undo(&back), move |this, ok| {
            if ok { this.redo_stack.push(entry) } else { this.undo_stack.push(entry) }
        });
    }

    pub fn redo(&mut self, cx: &mut Context<Self>) {
        let Some(entry) = self.redo_stack.pop() else { return };
        let label = format!("Redoing {}", entry.label.to_lowercase());
        let forward = entry.clone();
        self.run_inner(label, Change::Refs, false, cx, move |r| r.redo(&forward), move |this, ok| {
            if ok { this.undo_stack.push(entry) } else { this.redo_stack.push(entry) }
        });
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

    pub fn select_wip(&mut self, cx: &mut Context<Self>) {
        if self.has_wip() {
            self.select_row(0, cx);
        }
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
                let input = cx.new(|cx| InputState::new(window, cx).default_value(default).autocorrect(false));
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
            suggest: None,
            raw: false,
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
        let value = prompt.input.as_ref().map(|i| i.read(cx).value()).unwrap_or_default();
        let text = if prompt.raw {
            value.trim().to_string()
        } else {
            // Ref names: git rejects whitespace, so join words with `-`.
            value.split_whitespace().collect::<Vec<_>>().join("-")
        };
        if prompt.input.is_some() && text.is_empty() && !prompt.raw {
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

    /// Lower-cased filter text for the sidebar, if any.
    pub fn ref_filter(&self, cx: &App) -> String {
        self.sidebar_filter.read(cx).value().trim().to_lowercase()
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

/// `query` is lower-case: a SHA prefix, or text in the subject, body or author.
fn commit_matches(c: &Commit, query: &str) -> bool {
    (query.len() >= 4 && query.chars().all(|ch| ch.is_ascii_hexdigit()) && c.id.starts_with(query))
        || c.summary.to_lowercase().contains(query)
        || c.author_name.to_lowercase().contains(query)
        || c.author_email.to_lowercase().contains(query)
        || c.body.to_lowercase().contains(query)
}
