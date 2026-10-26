//! Status bar along the bottom of the window: update status on the left,
//! "About & credits" and the version on the right. Also the update flow and
//! the About dialog.

use std::time::Duration;

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::{Disableable as _, Sizable as _};
use gpui_kit::prelude::*;
use gpui_kit::*;

use crate::shell::Shell;
use crate::theme::{self, c};
use crate::update::{self, Installed, Release};

#[derive(Default)]
pub enum UpdateState {
    #[default]
    Idle,
    Checking,
    /// Only shown after a manual check.
    UpToDate,
    Available(Release),
    Downloading(Release, f32),
    Ready(Release, Installed),
    Failed(SharedString),
}

#[derive(Clone)]
struct Contributor {
    login: String,
    avatar: String,
    page: String,
    commits: u64,
}

#[derive(Default)]
pub struct About {
    contributors: Option<Result<Vec<Contributor>, String>>,
}

/// Projects GitChrono is built on, with their licences.
const CREDITS: &[(&str, &str, &str)] = &[
    ("Git", "used through the git installed on your computer", "https://git-scm.com"),
    ("Git for Windows", "MinGit, included in Windows builds for computers without Git (GPLv2)", "https://gitforwindows.org"),
    ("GPUI", "the UI framework behind the Zed editor (Apache-2.0)", "https://www.gpui.rs"),
    ("gpui-component", "UI components by Longbridge (Apache-2.0)", "https://github.com/longbridge/gpui-component"),
    ("gitoxide", "reads your history quickly (MIT / Apache-2.0)", "https://github.com/GitoxideLabs/gitoxide"),
    ("Alacritty", "terminal emulation (Apache-2.0)", "https://github.com/alacritty/alacritty"),
    ("syntect", "syntax highlighting in diffs (MIT)", "https://github.com/trishume/syntect"),
    ("Gravatar", "profile pictures, when turned on in Settings", "https://gravatar.com"),
    ("Claude Code", "Anthropic's AI coding assistant, which helped build GitChrono", "https://claude.com/claude-code"),
];

/// First check shortly after launch, then every six hours.
const FIRST_CHECK: Duration = Duration::from_secs(8);
const CHECK_EVERY: Duration = Duration::from_secs(6 * 3600);

