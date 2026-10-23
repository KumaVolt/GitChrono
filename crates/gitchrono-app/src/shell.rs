//! The window: a tab per repository plus a "home" tab listing recent
//! repositories and project folders (parent folders with several repos).

use std::collections::HashMap;
use std::path::PathBuf;

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::{Disableable as _, Sizable as _};
use gpui_kit::prelude::*;
use gpui_kit::*;

use crate::projects::{self, Session};
use crate::theme::{self, c};
use crate::workspace::{Workspace, WorkspaceEvent};

actions!(gitchrono, [NewTab, CloseTab, NextTab, PrevTab, OpenAny, ToggleTerminal, OpenPalette, FindCommits, Undo, Redo]);

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
    ]);
}

enum Tab {
    Home,
    Repo { path: PathBuf, view: Entity<Workspace>, _sub: Subscription },
}

pub struct Shell {
    tabs: Vec<Tab>,
    pub(crate) active: usize,
    pub(crate) palette: Option<crate::palette::Palette>,
    session: Session,
    /// Repositories found in each project folder.
    scans: HashMap<PathBuf, Vec<PathBuf>>,
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
                } else if let Some(ws) = this.active_workspace().cloned() {
                    ws.update(cx, |w, cx| w.close_search(cx));
                }
            }
        });
        let session = Session::load();
        let mut this =
            Self { tabs: Vec::new(), active: 0, palette: None, session: session.clone(), scans: HashMap::new(), filter, _filter_sub: filter_sub, focus: cx.focus_handle(), _keystrokes: keystrokes };

        for path in session.tabs.iter().filter(|p| projects::is_repo(p)) {
            this.push_repo(path.clone(), window, cx);
        }
        for folder in session.folders.clone() {
            this.scan(folder, cx);
        }
        this.active = session.active.min(this.tabs.len().saturating_sub(1));
        if let Some(path) = open {
            this.open_path(path, window, cx);
        }
        if this.tabs.is_empty() {
            this.tabs.push(Tab::Home);
        }
        this.activate(this.active, window, cx);
        this
    }

    fn push_repo(&mut self, path: PathBuf, window: &mut Window, cx: &mut Context<Self>) -> usize {
        let open = path.clone();
        let view = cx.new(|cx| Workspace::new(Some(open), window, cx));
        let sub = cx.subscribe_in(&view, window, |this, _, event: &WorkspaceEvent, window, cx| match event {
            WorkspaceEvent::OpenRequested => this.prompt_open(window, cx),
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
        self.scans.remove(folder);
        self.save_session();
        cx.notify();
    }

    fn save_session(&mut self) {
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
                let selected = i == self.active;
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
    }

    fn repo_row(&self, id: SharedString, path: &PathBuf, cx: &mut Context<Self>) -> impl IntoElement {
        let open = path.clone();
        let is_open = self.tabs.iter().any(|t| matches!(t, Tab::Repo { path: p, .. } if p == path));
        div()
            .id(id)
            .flex()
            .items_center()
            .gap_3()
            .px_3()
            .py_1p5()
            .rounded_md()
            .cursor_pointer()
            .hover(|d| d.bg(c(theme::HOVER)))
            .child(div().text_sm().font_weight(FontWeight::SEMIBOLD).child(projects::display_name(path)))
            .child(div().flex_1().text_xs().text_color(c(theme::FAINT)).overflow_hidden().whitespace_nowrap().child(projects::short_path(path)))
            .when(is_open, |d| d.child(div().text_xs().text_color(c(theme::ACCENT)).child("open")))
            .on_click(cx.listener(move |this, _, window, cx| this.open_path(open.clone(), window, cx)))
    }

    fn render_home(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let query = self.filter.read(cx).value().to_lowercase();
        let matches = |p: &PathBuf| query.is_empty() || p.to_string_lossy().to_lowercase().contains(&query);
        let recent: Vec<PathBuf> = self.session.recent.iter().filter(|p| projects::is_repo(p) && matches(p)).take(8).cloned().collect();

        let section = |title: String| {
            div().mt_6().mb_2().px_3().text_xs().font_weight(FontWeight::BOLD).text_color(c(theme::MUTED)).child(title)
        };

        let mut body = div().w_full().max_w(px(820.)).flex().flex_col();
        body = body
            .child(div().text_2xl().font_weight(FontWeight::BOLD).child("Open a repository"))
            .child(
                div().mt_1().text_sm().text_color(c(theme::MUTED)).child(
                    "Pick a repository, or a parent folder: GitChrono finds every repository inside it and keeps the folder here.",
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
                    .child(div().flex_1().child(Input::new(&self.filter))),
            );

        if !recent.is_empty() {
            body = body.child(section("RECENT".into()));
            for (i, path) in recent.iter().enumerate() {
                body = body.child(self.repo_row(format!("recent-{i}").into(), path, cx));
            }
        }

        for (fi, folder) in self.session.folders.iter().enumerate() {
            let repos = self.scans.get(folder);
            let shown: Vec<&PathBuf> = repos.map(|r| r.iter().filter(|p| matches(p)).collect()).unwrap_or_default();
            let count = repos.map_or_else(|| "scanning…".to_string(), |r| format!("{} repositories", r.len()));
            let remove = folder.clone();
            let open_all: Vec<PathBuf> = shown.iter().map(|p| (*p).clone()).collect();
            body = body.child(
                div()
                    .mt_6()
                    .mb_2()
                    .px_3()
                    .flex()
                    .items_center()
                    .gap_3()
                    .child(div().text_xs().font_weight(FontWeight::BOLD).text_color(c(theme::MUTED)).child(projects::display_name(folder).to_uppercase()))
                    .child(div().flex_1().text_xs().text_color(c(theme::FAINT)).child(format!("{} · {count}", projects::short_path(folder))))
                    .child(
                        Button::new(("folder-open-all", fi))
                            .label("Open all")
                            .xsmall()
                            .ghost()
                            .disabled(open_all.is_empty() || open_all.len() > 20)
                            .on_click(cx.listener(move |this, _, window, cx| {
                                for path in &open_all {
                                    if !this.tabs.iter().any(|t| matches!(t, Tab::Repo { path: p, .. } if p == path)) {
                                        this.push_repo(path.clone(), window, cx);
                                        this.session.touch_recent(path);
                                    }
                                }
                                let last = this.tabs.len() - 1;
                                this.activate(last, window, cx);
                            })),
                    )
                    .child(
                        Button::new(("folder-remove", fi))
                            .label("Remove")
                            .xsmall()
                            .ghost()
                            .on_click(cx.listener(move |this, _, _, cx| this.remove_folder(&remove, cx))),
                    ),
            );
            for (ri, path) in shown.iter().enumerate() {
                body = body.child(self.repo_row(format!("folder-{fi}-{ri}").into(), path, cx));
            }
        }

        div().id("home").size_full().overflow_y_scroll().flex().justify_center().px_8().py_10().bg(c(theme::BG)).child(body)
    }
}

impl Render for Shell {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // Actions dispatch from the focused element; with nothing focused they
        // would never reach our handlers.
        if window.focused(cx).is_none() {
            self.focus.focus(window, cx);
        }
        let content = match self.tabs.get(self.active) {
            Some(Tab::Repo { view, .. }) => view.clone().into_any_element(),
            _ => self.render_home(cx).into_any_element(),
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
            .on_action(cx.listener(|this, _: &FindCommits, window, cx| {
                if let Some(ws) = this.active_workspace().cloned() {
                    ws.update(cx, |w, cx| w.open_search(window, cx));
                }
            }))
            .relative()
            .child(self.render_tab_bar(cx))
            .child(div().flex_1().min_h_0().child(content))
            .children(self.render_palette(cx))
    }
}
