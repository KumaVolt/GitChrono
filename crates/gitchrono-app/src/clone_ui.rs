//! "Clone repository…": URL, destination folder and name, then `git clone`
//! off the UI thread with its progress shown; the result opens in a tab.

use std::path::PathBuf;

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::{Disableable as _, Sizable as _};
use gpui_kit::prelude::*;
use gpui_kit::*;

use crate::projects;
use crate::shell::Shell;
use crate::theme::{self, c};

pub struct CloneDialog {
    url: Entity<InputState>,
    name: Entity<InputState>,
    /// Folder the clone goes into (as `<parent>/<name>`).
    parent: PathBuf,
    /// The name last filled in from the URL; while the field still holds it,
    /// editing the URL keeps it in sync.
    auto_name: String,
    /// git's latest progress line while cloning.
    progress: Option<SharedString>,
    error: Option<SharedString>,
    _subs: Vec<Subscription>,
}

impl CloneDialog {
    fn target(&self, cx: &App) -> Option<(String, PathBuf)> {
        let url = self.url.read(cx).value().trim().to_string();
        let name = self.name.read(cx).value().trim().to_string();
        (!url.is_empty() && !name.is_empty()).then(|| (url, self.parent.join(name)))
    }
}

impl Shell {
    pub(crate) fn open_clone(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.clone_dialog.as_ref().is_some_and(|d| d.progress.is_some()) {
            return;
        }
        let url = cx.new(|cx| InputState::new(window, cx).placeholder("https://github.com/owner/repo.git or git@host:owner/repo.git").autocorrect(false));
        let name = cx.new(|cx| InputState::new(window, cx).placeholder("Folder name").autocorrect(false));
        let on_enter = |this: &mut Self, event: &InputEvent, window: &mut Window, cx: &mut Context<Self>| {
            if let InputEvent::PressEnter { .. } = event {
                this.start_clone(window, cx);
            }
        };
        let subs = vec![
            cx.subscribe_in(&url, window, move |this, _, event: &InputEvent, window, cx| {
                if let InputEvent::Change = event {
                    this.sync_clone_name(window, cx);
                }
                on_enter(this, event, window, cx);
            }),
            cx.subscribe_in(&name, window, move |this, _, event: &InputEvent, window, cx| {
                if let InputEvent::Change = event {
                    cx.notify();
                }
                on_enter(this, event, window, cx);
            }),
        ];
        url.update(cx, |i, cx| i.focus(window, cx));
        let parent = self.default_clone_parent();
        self.palette = None;
        self.clone_dialog = Some(CloneDialog { url, name, parent, auto_name: String::new(), progress: None, error: None, _subs: subs });
        cx.notify();
    }

    pub(crate) fn open_clone_with(&mut self, url: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.open_clone(window, cx);
        if let Some(d) = &self.clone_dialog {
            d.url.update(cx, |i, cx| i.set_value(url.to_string(), window, cx));
        }
        self.sync_clone_name(window, cx);
    }

    /// Last folder cloned into, else the folder of the most recent repository, else home.
    fn default_clone_parent(&self) -> PathBuf {
        self.session
            .clone_dir
            .clone()
            .filter(|d| d.is_dir())
            .or_else(|| self.session.recent.iter().find_map(|r| r.parent().filter(|p| p.is_dir()).map(PathBuf::from)))
            .or_else(dirs::home_dir)
            .unwrap_or_default()
    }

    fn sync_clone_name(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(d) = &mut self.clone_dialog else { return };
        let current = d.name.read(cx).value().to_string();
        if current == d.auto_name {
            let suggested = gitchrono_core::repo_name_from_url(&d.url.read(cx).value()).unwrap_or_default();
            if suggested != current {
                d.auto_name = suggested.clone();
                d.name.update(cx, |i, cx| i.set_value(suggested, window, cx));
            }
        }
        d.error = None;
        cx.notify();
    }

    pub(crate) fn close_clone(&mut self, cx: &mut Context<Self>) {
        // A running clone can't be stopped halfway; the dialog stays until it ends.
        if self.clone_dialog.as_ref().is_some_and(|d| d.progress.is_none()) {
            self.clone_dialog = None;
            cx.notify();
        }
    }

    fn choose_clone_parent(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let paths = cx.prompt_for_paths(PathPromptOptions { files: false, directories: true, multiple: false, prompt: Some("Choose".into()) });
        cx.spawn_in(window, async move |this, cx| {
            if let Ok(Ok(Some(mut paths))) = paths.await
                && let Some(path) = paths.pop()
            {
                this.update(cx, |this, cx| {
                    if let Some(d) = &mut this.clone_dialog {
                        d.parent = path;
                        d.error = None;
                        cx.notify();
                    }
                })
                .ok();
            }
        })
        .detach();
    }

