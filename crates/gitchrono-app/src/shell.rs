//! The window: a tab per repository plus a "home" tab listing recent
//! repositories and project folders (parent folders with several repos).

use std::collections::HashMap;
use std::path::PathBuf;

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::{Disableable as _, Sizable as _};
use gpui_kit::prelude::*;
use gpui_kit::*;

use gitchrono_core::gitbin::{self, GitCheck};
use gitchrono_core::summary::{self, RepoSummary};

use crate::projects::{self, Session};
use crate::theme::{self, c};
use crate::workspace::{Workspace, WorkspaceEvent};

actions!(gitchrono, [NewTab, CloseTab, NextTab, PrevTab, OpenAny, ToggleTerminal, OpenPalette, FindCommits, Undo, Redo, OpenSettings]);

pub fn init(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("secondary-t", NewTab, None),
        KeyBinding::new("secondary-w", CloseTab, None),
        KeyBinding::new("secondary-o", OpenAny, None),
        KeyBinding::new("ctrl-tab", NextTab, None),
        KeyBinding::new("ctrl-shift-tab", PrevTab, None),
        KeyBinding::new("secondary-shift-]", NextTab, None),
        KeyBinding::new("secondary-shift-[", PrevTab, None),
        KeyBinding::new("alt-t", ToggleTerminal, None),
        KeyBinding::new("ctrl-`", ToggleTerminal, None),
        KeyBinding::new("secondary-p", OpenPalette, None),
        KeyBinding::new("secondary-shift-p", OpenPalette, None),
        KeyBinding::new("secondary-k", OpenPalette, None),
        KeyBinding::new("secondary-f", FindCommits, None),
        KeyBinding::new("secondary-z", Undo, None),
        KeyBinding::new("secondary-shift-z", Redo, None),
        KeyBinding::new("ctrl-y", Redo, None),
        KeyBinding::new("secondary-,", OpenSettings, None),
    ]);
}

/// The app icon, drawn by `scripts/make-icons.py`.
pub(crate) fn logo() -> std::sync::Arc<Image> {
    static LOGO: std::sync::OnceLock<std::sync::Arc<Image>> = std::sync::OnceLock::new();
    LOGO.get_or_init(|| std::sync::Arc::new(Image::from_bytes(ImageFormat::Png, include_bytes!("../../../packaging/linux/gitchrono.png").to_vec())))
        .clone()
}

enum Tab {
    Home,
    Repo { path: PathBuf, view: Entity<Workspace>, _sub: Subscription },
}

pub struct Shell {
    tabs: Vec<Tab>,
    pub(crate) active: usize,
    pub(crate) palette: Option<crate::palette::Palette>,
    pub(crate) settings: Option<crate::settings::Settings>,
    pub(crate) update: crate::footer::UpdateState,
    pub(crate) about: Option<crate::footer::About>,
    pub(crate) clone_dialog: Option<crate::clone_ui::CloneDialog>,
    /// Whether a usable git was found; without one the help page replaces the tabs.
    pub(crate) git: GitCheck,
    /// The repository to open once git is available (from the command line).
    pending_open: Option<PathBuf>,
    pub(crate) session: Session,
    /// Repositories found in each project folder.
    scans: HashMap<PathBuf, Vec<PathBuf>>,
    /// Branch and status of each repository listed on the new-tab page.
    summaries: HashMap<PathBuf, Result<RepoSummary, SharedString>>,
    filter: Entity<InputState>,
    _filter_sub: Subscription,
    /// Keeps key bindings working when no input has focus.
    focus: FocusHandle,
    _keystrokes: Subscription,
}

