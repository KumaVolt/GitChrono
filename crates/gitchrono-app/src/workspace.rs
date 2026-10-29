//! Repository window state and the actions that mutate it. Rendering lives in
//! `views`, `diff_view` and `menus`.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
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
    pub highlights: Arc<crate::highlight::Highlights>,
}

/// What an AI "Suggest" button in a prompt fills in.
#[derive(Clone, Copy)]
pub enum Suggest {
    BranchName,
    StashMessage,
}

/// The selected commit's (or stash's) message, edited in place in the details panel.
pub struct MessageEdit {
    pub id: String,
    /// The stash being renamed, when `id` is a stash.
    pub stash: Option<usize>,
    pub summary: Entity<InputState>,
    pub description: Entity<TextareaState>,
    /// Whether a remote already has the commit (`None` while checking).
    pub pushed: Option<bool>,
    /// Save was pressed on a pushed commit: ask once more before rewriting it.
    pub confirm_pushed: bool,
    _enter: Subscription,
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
    /// Whether the operation goes on the undo stack.
    pub record: bool,
    action: Option<Box<dyn FnOnce(String) -> (String, Op)>>,
    _subscription: Option<Subscription>,
}

pub enum WorkspaceEvent {
    /// The user asked to open another repository (handled by the tab shell).
    OpenRequested,
    /// The user asked to clone a repository (the shell shows the dialog).
    CloneRequested,
    /// The user asked to create a repository with `git init` (handled by the tab shell).
    InitRequested,
    /// Open this repository or worktree in a tab.
    OpenPath(PathBuf),
    /// A linked worktree was removed; its tab should close.
    WorktreeRemoved(PathBuf),
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
    /// Per graph row, the lanes drawn as a stash's dashed line.
    pub stash_lines: Arc<Vec<u64>>,
    pub selection: Selection,
    pub commit_files: Vec<FileChange>,
    pub commit_signature: Option<gitchrono_core::Signature>,
    /// Open pull requests from `gh`; `None` when unavailable.
    pub pull_requests: Option<Vec<crate::github::PullRequest>>,
    /// Commit indices picked with ⌘/Shift-click (two or more means multi-select).
    pub multi: Vec<usize>,
    /// Files picked with ⌘/Shift-click in the staging lists: which list
    /// (staged or not) and the paths, the first being the Shift anchor.
    pub file_multi: (bool, Vec<String>),
    /// Graph row whose ref labels are hovered: (label cell, the full list it opens).
    pub refs_hover: (Option<usize>, Option<usize>),
    /// Graph column width set by dragging its edge; `None` sizes it to the lanes.
    pub graph_width: Option<f32>,
    /// Re-render when a profile picture finishes downloading.
    _avatars: Subscription,
    /// Scroll positions of the commit message and AI explanation boxes.
    pub message_scroll: ScrollHandle,
    pub explanation_scroll: ScrollHandle,
    /// Commit whose id was just copied, for a moment of "Copied ✓".
    pub copied_sha: Option<String>,
    /// Set when a push needs the remote's new commits first; the banner
    /// offers "Pull and push".
    pub push_behind: Option<SharedString>,
    /// Something done on the user's behalf worth knowing (changes stashed to
    /// make room for a stash); cleared by the next operation.
    pub notice: Option<SharedString>,
    /// Clear the commit message on the next render (needs the window).
    pub reset_commit_form: bool,
    /// AI explanation of a commit: (commit id, text).
    pub explanation: Option<(String, SharedString)>,
    /// Folder view for file lists (staging and commit files) instead of full paths.
    pub file_tree: bool,
    /// Collapsed folders in folder view, as "<list>:<dir path>".
    pub collapsed_dirs: std::collections::HashSet<String>,
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
    /// `GITCHRONO_TERMINAL=<command>`: open the terminal and run it after the first load (screenshots).
    debug_terminal: Option<String>,
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
    /// Open the worktree that the next reload discovers (after "create worktree").
    open_new_worktree: bool,
    pub rebase_editor: Option<crate::rebase_ui::RebaseEditor>,
    pub drop_menu: Option<crate::drag::DropMenu>,
    /// Branches hidden from / soloed in the graph (saved per repository).
    pub ref_filter: gitchrono_core::RefFilter,
    /// Collapsed sidebar sections, saved per repository.
    pub collapsed_sections: std::collections::BTreeSet<String>,
    pub conflict: Option<crate::conflict_ui::ConflictView>,
    /// Side-by-side instead of unified diffs.
    pub diff_split: bool,
    pub file_history: Option<crate::history::FileHistory>,
    pub blame: Option<crate::history::BlameView>,
    pub undo_stack: Vec<UndoEntry>,
    pub redo_stack: Vec<UndoEntry>,
    pub message_edit: Option<MessageEdit>,
    /// Commit to select after the next reload (a reworded commit's new id).
    reselect: Option<String>,
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
            stash_lines: Arc::default(),
            selection: Selection::None,
            commit_files: Vec::new(),
            commit_signature: None,
            pull_requests: None,
            multi: Vec::new(),
            file_multi: (false, Vec::new()),
            explanation: None,
            reset_commit_form: false,
            push_behind: None,
            notice: None,
            copied_sha: None,
            message_scroll: ScrollHandle::new(),
            explanation_scroll: ScrollHandle::new(),
            graph_width: None,
            _avatars: cx.observe(&cx.global::<crate::avatars::AvatarStore>().0.clone(), |_, _, cx| cx.notify()),
            refs_hover: (None, None),
            file_tree: std::env::var_os("GITCHRONO_FOLDERS").is_some(),
            collapsed_dirs: Default::default(),
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
            debug_terminal: std::env::var("GITCHRONO_TERMINAL").ok(),
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
            open_new_worktree: false,
            rebase_editor: None,
            drop_menu: None,
            ref_filter: Default::default(),
            collapsed_sections: Default::default(),
            conflict: None,
            diff_split: std::env::var_os("GITCHRONO_SPLIT").is_some(),
            file_history: None,
            blame: None,
            undo_stack: Vec::new(),
            redo_stack: Vec::new(),
            message_edit: None,
            reselect: None,
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
                self.ref_filter = crate::projects::load_ref_filter(repo.root());
                self.collapsed_sections = crate::projects::load_collapsed_sections(repo.root());
                self._watcher = self.start_watcher(&repo, cx);
                self.repo = Some(repo);
                self.selection = Selection::None;
                self.diff = None;
                self.reload(cx);
                self.load_pull_requests(cx);
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