    fn start_clone(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(d) = &mut self.clone_dialog else { return };
        if d.progress.is_some() {
            return;
        }
        let Some((url, dest)) = d.target(cx) else { return };
        if dest.exists() {
            d.error = Some(format!("{} already exists. Pick another name or folder.", projects::short_path(&dest)).into());
            cx.notify();
            return;
        }
        d.progress = Some("Starting…".into());
        d.error = None;
        cx.notify();

        let (tx, rx) = smol::channel::unbounded::<String>();
        let target = dest.clone();
        let task = cx.background_spawn(async move {
            gitchrono_core::clone_repo(&url, &target, |line| {
                tx.try_send(line.to_string()).ok();
            })
        });
        // Show the newest progress line; git prints many per second.
        cx.spawn(async move |this, cx| {
            while let Ok(mut line) = rx.recv().await {
                while let Ok(next) = rx.try_recv() {
                    line = next;
                }
                let alive = this.update(cx, |this, cx| {
                    if let Some(d) = &mut this.clone_dialog
                        && d.progress.is_some()
                    {
                        d.progress = Some(line.into());
                        cx.notify();
                    }
                });
                if alive.is_err() {
                    break;
                }
            }
        })
        .detach();
        cx.spawn_in(window, async move |this, cx| {
            let result = task.await;
            this.update_in(cx, |this, window, cx| match result {
                Ok(()) => {
                    this.session.clone_dir = dest.parent().map(PathBuf::from);
                    this.clone_dialog = None;
                    this.open_path(dest, window, cx);
                }
                Err(e) => {
                    if let Some(d) = &mut this.clone_dialog {
                        d.progress = None;
                        d.error = Some(clone_error(&e).into());
                    }
                    cx.notify();
                }
            })
            .ok();
        })
        .detach();
    }

    pub(crate) fn render_clone(&self, cx: &mut Context<Self>) -> Option<impl IntoElement> {
        let d = self.clone_dialog.as_ref()?;
        let busy = d.progress.is_some();
        let target = d.target(cx);
        let label = |text: &'static str| div().text_xs().font_weight(FontWeight::BOLD).text_color(c(theme::MUTED)).child(text);
        let hint = match &target {
            Some((_, dest)) => format!("Clones into {}", projects::short_path(dest)),
            None => "Paste the address from GitHub, GitLab, Bitbucket or your own server.".into(),
        };

        Some(
            div()
                .id("clone-backdrop")
                .absolute()
                .inset_0()
                .flex()
                .items_center()
                .justify_center()
                .bg(gpui_kit::black().opacity(0.5))
                .on_click(cx.listener(|this, _, _, cx| this.close_clone(cx)))
                .child(
                    div()
                        .id("clone")
                        .w(px(560.))
                        .p_5()
                        .flex()
                        .flex_col()
                        .gap_2()
                        .rounded_lg()
                        .bg(c(theme::PANEL))
                        .border_1()
                        .border_color(c(theme::BORDER))
                        .shadow_lg()
                        .on_click(|_, _, cx| cx.stop_propagation())
                        .child(div().text_xl().font_weight(FontWeight::BOLD).child("Clone a repository"))
                        .child(label("Repository URL"))
                        .child(Input::new(&d.url).disabled(busy))
                        .child(div().mt_1().child(label("Clone into")))
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap_2()
                                .child(
                                    div()
                                        .flex_1()
                                        .min_w_0()
                                        .h(px(30.))
                                        .px_2()
                                        .flex()
                                        .items_center()
                                        .rounded_md()
                                        .bg(c(theme::BG))
                                        .border_1()
                                        .border_color(c(theme::BORDER))
                                        .text_sm()
                                        .overflow_hidden()
                                        .text_ellipsis()
                                        .whitespace_nowrap()
                                        .child(projects::short_path(&d.parent)),
                                )
                                .child(
                                    Button::new("clone-choose")
                                        .label("Choose…")
                                        .small()
                                        .disabled(busy)
                                        .on_click(cx.listener(|this, _, window, cx| this.choose_clone_parent(window, cx))),
                                ),
                        )
                        .child(div().mt_1().child(label("Folder name")))
                        .child(Input::new(&d.name).disabled(busy))
                        .child(div().text_xs().text_color(c(theme::FAINT)).truncate().child(hint))
                        .children(d.progress.clone().map(|p| div().text_sm().text_color(c(theme::ACCENT)).truncate().child(format!("Cloning · {p}"))))
                        .children(d.error.clone().map(|e| div().text_sm().text_color(c(theme::DEL)).child(e)))
                        .child(
                            div()
                                .mt_2()
                                .flex()
                                .justify_end()
                                .gap_2()
                                .child(Button::new("clone-cancel").label("Cancel").ghost().disabled(busy).on_click(cx.listener(|this, _, _, cx| this.close_clone(cx))))
                                .child(
                                    Button::new("clone-start")
                                        .label(if busy { "Cloning…" } else { "Clone" })
                                        .primary()
                                        .disabled(busy || target.is_none())
                                        .on_click(cx.listener(|this, _, window, cx| this.start_clone(window, cx))),
                                ),
                        ),
                ),
        )
    }
}

/// git's own explanation, without the "git clone … failed:" wrapper.
fn clone_error(e: &anyhow::Error) -> String {
    match e.downcast_ref::<gitchrono_core::GitError>() {
        Some(g) if !g.stderr.is_empty() => g.stderr.lines().filter(|l| !l.trim().is_empty()).collect::<Vec<_>>().join("\n"),
        _ => format!("{e:#}"),
    }
}