impl Shell {
    pub(crate) fn schedule_update_checks(&mut self, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(FIRST_CHECK).await;
            loop {
                let Ok(enabled) = this.update(cx, |this, _| this.auto_update_enabled()) else { return };
                if enabled {
                    let _ = this.update(cx, |this, cx| this.check_for_updates(false, cx));
                }
                cx.background_executor().timer(CHECK_EVERY).await;
            }
        })
        .detach();
    }

    pub(crate) fn check_for_updates(&mut self, manual: bool, cx: &mut Context<Self>) {
        if matches!(self.update, UpdateState::Checking | UpdateState::Downloading(..) | UpdateState::Ready(..)) {
            return;
        }
        self.update = UpdateState::Checking;
        cx.notify();
        let task = cx.background_spawn(async move { update::check() });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            this.update(cx, |this, cx| {
                this.update = match result {
                    Ok(Some(release)) => UpdateState::Available(release),
                    Ok(None) if manual => UpdateState::UpToDate,
                    Ok(None) => UpdateState::Idle,
                    // Background checks fail quietly (offline, rate limit).
                    Err(e) if manual => UpdateState::Failed(format!("Couldn't check for updates: {e}").into()),
                    Err(_) => UpdateState::Idle,
                };
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn start_update(&mut self, cx: &mut Context<Self>) {
        let UpdateState::Available(release) = std::mem::take(&mut self.update) else { return };
        self.update = UpdateState::Downloading(release.clone(), 0.);
        cx.notify();
        let (tx, rx) = smol::channel::unbounded::<f32>();
        let task = cx.background_spawn(async move {
            let file = update::download(&release, |p| {
                let _ = tx.try_send(p);
            })?;
            update::install(&file).map(|installed| (release, installed))
        });
        cx.spawn(async move |this, cx| {
            while let Ok(p) = rx.recv().await {
                let alive = this.update(cx, |this, cx| {
                    if let UpdateState::Downloading(_, progress) = &mut this.update {
                        *progress = p;
                        cx.notify();
                    }
                });
                if alive.is_err() {
                    return;
                }
            }
        })
        .detach();
        cx.spawn(async move |this, cx| {
            let result = task.await;
            this.update(cx, |this, cx| {
                this.update = match result {
                    Ok((release, installed)) => UpdateState::Ready(release, installed),
                    Err(e) => UpdateState::Failed(format!("Update failed: {e:#}").into()),
                };
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn restart_into_update(&mut self, cx: &mut Context<Self>) {
        let UpdateState::Ready(_, installed) = &self.update else { return };
        match update::relaunch(installed) {
            Ok(()) => cx.quit(),
            Err(e) => {
                self.update = UpdateState::Failed(format!("Couldn't restart: {e}").into());
                cx.notify();
            }
        }
    }

    pub(crate) fn update_status(&self) -> Option<SharedString> {
        Some(match &self.update {
            UpdateState::Idle => return None,
            UpdateState::Checking => "Checking for updates…".into(),
            UpdateState::UpToDate => format!("GitChrono {} is the latest version", update::current()).into(),
            UpdateState::Available(r) => format!("Version {} is available", r.version).into(),
            UpdateState::Downloading(r, p) => format!("Downloading {}… {:.0}%", r.version, p * 100.).into(),
            UpdateState::Ready(r, _) => format!("Version {} is installed; restart to use it", r.version).into(),
            UpdateState::Failed(e) => e.clone(),
        })
    }

    pub(crate) fn render_footer(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let status = match &self.update {
            UpdateState::Idle => None,
            UpdateState::Available(r) => Some(
                Button::new("update-install")
                    .label(format!("Update to {}", r.version))
                    .xsmall()
                    .outline()
                    .on_click(cx.listener(|this, _, _, cx| this.start_update(cx)))
                    .into_any_element(),
            ),
            UpdateState::Ready(..) => Some(
                div()
                    .id("update-restart")
                    .px_2()
                    .h(px(18.))
                    .flex()
                    .items_center()
                    .rounded_sm()
                    .border_1()
                    .border_color(c(theme::ADD))
                    .bg(c(theme::ADD_BG))
                    .text_color(c(theme::TEXT))
                    .cursor_pointer()
                    .hover(|d| d.bg(c(theme::ADD).opacity(0.3)))
                    .child("Update ready (restart GitChrono)")
                    .on_click(cx.listener(|this, _, _, cx| this.restart_into_update(cx)))
                    .into_any_element(),
            ),
            UpdateState::Failed(e) => Some(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(div().text_color(c(theme::DEL)).truncate().child(e.clone()))
                    .child(Button::new("update-retry").label("Retry").xsmall().ghost().on_click(cx.listener(|this, _, _, cx| this.check_for_updates(true, cx))))
                    .into_any_element(),
            ),
            _ => self.update_status().map(|s| div().child(s).into_any_element()),
        };
        div()
            .h(px(26.))
            .flex_none()
            .flex()
            .items_center()
            .gap_3()
            .px_2()
            .bg(c(theme::PANEL_ALT))
            .border_t_1()
            .border_color(c(theme::BORDER))
            .text_xs()
            .text_color(c(theme::MUTED))
            .children(status)
            .child(div().flex_1())
            .child(Button::new("about-open").label("About & credits").xsmall().ghost().on_click(cx.listener(|this, _, _, cx| this.open_about(cx))))
            .child(div().text_color(c(theme::FAINT)).child(format!("v{}", update::current())))
    }

    pub(crate) fn open_about(&mut self, cx: &mut Context<Self>) {
        self.about = Some(About::default());
        cx.notify();
        let task = cx.background_spawn(async move { fetch_contributors() });
        cx.spawn(async move |this, cx| {
            let result = task.await.map_err(|e| e.to_string());
            this.update(cx, |this, cx| {
                if let Some(about) = &mut this.about {
                    about.contributors = Some(result);
                    cx.notify();
                }
            })
            .ok();
        })
        .detach();
    }

    pub(crate) fn render_about(&self, cx: &mut Context<Self>) -> Option<impl IntoElement> {
        let about = self.about.as_ref()?;
        let link = |id: &'static str, label: &'static str, url: String| {
            Button::new(id).label(label).xsmall().ghost().on_click(move |_, _, cx| cx.open_url(&url))
        };
        let heading = |text: &'static str| div().mt_2().text_xs().font_weight(FontWeight::BOLD).text_color(c(theme::MUTED)).child(text);

        let contributors = match &about.contributors {
            None => div().text_sm().text_color(c(theme::MUTED)).child("Loading contributors…").into_any_element(),
            Some(Err(_)) => div()
                .text_sm()
                .text_color(c(theme::MUTED))
                .child("Couldn't load the contributor list (offline?). See it on GitHub.")
                .into_any_element(),
            Some(Ok(list)) => div()
                .flex()
                .flex_wrap()
                .gap_2()
                .children(list.iter().enumerate().map(|(i, person)| {
                    let page = person.page.clone();
                    div()
                        .id(("contributor", i))
                        .flex()
                        .items_center()
                        .gap_1p5()
                        .px_2()
                        .py_1()
                        .rounded_md()
                        .bg(c(theme::BG))
                        .cursor_pointer()
                        .hover(|d| d.bg(c(theme::HOVER)))
                        .children(crate::avatars::for_url(&person.avatar, cx).map(|path| img(path).size(px(20.)).rounded_full()))
                        .child(div().text_sm().child(person.login.clone()))
                        .child(div().text_xs().text_color(c(theme::FAINT)).child(person.commits.to_string()))
                        .on_click(move |_, _, cx| cx.open_url(&page))
                }))
                .into_any_element(),
        };

        Some(
            div()
                .id("about-backdrop")
                .absolute()
                .inset_0()
                .flex()
                .items_center()
                .justify_center()
                .bg(gpui_kit::black().opacity(0.5))
                .on_click(cx.listener(|this, _, _, cx| {
                    this.about = None;
                    cx.notify();
                }))
                .child(
                    div()
                        .id("about")
                        .w(px(560.))
                        .max_h(relative(0.85))
                        .overflow_y_scroll()
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
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap_3()
                                .child(img(crate::shell::logo()).size(px(56.)).flex_none())
                                .child(
                                    div()
                                        .flex()
                                        .flex_col()
                                        .child(div().text_xl().font_weight(FontWeight::BOLD).child("GitChrono"))
                                        .child(div().text_sm().text_color(c(theme::MUTED)).child(format!("Version {} · free and open source (Apache-2.0)", update::current()))),
                                ),
                        )
                        .child(
                            div()
                                .flex()
                                .flex_wrap()
                                .gap_1()
                                .child(link("about-site", "Website", "https://gitchrono.com".into()))
                                .child(link("about-github", "GitHub", format!("https://github.com/{}", update::REPO)))
                                .child(link("about-issues", "Report an issue", format!("https://github.com/{}/issues/new", update::REPO)))
                                .child(link("about-releases", "Release notes", "https://gitchrono.com/changelog/".into()))
                                .child(Button::new("about-check").label("Check for updates").xsmall().ghost().disabled(matches!(self.update, UpdateState::Checking | UpdateState::Downloading(..))).on_click(cx.listener(|this, _, _, cx| this.check_for_updates(true, cx)))),
                        )
                        .children(self.update_status().map(|s| div().text_xs().text_color(c(theme::MUTED)).child(s)))
                        .children(match &self.git {
                            gitchrono_core::gitbin::GitCheck::Ok { path, version, bundled } => Some(
                                div()
                                    .text_xs()
                                    .text_color(c(theme::FAINT))
                                    .child(format!("Using {version}{} · {}", if *bundled { " (included)" } else { "" }, path.display())),
                            ),
                            _ => None,
                        })
                        .child(heading("CONTRIBUTORS"))
                        .child(contributors)
                        .child(heading("BUILT WITH"))
                        .children(CREDITS.iter().enumerate().map(|(i, (name, what, url))| {
                            let url = url.to_string();
                            div()
                                .id(("credit", i))
                                .flex()
                                .gap_2()
                                .text_sm()
                                .cursor_pointer()
                                .child(div().font_weight(FontWeight::SEMIBOLD).text_color(c(theme::ACCENT)).hover(|d| d.underline()).child(*name))
                                .child(div().text_color(c(theme::MUTED)).child(*what))
                                .on_click(move |_, _, cx| cx.open_url(&url))
                        }))
                        .child(div().flex().justify_end().child(Button::new("about-close").label("Close").small().on_click(cx.listener(|this, _, _, cx| {
                            this.about = None;
                            cx.notify();
                        })))),
                ),
        )
    }
}

fn fetch_contributors() -> anyhow::Result<Vec<Contributor>> {
    let agent: ureq::Agent = ureq::Agent::config_builder().timeout_global(Some(Duration::from_secs(15))).user_agent(concat!("GitChrono/", env!("CARGO_PKG_VERSION"))).build().into();
    let json: serde_json::Value = agent
        .get(&format!("https://api.github.com/repos/{}/contributors?per_page=100", update::REPO))
        .header("Accept", "application/vnd.github+json")
        .call()?
        .body_mut()
        .read_json()?;
    Ok(json
        .as_array()
        .into_iter()
        .flatten()
        .filter(|c| c["type"].as_str() != Some("Bot"))
        .filter_map(|c| {
            Some(Contributor {
                login: c["login"].as_str()?.to_string(),
                avatar: format!("{}&s=40", c["avatar_url"].as_str()?),
                page: c["html_url"].as_str()?.to_string(),
                commits: c["contributions"].as_u64().unwrap_or(0),
            })
        })
        .collect())
}