    pub fn prompt_init(&mut self, cx: &mut Context<Self>) {
        cx.emit(WorkspaceEvent::InitRequested);
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

    /// What quitting now would throw away or cut short, for the restart prompt.
    pub fn unsaved_work(&self, cx: &App) -> Vec<&'static str> {
        let mut lost = Vec::new();
        if !self.summary.read(cx).value().trim().is_empty() || !self.description.read(cx).value().trim().is_empty() {
            lost.push("the commit message");
        }
        if self.busy.is_some() {
            lost.push("a running git command");
        }
        if self.rebase_editor.is_some() {
            lost.push("the rebase plan");
        }
        if self.conflict.as_ref().is_some_and(|c| c.choices.iter().any(Option::is_some)) {
            lost.push("conflict choices");
        }
        if self.terminals.iter().any(|t| !t.read(cx).exited) {
            lost.push("open terminals");
        }
        lost
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

    // ------------------------------------------------------------ worktrees

    /// `../<repo>-<branch>` next to the main working tree.
    fn default_worktree_path(&self, branch: &str) -> String {
        let main = self.snap.worktrees.iter().find(|w| w.main).map(|w| w.path.clone());
        let root = main.or_else(|| self.repo.as_ref().map(|r| r.root().to_path_buf())).unwrap_or_default();
        let name = root.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let parent = root.parent().map(Path::to_path_buf).unwrap_or_default();
        parent.join(format!("{name}-{}", branch.replace('/', "-"))).to_string_lossy().into_owned()
    }

    /// Create a worktree for an existing local or remote branch.
    pub fn prompt_worktree_for_branch(&mut self, branch: String, remote: bool, window: &mut Window, cx: &mut Context<Self>) {
        let local = if remote { branch.split_once('/').map_or(branch.clone(), |(_, b)| b.to_string()) } else { branch.clone() };
        let default = self.default_worktree_path(&local);
        let message = format!("Checks out {branch} in a separate folder, opened in a new tab.");
        self.open_prompt(format!("Create worktree for {branch}"), Some(message), Some(&default), "Create worktree", false, window, cx, move |path| {
            let op = move |r: &Repo| {
                let path = PathBuf::from(&path);
                if remote { r.add_worktree(&path, Some(&local), Some(&branch)) } else { r.add_worktree(&path, None, Some(&branch)) }
            };
            ("Creating worktree".into(), Box::new(op) as Op)
        });
        self.prepare_worktree_prompt();
    }

    /// Create a worktree on a new branch from HEAD.
    pub fn prompt_new_worktree(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let base = self.default_worktree_path("");
        let message = format!("New branch from HEAD, checked out in {base}<branch>, opened in a new tab.");
        self.open_prompt("Create worktree", Some(message), Some(""), "Create worktree", false, window, cx, move |name| {
            let path = PathBuf::from(format!("{base}{}", name.replace('/', "-")));
            ("Creating worktree".into(), Box::new(move |r: &Repo| r.add_worktree(&path, Some(&name), None)) as Op)
        });
        if let Some(p) = &mut self.prompt {
            p.record = false;
        }
        self.open_new_worktree = true;
    }

    fn prepare_worktree_prompt(&mut self) {
        if let Some(p) = &mut self.prompt {
            p.raw = true;
            // Undoing would delete a branch the new worktree has checked out.
            p.record = false;
        }
        self.open_new_worktree = true;
    }

    pub fn remove_worktree(&mut self, path: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        let shown = crate::projects::short_path(&path);
        self.confirm(
            "Remove worktree",
            format!("Remove the worktree at {shown}? Its branch is kept; uncommitted changes there block removal."),
            "Remove",
            "Removing worktree",
            window,
            cx,
            move |r| r.remove_worktree(&path, false),
        );
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
        let selected_id = self.reselect.take().or_else(|| match self.selection {
            Selection::Commit(i) => self.snap.commits.get(i).map(|c| c.id.clone()),
            _ => None,
        });
        let filter = self.ref_filter.clone();
        let task = cx.background_spawn(async move {
            let snap = repo.snapshot_filtered(LOG_LIMIT, &filter)?;
            let graph = gitchrono_graph::layout(
                snap.commits.iter().map(|c| (c.id.as_str(), c.parents.iter().map(String::as_str))),
            );
            let stash_lines = crate::graph_paint::stash_lines(&graph, &snap.commits);
            anyhow::Ok((snap, graph, stash_lines))
        });
        self.load_task = Some(cx.spawn(async move |this, cx| {
            let result = task.await;
            this.update(cx, |this, cx| {
                match result {
                    Ok((mut snap, graph, stash_lines)) => {
                        for gone in this.snap.worktrees.iter().filter(|old| !snap.worktrees.iter().any(|w| w.path == old.path)) {
                            cx.emit(WorkspaceEvent::WorktreeRemoved(gone.path.clone()));
                        }
                        if std::mem::take(&mut this.open_new_worktree) {
                            let known: Vec<&PathBuf> = this.snap.worktrees.iter().map(|w| &w.path).collect();
                            if let Some(new) = snap.worktrees.iter().find(|w| !known.contains(&&w.path)) {
                                cx.emit(WorkspaceEvent::OpenPath(new.path.clone()));
                            }
                        }
                        this.status = std::mem::take(&mut snap.status);
                        // Indices into the old commit list mean nothing now.
                        this.multi.clear();
                        this.snap = Arc::new(snap);
                        this.graph = Arc::new(graph);
                        this.stash_lines = Arc::new(stash_lines);
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
                        // Saved (its id changed) or gone: either way the editor is done.
                        if this.message_edit.as_ref().is_some_and(|e| this.selected_commit().is_none_or(|c| c.id != e.id)) {
                            this.message_edit = None;
                        }
                        if let Some(command) = this.debug_terminal.take() {
                            this.debug_run_in_terminal(command, cx);
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
                Some(DiffSource::Workdir { change, staged }) => Some(repo.workdir_file_diff(&change, staged).map(|diff| {
                    let highlights = crate::highlight::highlight_diff(&change.path, &diff);
                    (diff, highlights)
                })),
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
                        if let Some(Ok((diff, highlights))) = diff {
                            if diff.hunks.is_empty() && !diff.binary {
                                this.diff = None;
                            } else if let Some(view) = this.diff.as_mut() {
                                view.diff = diff;
                                view.highlights = Arc::new(highlights);
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

    pub(crate) fn run_inner(
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
        self.notice = None;
        cx.notify();
        let soft = label == "Committing" || label == "Rewording";
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
                    this.record_undo(entry);
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

    fn record_undo(&mut self, entry: UndoEntry) {
        self.undo_stack.push(entry);
        if self.undo_stack.len() > UNDO_LIMIT {
            self.undo_stack.remove(0);
        }
        self.redo_stack.clear();
    }

    /// Apply or pop `stash@{index}` over uncommitted changes: changes in other
    /// files stay put; if the stash touches a changed file, the changes are
    /// stashed first (undoably) and a notice says so.
    pub fn apply_stash(&mut self, index: usize, pop: bool, cx: &mut Context<Self>) {
        let label = if pop { "Popping stash" } else { "Applying stash" };
        let slot = Arc::new(std::sync::Mutex::new(None));
        let out = slot.clone();
        self.run_inner(label.into(), Change::Refs, true, cx, move |r| {
            let swap = r.apply_stash(index, pop)?;
            *out.lock().unwrap() = swap;
            Ok(())
        }, move |this, _| {
            let Some(swap) = slot.lock().ok().and_then(|mut s| s.take()) else { return };
            let mut files = swap.overlap.iter().take(3).cloned().collect::<Vec<_>>().join(", ");
            if swap.overlap.len() > 3 {
                files += &format!(" and {} more", swap.overlap.len() - 3);
            }
            this.notice = Some(
                format!("The stash also changes {files}, so your changes were saved as stash@{{0}} first instead of being mixed in. Undo (⌘Z) brings them back.").into(),
            );
            this.record_undo(UndoEntry::stash_swap(label, swap));
        });
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
        self.message_edit = None;
        self.selection = if wip && row == 0 { Selection::Wip } else { Selection::Commit(row - usize::from(wip)) };
        self.diff = None;
        if let Selection::Commit(i) = self.selection {
            self.load_commit_files(i, cx);
        }
        cx.notify();
    }

    pub fn load_pull_requests(&mut self, cx: &mut Context<Self>) {
        let Some(root) = self.repo.as_ref().map(|r| r.root().to_path_buf()) else { return };
        let task = cx.background_spawn(async move { crate::github::open_pull_requests(&root) });
        cx.spawn(async move |this, cx| {
            let prs = task.await;
            this.update(cx, |this, cx| {
                this.pull_requests = prs;
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    // ------------------------------------------------------------ graph filter

    pub fn toggle_hidden(&mut self, name: &str, cx: &mut Context<Self>) {
        let f = &mut self.ref_filter;
        if !f.hidden.remove(name) {
            f.hidden.insert(name.to_string());
            f.solo.remove(name);
        }
        self.filter_changed(cx);
    }

    pub fn toggle_solo(&mut self, name: &str, cx: &mut Context<Self>) {
        let f = &mut self.ref_filter;
        if !f.solo.remove(name) {
            f.solo.insert(name.to_string());
            f.hidden.remove(name);
        }
        self.filter_changed(cx);
    }

    pub fn clear_ref_filter(&mut self, cx: &mut Context<Self>) {
        self.ref_filter = Default::default();
        self.filter_changed(cx);
    }

    fn filter_changed(&mut self, cx: &mut Context<Self>) {
        if let Some(repo) = &self.repo {
            crate::projects::save_ref_filter(repo.root(), &self.ref_filter);
        }
        self.reload(cx);
    }

    pub fn copy_sha(&mut self, id: &str, cx: &mut Context<Self>) {
        cx.write_to_clipboard(ClipboardItem::new_string(id.to_string()));
        self.copied_sha = Some(id.to_string());
        cx.notify();
        let id = id.to_string();
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(std::time::Duration::from_millis(1500)).await;
            this.update(cx, |this, cx| {
                if this.copied_sha.as_deref() == Some(id.as_str()) {
                    this.copied_sha = None;
                    cx.notify();
                }
            })
            .ok();
        })
        .detach();
    }

    /// Push, but ask to pull first when the branch is known to be behind, and
    /// offer the same when the remote turns out to have moved on.
    pub fn push(&mut self, cx: &mut Context<Self>) {
        if self.status.behind > 0 {
            let upstream = self.status.upstream.clone().unwrap_or_else(|| "the remote".into());
            let n = self.status.behind;
            self.push_behind = Some(format!("{upstream} has {n} commit{} you don't have yet. Pull them in first, then push.", if n == 1 { "" } else { "s" }).into());
            cx.notify();
            return;
        }
        self.push_behind = None;
        let rejected = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let flag = rejected.clone();
        self.run_inner("Pushing".into(), Change::Refs, true, cx, move |r| {
            r.push(false).inspect_err(|e| {
                if e.downcast_ref::<gitchrono_core::GitError>().is_some_and(|g| g.is_push_rejected()) {
                    flag.store(true, std::sync::atomic::Ordering::Relaxed);
                }
            })
        }, move |this, ok| {
            if !ok && rejected.load(std::sync::atomic::Ordering::Relaxed) {
                this.error = None;
                this.push_behind = Some("Someone pushed to this branch in the meantime. Pull their commits in first, then push.".into());
            }
        });
    }

    pub fn pull_and_push(&mut self, cx: &mut Context<Self>) {
        self.push_behind = None;
        self.run("Pulling and pushing", Change::Refs, cx, |r| r.pull_then_push());
    }

    pub fn toggle_section(&mut self, key: &str, cx: &mut Context<Self>) {
        if !self.collapsed_sections.remove(key) {
            self.collapsed_sections.insert(key.to_string());
        }
        if let Some(repo) = &self.repo {
            crate::projects::save_collapsed_sections(repo.root(), &self.collapsed_sections);
        }
        cx.notify();
    }

    /// Staging list click: plain picks one file (and the caller shows its diff,
    /// signalled by `true`), ⌘/Ctrl toggles, Shift selects a range.
    pub fn click_file(&mut self, files: &[FileChange], index: usize, staged: bool, modifiers: Modifiers, cx: &mut Context<Self>) -> bool {
        let path = files[index].path.clone();
        let toggle = modifiers.platform || modifiers.control;
        if !(toggle || modifiers.shift) {
            self.file_multi = (staged, vec![path]);
            return true;
        }
        if self.file_multi.0 != staged {
            self.file_multi = (staged, Vec::new());
        }
        let picked = &mut self.file_multi.1;
        picked.retain(|p| files.iter().any(|f| &f.path == p));
        if modifiers.shift {
            let anchor = picked.first().and_then(|a| files.iter().position(|f| &f.path == a)).unwrap_or(index);
            let (lo, hi) = (anchor.min(index), anchor.max(index));
            *picked = std::iter::once(anchor).chain((lo..=hi).filter(|&i| i != anchor)).map(|i| files[i].path.clone()).collect();
        } else if let Some(pos) = picked.iter().position(|p| *p == path) {
            picked.remove(pos);
        } else {
            picked.push(path);
        }
        cx.notify();
        false
    }

    /// The picked files still present in that list, in list order.
    pub fn picked_files(&self, files: &[FileChange], staged: bool) -> Vec<FileChange> {
        if self.file_multi.0 != staged {
            return Vec::new();
        }
        files.iter().filter(|f| self.file_multi.1.contains(&f.path)).cloned().collect()
    }

    pub fn stage_files(&mut self, files: &[FileChange], staged: bool, cx: &mut Context<Self>) {
        let paths: Vec<String> = files.iter().map(|f| f.path.clone()).collect();
        self.file_multi.1.clear();
        self.run(if staged { "Unstaging" } else { "Staging" }, Change::Status, cx, move |r| {
            let paths: Vec<&str> = paths.iter().map(String::as_str).collect();
            if staged { r.unstage(&paths) } else { r.stage(&paths) }
        });
    }

    /// Row click: plain selects one, ⌘/Ctrl toggles, Shift extends a range.
    pub fn click_row(&mut self, row: usize, modifiers: Modifiers, cx: &mut Context<Self>) {
        let wip = usize::from(self.has_wip());
        let toggle = modifiers.platform || modifiers.control;
        if row < wip || !(toggle || modifiers.shift) {
            // Clicking the selected commit again closes its details.
            if row >= wip && self.multi.is_empty() && self.selection == Selection::Commit(row - wip) {
                self.selection = Selection::None;
                self.diff = None;
                cx.notify();
                return;
            }
            self.multi.clear();
            self.select_row(row, cx);
            return;
        }
        let index = row - wip;
        if self.multi.is_empty() {
            if let Selection::Commit(current) = self.selection {
                self.multi.push(current);
            }
        }
        if modifiers.shift {
            let anchor = self.multi.first().copied().unwrap_or(index);
            let (lo, hi) = (anchor.min(index), anchor.max(index));
            self.multi = std::iter::once(anchor).chain((lo..=hi).filter(|&i| i != anchor)).collect();
        } else if let Some(pos) = self.multi.iter().position(|&i| i == index) {
            self.multi.remove(pos);
        } else {
            self.multi.push(index);
        }
        match self.multi.len() {
            0 => self.selection = Selection::None,
            1 => {
                let only = self.multi[0];
                self.multi.clear();
                self.select_row(only + wip, cx);
                return;
            }
            _ => {
                self.selection = Selection::Commit(index);
                self.diff = None;
            }
        }
        cx.notify();
    }

    /// Multi-selected commits, oldest first (the list is newest first).
    pub fn multi_commits(&self) -> Vec<&Commit> {
        let mut indices = self.multi.clone();
        indices.sort_unstable_by(|a, b| b.cmp(a));
        indices.iter().filter_map(|&i| self.snap.commits.get(i)).collect()
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
        let task = cx.background_spawn(async move { (repo.commit_files(&id), repo.signature(&id)) });
        cx.spawn(async move |this, cx| {
            let (files, signature) = task.await;
            this.update(cx, |this, cx| {
                if this.selection == Selection::Commit(index) {
                    this.commit_files = files.unwrap_or_default();
                    this.commit_signature = signature.ok().flatten();
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
            let (diff, path) = match load {
                DiffSource::Commit { id, path } => (repo.commit_file_diff(&id, &path)?, path),
                DiffSource::Workdir { change, staged } => (repo.workdir_file_diff(&change, staged)?, change.path),
            };
            let highlights = crate::highlight::highlight_diff(&path, &diff);
            anyhow::Ok((diff, highlights))
        });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            this.update(cx, |this, cx| {
                match result {
                    Ok((diff, highlights)) => {
                        this.diff = Some(DiffView {
                            title: title.into(),
                            source,
                            diff,
                            selected: HashSet::new(),
                            highlights: Arc::new(highlights),
                        })
                    }
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

    pub fn commit(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        let summary = self.summary.read(cx).value().trim().to_string();
        let body = self.description.read(cx).value().trim().to_string();
        let message = if body.is_empty() { summary } else { format!("{summary}\n\n{body}") };
        let amend = self.amend;
        // The form clears only once the commit worked, so a failure keeps the message.
        self.run_inner("Committing".into(), Change::Refs, true, cx, move |repo| repo.commit(&message, amend), |this, ok| {
            if ok {
                this.reset_commit_form = true;
                this.amend = false;
                // The files just committed are gone from the lists; so is their diff.
                if matches!(this.diff.as_ref().map(|d| &d.source), Some(DiffSource::Workdir { .. })) {
                    this.diff = None;
                }
                this.file_multi.1.clear();
            }
        });
    }

    /// Start editing the selected commit's message (or stash's) in the details panel.
    pub fn edit_message(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (Some(repo), Some(commit)) = (self.repo.clone(), self.selected_commit()) else { return };
        let stash = self.snap.refs.stashes.iter().find(|s| s.target == commit.id).map(|s| s.index);
        if stash.is_none() && commit.parents.len() > 1 {
            self.error = Some("Merge commits can't be reworded: rewriting one would flatten the merge.".into());
            cx.notify();
            return;
        }
        let id = commit.id.clone();
        let (summary_text, body) = (commit.summary.clone(), commit.body.trim().to_string());
        let summary = cx.new(|cx| InputState::new(window, cx).placeholder(if stash.is_some() { "Stash message" } else { "Commit summary" }).default_value(summary_text));
        let description = cx.new(|cx| TextareaState::new(window, cx).placeholder("Description"));
        description.update(cx, |d, cx| d.set_value(body, window, cx));
        summary.update(cx, |i, cx| i.focus(window, cx));
        let enter = cx.subscribe_in(&summary, window, |this, _, ev: &InputEvent, _, cx| {
            if let InputEvent::PressEnter { .. } = ev {
                this.save_message_edit(cx);
            }
        });
        self.message_edit =
            Some(MessageEdit { id: id.clone(), stash, summary, description, pushed: stash.map(|_| false), confirm_pushed: false, _enter: enter });
        if stash.is_none() {
            let task = cx.background_spawn(async move { repo.is_pushed(&id).unwrap_or(false) });
            cx.spawn(async move |this, cx| {
                let pushed = task.await;
                this.update(cx, |this, cx| {
                    if let Some(edit) = &mut this.message_edit {
                        edit.pushed = Some(pushed);
                        cx.notify();
                    }
                })
                .ok();
            })
            .detach();
        }
        cx.notify();
    }

    /// Rename the stash at `index` from its row's menu.
    pub fn edit_stash_message(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(target) = self.snap.refs.stashes.iter().find(|s| s.index == index).map(|s| s.target.clone()) {
            self.select_commit_id(&target, cx);
            self.edit_message(window, cx);
        }
    }

    pub fn cancel_message_edit(&mut self, cx: &mut Context<Self>) {
        self.message_edit = None;
        cx.notify();
    }

    /// Reword the commit (or rename the stash). A pushed commit asks first.
    pub fn save_message_edit(&mut self, cx: &mut Context<Self>) {
        let Some(edit) = &mut self.message_edit else { return };
        let summary = edit.summary.read(cx).value().trim().to_string();
        let body = edit.description.read(cx).value().trim().to_string();
        if summary.is_empty() {
            return;
        }
        let message = if body.is_empty() { summary } else { format!("{summary}\n\n{body}") };
        match (edit.stash, edit.pushed) {
            (Some(index), _) => {
                let done = Arc::new(std::sync::Mutex::new(None));
                let result = done.clone();
                self.run_inner(
                    "Renaming stash".into(),
                    Change::Refs,
                    false,
                    cx,
                    move |r| {
                        *result.lock().unwrap() = Some(r.rename_stash(index, &message)?);
                        Ok(())
                    },
                    move |this, ok| {
                        if let Some(stash_edit) = done.lock().unwrap().take().filter(|_| ok) {
                            this.reselect = Some(stash_edit.new_target.clone());
                            this.record_undo(UndoEntry::stash("Renaming stash", stash_edit));
                        }
                    },
                );
            }
            (None, None) => {}
            (None, Some(true)) if !edit.confirm_pushed => {
                edit.confirm_pushed = true;
                cx.notify();
            }
            (None, Some(_)) => {
                let id = edit.id.clone();
                let done = Arc::new(std::sync::Mutex::new(None));
                let result = done.clone();
                self.run_inner(
                    "Rewording".into(),
                    Change::Refs,
                    true,
                    cx,
                    move |r| {
                        *result.lock().unwrap() = Some(r.reword(&id, &message)?);
                        Ok(())
                    },
                    move |this, ok| {
                        if let Some(new_id) = done.lock().unwrap().take().filter(|_| ok) {
                            this.reselect = Some(new_id);
                        }
                    },
                );
            }
        }
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
            record: true,
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
            self.run_inner(label, Change::Refs, prompt.record, cx, op, |_, _| {});
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

    fn debug_run_in_terminal(&mut self, command: String, cx: &mut Context<Self>) {
        let Some(root) = self.repo.as_ref().map(|r| r.root().to_path_buf()) else { return };
        let Ok(spawned) = Terminal::spawn(&root) else { return };
        let terminal = cx.new(|cx| Terminal::new(spawned, cx));
        // Give the shell a moment to start before typing.
        let t = terminal.clone();
        cx.spawn(async move |_, cx| {
            cx.background_executor().timer(Duration::from_millis(1200)).await;
            t.update(cx, |t, _| t.write(format!("{command}\r").into_bytes()));
        })
        .detach();
        self.terminals.push(terminal);
        self.terminal_index = self.terminals.len() - 1;
        self.terminal_open = true;
    }

    fn debug_open_first_diff(&mut self, cx: &mut Context<Self>) {
        match self.selection {
            Selection::Wip => {
                if let Some(conflicted) = self.status.conflicted.first() {
                    let path = conflicted.path.clone();
                    self.open_conflict(path, cx);
                    return;
                }
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