impl Shell {
    pub fn new(open: Option<PathBuf>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let filter = cx.new(|cx| InputState::new(window, cx).placeholder("Filter repositories…").autocorrect(false));
        let filter_sub = cx.observe(&filter, |_, _, cx| cx.notify());
        // Escape closes overlays even when a text input consumed the key.
        let keystrokes = cx.observe_keystrokes(|this: &mut Self, event, _, cx| {
            if event.keystroke.key == "escape" {
                if this.palette.is_some() {
                    this.close_palette(cx);
                } else if this.clone_dialog.is_some() {
                    this.close_clone(cx);
                } else if this.about.is_some() {
                    this.about = None;
                    cx.notify();
                } else if this.settings.is_some() {
                    this.close_settings(cx);
                } else if let Some(ws) = this.active_workspace().cloned() {
                    ws.update(cx, |w, cx| w.close_search(cx));
                }
            }
        });
        let session = Session::load();
        let mut this =
            Self { tabs: Vec::new(), active: 0, palette: None, settings: None, update: Default::default(), about: None, clone_dialog: None, git: gitbin::check(), pending_open: None, session: session.clone(), scans: HashMap::new(), summaries: HashMap::new(), filter, _filter_sub: filter_sub, focus: cx.focus_handle(), _keystrokes: keystrokes };

        if matches!(this.git, GitCheck::Ok { .. }) {
            this.restore(open, window, cx);
        } else {
            this.pending_open = open;
            this.tabs.push(Tab::Home);
            this.activate(0, window, cx);
        }
        this.schedule_update_checks(cx);
        // `GITCHRONO_SETTINGS=1`: open the settings page on launch (screenshots).
        if std::env::var_os("GITCHRONO_SETTINGS").is_some() {
            this.open_settings(this.active_repo_path(), window, cx);
        }
        // `GITCHRONO_CLONE=<url>`: open the clone dialog with the URL filled in (screenshots).
        if let Ok(url) = std::env::var("GITCHRONO_CLONE") {
            this.open_clone_with(&url, window, cx);
        }
        this
    }

    /// Reopens the last session's tabs and folders (once git is available).
    fn restore(&mut self, open: Option<PathBuf>, window: &mut Window, cx: &mut Context<Self>) {
        self.tabs.clear();
        for path in self.session.tabs.clone().iter().filter(|p| projects::is_repo(p)) {
            self.push_repo(path.clone(), window, cx);
        }
        for folder in self.session.folders.clone() {
            self.scan(folder, cx);
        }
        self.active = self.session.active.min(self.tabs.len().saturating_sub(1));
        if let Some(path) = open {
            self.open_path(path, window, cx);
        }
        if self.tabs.is_empty() {
            self.tabs.push(Tab::Home);
        }
        self.activate(self.active, window, cx);
    }

    pub(crate) fn recheck_git(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.git = gitbin::check();
        if matches!(self.git, GitCheck::Ok { .. }) {
            let open = self.pending_open.take();
            self.restore(open, window, cx);
        }
        cx.notify();
    }

    fn push_repo(&mut self, path: PathBuf, window: &mut Window, cx: &mut Context<Self>) -> usize {
        let open = path.clone();
        let view = cx.new(|cx| Workspace::new(Some(open), window, cx));
        let sub = cx.subscribe_in(&view, window, |this, _, event: &WorkspaceEvent, window, cx| match event {
            WorkspaceEvent::OpenRequested => this.prompt_open(window, cx),
            WorkspaceEvent::CloneRequested => this.open_clone(window, cx),
            WorkspaceEvent::OpenPath(path) => this.open_path(path.clone(), window, cx),
            WorkspaceEvent::WorktreeRemoved(path) => {
                let gone = path.canonicalize().unwrap_or_else(|_| path.clone());
                if let Some(i) = this.tabs.iter().position(|t| matches!(t, Tab::Repo { path: p, .. } if *p == gone || *p == *path)) {
                    this.close_tab(i, window, cx);
                }
            }
        });
        self.tabs.push(Tab::Repo { path, view, _sub: sub });
        self.tabs.len() - 1
    }

    /// Open a repository in a tab (reusing an open one), or register a folder
    /// of repositories on the home tab.
    pub fn open_path(&mut self, path: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        let path = path.canonicalize().unwrap_or(path);
        if !projects::is_repo(&path) {
            if path.is_dir() {
                self.session.add_folder(&path);
                self.scan(path, cx);
                let home = self.tabs.iter().position(|t| matches!(t, Tab::Home)).unwrap_or_else(|| {
                    self.tabs.push(Tab::Home);
                    self.tabs.len() - 1
                });
                self.activate(home, window, cx);
            }
            return;
        }
        self.session.touch_recent(&path);
        if let Some(i) = self.tabs.iter().position(|t| matches!(t, Tab::Repo { path: p, .. } if *p == path)) {
            self.activate(i, window, cx);
            return;
        }
        // A home tab turns into the repository it opened.
        let index = if matches!(self.tabs.get(self.active), Some(Tab::Home)) {
            let i = self.push_repo(path, window, cx);
            let tab = self.tabs.pop().unwrap();
            self.tabs[self.active] = tab;
            let _ = i;
            self.active
        } else {
            self.push_repo(path, window, cx)
        };
        self.activate(index, window, cx);
    }

    fn scan(&mut self, folder: PathBuf, cx: &mut Context<Self>) {
        let target = folder.clone();
        let task = cx.background_spawn(async move { projects::scan_repos(&target) });
        cx.spawn(async move |this, cx| {
            let repos = task.await;
            this.update(cx, |this, cx| {
                this.scans.insert(folder, repos);
                this.refresh_summaries(cx);
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    pub fn prompt_open(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        let paths = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: true,
            prompt: Some("Open".into()),
        });
        cx.spawn_in(_window, async move |this, cx| {
            if let Ok(Ok(Some(paths))) = paths.await {
                this.update_in(cx, |this, window, cx| {
                    for path in paths {
                        this.open_path(path, window, cx);
                    }
                })
                .ok();
            }
        })
        .detach();
    }

    fn activate(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        self.settings = None;
        self.active = index.min(self.tabs.len().saturating_sub(1));
        for (i, tab) in self.tabs.iter().enumerate() {
            if let Tab::Repo { view, .. } = tab {
                view.update(cx, |w, cx| w.set_visible(i == self.active, cx));
            }
        }
        let title = match self.tabs.get(self.active) {
            Some(Tab::Repo { path, .. }) => format!("{} — GitChrono", projects::display_name(path)),
            _ => "GitChrono".into(),
        };
        window.set_window_title(&title);
        self.save_session();
        if matches!(self.tabs.get(self.active), Some(Tab::Home)) {
            self.refresh_summaries(cx);
        }
        cx.notify();
    }

    pub(crate) fn close_tab(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        if index >= self.tabs.len() {
            return;
        }
        self.tabs.remove(index);
        if self.tabs.is_empty() {
            self.tabs.push(Tab::Home);
        }
        let next = if index < self.active || self.active >= self.tabs.len() { self.active.saturating_sub(1) } else { self.active };
        self.activate(next, window, cx);
    }

    pub(crate) fn new_tab(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let i = self.tabs.iter().position(|t| matches!(t, Tab::Home)).unwrap_or_else(|| {
            self.tabs.push(Tab::Home);
            self.tabs.len() - 1
        });
        self.activate(i, window, cx);
    }

    pub(crate) fn active_workspace(&self) -> Option<&Entity<Workspace>> {
        match self.tabs.get(self.active) {
            Some(Tab::Repo { view, .. }) => Some(view),
            _ => None,
        }
    }

    pub(crate) fn auto_update_enabled(&self) -> bool {
        self.session.auto_update.unwrap_or(true)
    }

    /// Changes an app preference stored with the session.
    pub(crate) fn set_pref(&mut self, cx: &mut Context<Self>, change: impl FnOnce(&mut Session)) {
        change(&mut self.session);
        cx.set_global(crate::avatars::Prefs { gravatar: self.session.gravatar });
        self.save_session();
        cx.refresh_windows();
    }

    pub(crate) fn active_repo_path(&self) -> Option<PathBuf> {
        match self.tabs.get(self.active) {
            Some(Tab::Repo { path, .. }) => Some(path.clone()),
            _ => None,
        }
    }

    /// (name, path) of every open repository tab.
    pub(crate) fn tab_entries(&self) -> Vec<(String, PathBuf)> {
        self.tabs
            .iter()
            .filter_map(|t| match t {
                Tab::Repo { path, .. } => Some((projects::display_name(path), path.clone())),
                Tab::Home => None,
            })
            .collect()
    }

    /// Recent and project-folder repositories that aren't open in a tab.
    pub(crate) fn known_repos(&self) -> Vec<PathBuf> {
        let open: Vec<PathBuf> = self.tab_entries().into_iter().map(|(_, p)| p).collect();
        let mut out: Vec<PathBuf> = Vec::new();
        for path in self.session.recent.iter().chain(self.scans.values().flatten()) {
            if !open.contains(path) && !out.contains(path) {
                out.push(path.clone());
            }
        }
        out
    }

    fn cycle(&mut self, delta: isize, window: &mut Window, cx: &mut Context<Self>) {
        let n = self.tabs.len() as isize;
        self.activate(((self.active as isize + delta).rem_euclid(n)) as usize, window, cx);
    }

    fn remove_folder(&mut self, folder: &PathBuf, cx: &mut Context<Self>) {
        self.session.folders.retain(|f| f != folder);
        self.session.collapsed.retain(|f| f != folder);
        self.scans.remove(folder);
        self.save_session();
        cx.notify();
    }

    fn save_session(&mut self) {
        // Without git the tabs weren't restored; saving now would forget them.
        if !matches!(self.git, GitCheck::Ok { .. }) {
            return;
        }
        self.session.tabs = self
            .tabs
            .iter()
            .filter_map(|t| match t {
                Tab::Repo { path, .. } => Some(path.clone()),
                Tab::Home => None,
            })
            .collect();
        let repo_index = self.tabs[..self.active.min(self.tabs.len())].iter().filter(|t| matches!(t, Tab::Repo { .. })).count();
        self.session.active = repo_index;
        self.session.save();
    }

    // ------------------------------------------------------------ rendering

    fn render_tab_bar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .id("tab-bar")
            .h(px(36.))
            .flex_none()
            .flex()
            .items_end()
            .gap_px()
            .px_2()
            .bg(c(theme::PANEL_ALT))
            .border_b_1()
            .border_color(c(theme::BORDER))
            .overflow_x_scroll()
            .children(self.tabs.iter().enumerate().map(|(i, tab)| {
                let selected = i == self.active && self.settings.is_none();
                let label = match tab {
                    Tab::Home => "New tab".to_string(),
                    Tab::Repo { path, .. } => projects::display_name(path),
                };
                div()
                    .id(("tab", i))
                    .h(px(30.))
                    .min_w(px(120.))
                    .max_w(px(220.))
                    .flex()
                    .items_center()
                    .gap_2()
                    .px_3()
                    .rounded_t_md()
                    .text_sm()
                    .cursor_pointer()
                    .when(selected, |d| d.bg(c(theme::BG)).text_color(c(theme::TEXT)))
                    .when(!selected, |d| d.text_color(c(theme::MUTED)).hover(|d| d.bg(c(theme::HOVER))))
                    .child(div().flex_1().overflow_hidden().text_ellipsis().whitespace_nowrap().child(label))
                    .child(
                        div()
                            .id(("tab-close", i))
                            .px_1()
                            .rounded_sm()
                            .text_color(c(theme::FAINT))
                            .hover(|d| d.text_color(c(theme::TEXT)).bg(c(theme::HOVER)))
                            .child("×")
                            .on_click(cx.listener(move |this, _, window, cx| {
                                cx.stop_propagation();
                                this.close_tab(i, window, cx);
                            })),
                    )
                    .on_click(cx.listener(move |this, _, window, cx| this.activate(i, window, cx)))
            }))
            .child(
                div()
                    .id("tab-new")
                    .h(px(30.))
                    .px_3()
                    .flex()
                    .items_center()
                    .text_color(c(theme::MUTED))
                    .cursor_pointer()
                    .hover(|d| d.text_color(c(theme::TEXT)))
                    .child("+")
                    .on_click(cx.listener(|this, _, window, cx| this.new_tab(window, cx))),
            )
            .child(div().flex_1())
            .child(
                div()
                    .id("tab-settings")
                    .h(px(30.))
                    .px_3()
                    .flex()
                    .items_center()
                    .rounded_t_md()
                    .text_color(c(theme::MUTED))
                    .cursor_pointer()
                    .when(self.settings.is_some(), |d| d.bg(c(theme::BG)).text_color(c(theme::TEXT)))
                    .hover(|d| d.text_color(c(theme::TEXT)))
                    .child("⚙  Settings")
                    .on_click(cx.listener(|this, _, window, cx| this.open_settings(this.active_repo_path(), window, cx))),
            )
    }

    /// `base` is the project folder the row sits in; its path is then shown
    /// relative to that folder, and only when the repo is nested deeper.
    /// Reloads branch and status for every repository the new-tab page lists.
    /// Rows keep their previous values until the new ones arrive.
    fn refresh_summaries(&mut self, cx: &mut Context<Self>) {
        let mut paths: Vec<PathBuf> = self.session.recent.iter().filter(|p| projects::is_repo(p)).cloned().collect();
        for repos in self.scans.values() {
            paths.extend(repos.iter().cloned());
        }
        paths.sort();
        paths.dedup();
        for path in paths {
            let dir = path.clone();
            let task = cx.background_spawn(async move { summary::repo_summary(&dir).map_err(|e| SharedString::from(format!("{e:#}"))) });
            cx.spawn(async move |this, cx| {
                let result = task.await;
                this.update(cx, |this, cx| {
                    this.summaries.insert(path, result);
                    cx.notify();
                })
                .ok();
            })
            .detach();
        }
    }

    /// Branch, ahead/behind and change counts, as small chips.
    fn summary_chips(&self, path: &PathBuf) -> Vec<AnyElement> {
        let chip = |text: String, color: u32| {
            div()
                .flex_none()
                .px_1p5()
                .h(px(20.))
                .flex()
                .items_center()
                .gap_1()
                .rounded_sm()
                .bg(c(theme::PANEL_ALT))
                .text_xs()
                .text_color(c(color))
                .child(text)
                .into_any_element()
        };
        match self.summaries.get(path) {
            None => Vec::new(),
            Some(Err(e)) => vec![div()
                .flex_none()
                .text_xs()
                .text_color(c(theme::DEL))
                .truncate()
                .max_w(px(260.))
                .child(format!("⚠ {}", e.lines().last().unwrap_or("can't read this repository")))
                .into_any_element()],
            Some(Ok(s)) => {
                let mut out = Vec::new();
                let branch = match (&s.branch, &s.detached_at) {
                    (Some(b), _) => format!("⎇ {b}"),
                    (None, Some(at)) => format!("⎇ detached at {at}"),
                    (None, None) => "⎇ no commits yet".into(),
                };
                out.push(chip(branch, theme::TEXT));
                if let Some((ahead, behind)) = s.ahead_behind {
                    if ahead > 0 {
                        out.push(chip(format!("↑{ahead}"), theme::ADD));
                    }
                    if behind > 0 {
                        out.push(chip(format!("↓{behind}"), theme::WARN));
                    }
                }
                let changes: Vec<(String, u32)> = [
                    (s.conflicted, "⚠", theme::DEL),
                    (s.modified, "✎", theme::WARN),
                    (s.added, "+", theme::ADD),
                    (s.deleted, "−", theme::DEL),
                ]
                .into_iter()
                .filter(|(n, ..)| *n > 0)
                .map(|(n, sign, color)| (format!("{sign}{n}"), color))
                .collect();
                if !changes.is_empty() {
                    out.push(
                        div()
                            .flex_none()
                            .px_1p5()
                            .h(px(20.))
                            .flex()
                            .items_center()
                            .gap_2()
                            .rounded_sm()
                            .bg(c(theme::PANEL_ALT))
                            .text_xs()
                            .children(changes.into_iter().map(|(text, color)| div().text_color(c(color)).child(text)))
                            .into_any_element(),
                    );
                }
                out
            }
        }
    }

    fn repo_row(&self, id: SharedString, path: &PathBuf, base: Option<&PathBuf>, cx: &mut Context<Self>) -> impl IntoElement {
        let open = path.clone();
        let is_open = self.tabs.iter().any(|t| matches!(t, Tab::Repo { path: p, .. } if p == path));
        let hint = match base {
            Some(base) => path.parent().filter(|parent| parent != base).and_then(|parent| parent.strip_prefix(base).ok()).map(|rel| rel.display().to_string()),
            None => Some(projects::short_path(path)),
        };
        div()
            .id(id)
            .h(px(32.))
            .flex_none()
            .flex()
            .items_center()
            .gap_3()
            .px_3()
            .border_t_1()
            .border_color(c(theme::BORDER))
            .cursor_pointer()
            .hover(|d| d.bg(c(theme::HOVER)))
            .child(div().size(px(6.)).rounded_full().bg(c(if is_open { theme::ACCENT } else { theme::FAINT })))
            .child(div().w(px(170.)).flex_none().text_sm().font_weight(FontWeight::MEDIUM).truncate().child(projects::display_name(path)))
            .child(
                div()
                    .w(px(120.))
                    .flex_none()
                    .text_xs()
                    .text_color(c(theme::MUTED))
                    .truncate()
                    .children(self.summaries.get(path).and_then(|s| s.as_ref().ok()).and_then(|s| s.owner.clone())),
            )
            .children(self.summary_chips(path))
            .child(div().flex_1().min_w_0().text_xs().text_color(c(theme::FAINT)).overflow_hidden().text_ellipsis().whitespace_nowrap().text_right().children(hint))
            .when(is_open, |d| {
                d.child(div().px_1p5().rounded_sm().bg(c(theme::SELECTED)).text_xs().text_color(c(theme::ACCENT)).child("open"))
            })
            .on_click(cx.listener(move |this, _, window, cx| this.open_path(open.clone(), window, cx)))
    }

    fn toggle_group(&mut self, key: PathBuf, cx: &mut Context<Self>) {
        if let Some(i) = self.session.collapsed.iter().position(|k| *k == key) {
            self.session.collapsed.remove(i);
        } else {
            self.session.collapsed.push(key);
        }
        self.save_session();
        cx.notify();
    }

    /// A collapsible card: clickable header (chevron, title, subtitle, actions),
    /// then the rows unless collapsed.
    fn group(&self, id: SharedString, key: PathBuf, collapsed: bool, title: String, subtitle: String, actions: Vec<AnyElement>, rows: Vec<AnyElement>, cx: &mut Context<Self>) -> Div {
        div()
            .mt_5()
            .flex_none()
            .flex()
            .flex_col()
            .rounded_lg()
            .border_1()
            .border_color(c(theme::BORDER))
            .bg(c(theme::PANEL))
            .overflow_hidden()
            .child(
                div()
                    .id(id)
                    .h(px(38.))
                    .flex_none()
                    .flex()
                    .items_center()
                    .gap_2()
                    .px_3()
                    .cursor_pointer()
                    .hover(|d| d.bg(c(theme::HOVER)))
                    .child(div().w(px(14.)).text_base().text_color(c(theme::MUTED)).child(if collapsed { "▸" } else { "▾" }))
                    .child(div().text_sm().font_weight(FontWeight::BOLD).child(title))
                    .child(div().flex_1().min_w_0().text_xs().text_color(c(theme::FAINT)).overflow_hidden().text_ellipsis().whitespace_nowrap().child(subtitle))
                    .children(actions)
                    .on_click(cx.listener(move |this, _, _, cx| this.toggle_group(key.clone(), cx))),
            )
            .when(!collapsed, |d| d.children(rows))
    }

    fn render_home(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let query = self.filter.read(cx).value().to_lowercase();
        let filtering = !query.is_empty();
        let matches = |p: &PathBuf| !filtering || p.to_string_lossy().to_lowercase().contains(&query);
        // While filtering every group is expanded so matches are never hidden.
        let is_collapsed = |key: &PathBuf| !filtering && self.session.collapsed.contains(key);
        let recent: Vec<PathBuf> = self.session.recent.iter().filter(|p| projects::is_repo(p) && matches(p)).take(8).cloned().collect();

        let mut body = div().w_full().max_w(px(820.)).flex().flex_col();
        body = body
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_3()
                    .child(img(logo()).size(px(44.)).flex_none())
                    .child(div().text_2xl().font_weight(FontWeight::BOLD).child("Open a repository")),
            )
            .child(
                div().mt_1().text_sm().text_color(c(theme::MUTED)).child(
                    "Pick a repository, or a parent folder: GitChrono finds every repository inside it and keeps the folder here. Or clone one from a URL.",
                ),
            )
            .child(
                div()
                    .mt_4()
                    .flex()
                    .gap_2()
                    .child(
                        Button::new("home-open")
                            .label("Open repository or folder…")
                            .primary()
                            .on_click(cx.listener(|this, _, window, cx| this.prompt_open(window, cx))),
                    )
                    .child(Button::new("home-clone").label("Clone…").on_click(cx.listener(|this, _, window, cx| this.open_clone(window, cx))))
                    .child(div().flex_1().child(Input::new(&self.filter))),
            );

        if !recent.is_empty() {
            let key = PathBuf::new();
            let collapsed = is_collapsed(&key);
            let rows = recent.iter().enumerate().map(|(i, path)| self.repo_row(format!("recent-{i}").into(), path, None, cx).into_any_element()).collect();
            body = body.child(self.group("group-recent".into(), key, collapsed, "Recent".into(), format!("{}", recent.len()), Vec::new(), rows, cx));
        }

        for (fi, folder) in self.session.folders.iter().enumerate() {
            let repos = self.scans.get(folder);
            let shown: Vec<&PathBuf> = repos.map(|r| r.iter().filter(|p| matches(p)).collect()).unwrap_or_default();
            if filtering && repos.is_some() && shown.is_empty() {
                continue;
            }
            let count = match repos {
                None => "scanning…".to_string(),
                Some(r) if filtering => format!("{} of {}", shown.len(), r.len()),
                Some(r) => format!("{} {}", r.len(), if r.len() == 1 { "repository" } else { "repositories" }),
            };
            let remove = folder.clone();
            let open_all: Vec<PathBuf> = shown.iter().map(|p| (*p).clone()).collect();
            let actions = vec![
                Button::new(("folder-open-all", fi))
                    .label("Open all")
                    .xsmall()
                    .ghost()
                    .disabled(open_all.is_empty() || open_all.len() > 20)
                    .on_click(cx.listener(move |this, _, window, cx| {
                        cx.stop_propagation();
                        for path in &open_all {
                            if !this.tabs.iter().any(|t| matches!(t, Tab::Repo { path: p, .. } if p == path)) {
                                this.push_repo(path.clone(), window, cx);
                                this.session.touch_recent(path);
                            }
                        }
                        let last = this.tabs.len() - 1;
                        this.activate(last, window, cx);
                    }))
                    .into_any_element(),
                Button::new(("folder-remove", fi))
                    .label("Remove")
                    .xsmall()
                    .ghost()
                    .on_click(cx.listener(move |this, _, _, cx| {
                        cx.stop_propagation();
                        this.remove_folder(&remove, cx);
                    }))
                    .into_any_element(),
            ];
            let rows = shown.iter().enumerate().map(|(ri, path)| self.repo_row(format!("folder-{fi}-{ri}").into(), path, Some(folder), cx).into_any_element()).collect();
            body = body.child(self.group(
                format!("group-folder-{fi}").into(),
                folder.clone(),
                is_collapsed(folder),
                projects::display_name(folder),
                format!("{} · {count}", projects::short_path(folder)),
                actions,
                rows,
                cx,
            ));
        }

        div().id("home").size_full().overflow_y_scroll().flex().justify_center().items_start().px_8().py_10().bg(c(theme::BG)).child(body)
    }
}

impl Render for Shell {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // Actions dispatch from the focused element; with nothing focused they
        // would never reach our handlers.
        if window.focused(cx).is_none() {
            self.focus.focus(window, cx);
        }
        let content = if !matches!(self.git, GitCheck::Ok { .. }) {
            crate::git_help::render(&self.git, cx).into_any_element()
        } else if let Some(settings) = self.render_settings(cx) {
            settings.into_any_element()
        } else {
            match self.tabs.get(self.active) {
                Some(Tab::Repo { view, .. }) => view.clone().into_any_element(),
                _ => self.render_home(cx).into_any_element(),
            }
        };
        div()
            .track_focus(&self.focus)
            .size_full()
            .flex()
            .flex_col()
            .bg(c(theme::BG))
            .text_color(c(theme::TEXT))
            .on_action(cx.listener(|this, _: &NewTab, window, cx| this.new_tab(window, cx)))
            .on_action(cx.listener(|this, _: &CloseTab, window, cx| this.close_tab(this.active, window, cx)))
            .on_action(cx.listener(|this, _: &NextTab, window, cx| this.cycle(1, window, cx)))
            .on_action(cx.listener(|this, _: &PrevTab, window, cx| this.cycle(-1, window, cx)))
            .on_action(cx.listener(|this, _: &OpenAny, window, cx| this.prompt_open(window, cx)))
            .on_action(cx.listener(|this, _: &ToggleTerminal, window, cx| {
                if let Some(Tab::Repo { view, .. }) = this.tabs.get(this.active) {
                    view.update(cx, |w, cx| w.toggle_terminal(window, cx));
                }
            }))
            .on_action(cx.listener(|this, _: &Undo, _, cx| {
                if let Some(ws) = this.active_workspace().cloned() {
                    ws.update(cx, |w, cx| w.undo(cx));
                }
            }))
            .on_action(cx.listener(|this, _: &Redo, _, cx| {
                if let Some(ws) = this.active_workspace().cloned() {
                    ws.update(cx, |w, cx| w.redo(cx));
                }
            }))
            .on_action(cx.listener(|this, _: &OpenPalette, window, cx| this.open_palette(window, cx)))
            .on_action(cx.listener(|this, _: &OpenSettings, window, cx| this.open_settings(this.active_repo_path(), window, cx)))
            .on_action(cx.listener(|this, _: &FindCommits, window, cx| {
                if let Some(ws) = this.active_workspace().cloned() {
                    ws.update(cx, |w, cx| w.open_search(window, cx));
                }
            }))
            .relative()
            .child(self.render_tab_bar(cx))
            .child(div().flex_1().min_h_0().child(content))
            .child(self.render_footer(cx))
            .children(self.render_palette(cx))
            .children(self.render_about(cx))
            .children(self.render_clone(cx))
    }
}
