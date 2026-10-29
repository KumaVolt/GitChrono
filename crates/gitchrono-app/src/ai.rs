//! AI actions (commit message, branch and stash names) and the provider
//! settings dialog. Requests run off the UI thread and fill inputs when done.

use anyhow::{Result, bail};
use gitchrono_ai::{AiConfig, PRESETS, Request};
use gitchrono_core::Repo;
use gpui_kit::component::button::ButtonVariants as _;
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::Sizable as _;
use gpui_kit::prelude::*;
use gpui_kit::*;

use crate::theme::{self, c};
use crate::workspace::{Selection, Suggest, Workspace};
use crate::ui::button;

/// The saved AI configuration, shared by every tab so they all agree on it.
pub struct AiSetup {
    /// `None` until a provider is saved.
    pub config: Option<AiConfig>,
    /// A provider is chosen and has the key it needs.
    pub ready: bool,
}

impl Global for AiSetup {}

impl AiSetup {
    fn new(config: Option<AiConfig>) -> Self {
        let ready = config.as_ref().is_some_and(|c| !c.needs_key() || c.key().is_some());
        Self { config, ready }
    }
}

pub fn init(cx: &mut App) {
    cx.set_global(AiSetup::new(AiConfig::load().filter(|c| c.enabled)));
}

pub struct AiSettings {
    preset: usize,
    model: Entity<InputState>,
    base_url: Entity<InputState>,
    key: Entity<InputState>,
    has_stored_key: bool,
    error: Option<SharedString>,
}

impl Workspace {
    pub fn open_ai_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let config = self.ai.clone().unwrap_or_else(|| AiConfig::from_preset(&PRESETS[0]));
        let preset = PRESETS.iter().position(|p| p.id == config.provider).unwrap_or(0);
        let model = cx.new(|cx| InputState::new(window, cx).default_value(config.model.clone()).autocorrect(false));
        let base_url = cx.new(|cx| InputState::new(window, cx).default_value(config.base_url.clone()).autocorrect(false));
        let key = cx.new(|cx| InputState::new(window, cx).masked(true).autocorrect(false));
        self.ai_settings = Some(AiSettings {
            preset,
            model,
            base_url,
            key,
            has_stored_key: gitchrono_ai::load_key(&config.provider).is_some(),
            error: None,
        });
        self.refresh_key_placeholder(window, cx);
        cx.notify();
    }

    fn refresh_key_placeholder(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(s) = &self.ai_settings else { return };
        let preset = &PRESETS[s.preset];
        let placeholder = if !preset.needs_key {
            "No key needed".to_string()
        } else if s.has_stored_key {
            "•••••••• saved in keychain (type to replace)".to_string()
        } else {
            format!("API key (or set {})", preset.env_key)
        };
        s.key.update(cx, |k, cx| {
            k.set_value("", window, cx);
            k.set_placeholder(placeholder, window, cx);
        });
    }

    fn pick_preset(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(s) = &mut self.ai_settings else { return };
        let preset = &PRESETS[index];
        s.preset = index;
        s.has_stored_key = gitchrono_ai::load_key(preset.id).is_some();
        s.error = None;
        s.model.update(cx, |i, cx| i.set_value(preset.model, window, cx));
        s.base_url.update(cx, |i, cx| i.set_value(preset.base_url, window, cx));
        self.refresh_key_placeholder(window, cx);
        cx.notify();
    }

    fn save_ai_settings(&mut self, cx: &mut Context<Self>) {
        let Some(s) = &mut self.ai_settings else { return };
        let preset = &PRESETS[s.preset];
        let mut config = AiConfig::from_preset(preset);
        let model = s.model.read(cx).value().trim().to_string();
        let base_url = s.base_url.read(cx).value().trim().to_string();
        if !model.is_empty() {
            config.model = model;
        }
        if !base_url.is_empty() {
            config.base_url = base_url;
        }
        let key = s.key.read(cx).value().trim().to_string();
        let result = (|| -> Result<()> {
            if !key.is_empty() {
                gitchrono_ai::save_key(preset.id, &key)?;
            }
            config.save()
        })();
        match result {
            Ok(()) => {
                cx.set_global(AiSetup::new(Some(config)));
                self.ai_settings = None;
            }
            Err(e) => s.error = Some(format!("{e:#}").into()),
        }
        cx.notify();
    }

    fn remove_ai_key(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(s) = &mut self.ai_settings else { return };
        match gitchrono_ai::delete_key(PRESETS[s.preset].id) {
            Ok(()) => s.has_stored_key = false,
            Err(e) => s.error = Some(format!("{e:#}").into()),
        }
        cx.set_global(AiSetup::new(self.ai.clone()));
        self.refresh_key_placeholder(window, cx);
        cx.notify();
    }

    /// Run one completion in the background; `done` receives the model's text.
    fn ai_request(
        &mut self,
        label: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
        build: impl FnOnce(&Repo) -> Result<Request<'static>> + Send + 'static,
        done: impl FnOnce(&mut Self, String, &mut Window, &mut Context<Self>) + 'static,
    ) {
        let Some(config) = self.ai.clone() else {
            self.open_ai_settings(window, cx);
            return;
        };
        let Some(repo) = self.repo.clone() else { return };
        if self.ai_busy.is_some() {
            return;
        }
        let key = config.key();
        if key.is_none() && config.needs_key() {
            self.open_ai_settings(window, cx);
            return;
        }
        self.ai_busy = Some(label.to_string().into());
        self.error = None;
        cx.notify();
        let task = cx.background_spawn(async move {
            let request = build(&repo)?;
            gitchrono_ai::complete(&config, key.as_deref(), &request)
        });
        self.ai_task = Some(cx.spawn_in(window, async move |this, cx| {
            let result = task.await;
            this.update_in(cx, |this, window, cx| {
                this.ai_busy = None;
                match result {
                    Ok(completion) => {
                        this.ai_usage = Some(
                            format!("{} in · {} out tokens", completion.input_tokens, completion.output_tokens).into(),
                        );
                        done(this, completion.text, window, cx);
                    }
                    Err(e) => this.error = Some(format!("AI: {e:#}").into()),
                }
                cx.notify();
            })
            .ok();
        }));
    }

    pub fn generate_commit_message(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let recent: Vec<String> = self.snap.commits.iter().take(30).map(|c| c.summary.to_string()).collect();
        let branch = self.status.branch.clone();
        self.ai_request(
            "Writing message…",
            window,
            cx,
            move |repo| {
                let diff = repo.staged_patch()?;
                if diff.trim().is_empty() {
                    bail!("stage some changes first");
                }
                let recent: Vec<&str> = recent.iter().map(String::as_str).collect();
                Ok(gitchrono_ai::commit_message(&diff, &recent, branch.as_deref()))
            },
            |this, text, window, cx| {
                let message = gitchrono_ai::parse_commit_message(&text);
                this.summary.update(cx, |s, cx| s.set_value(message.summary, window, cx));
                this.description.update(cx, |s, cx| s.set_value(message.body, window, cx));
            },
        );
    }

    /// Explain the selected commit in a few sentences, shown in the detail panel.
    pub fn explain_commit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Selection::Commit(i) = self.selection else { return };
        let Some(commit) = self.snap.commits.get(i).cloned() else { return };
        let id = commit.id.clone();
        self.ai_request(
            "Explaining…",
            window,
            cx,
            move |repo| Ok(gitchrono_ai::explain_commit(&commit.summary, &commit.body, &repo.commit_patch(&commit.id)?)),
            move |this, text, _, _| this.explanation = Some((id, text.trim().to_string().into())),
        );
    }

    /// Fill the open prompt's input with a suggested branch name or stash message.
    pub fn suggest_for_prompt(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(kind) = self.prompt.as_ref().and_then(|p| p.suggest) else { return };
        self.ai_request(
            "Suggesting…",
            window,
            cx,
            move |repo| {
                let diff = repo.workdir_patch()?;
                if diff.trim().is_empty() {
                    bail!("there are no changes to describe");
                }
                Ok(match kind {
                    Suggest::BranchName => gitchrono_ai::branch_name(&diff),
                    Suggest::StashMessage => gitchrono_ai::stash_message(&diff),
                })
            },
            move |this, text, window, cx| {
                let value = match kind {
                    Suggest::BranchName => gitchrono_ai::sanitize_ref_name(&text),
                    Suggest::StashMessage => gitchrono_ai::parse_commit_message(&text).summary,
                };
                if let Some(input) = this.prompt.as_ref().and_then(|p| p.input.clone()) {
                    input.update(cx, |i, cx| i.set_value(value, window, cx));
                }
            },
        );
    }

    pub fn render_ai_settings(&self, cx: &mut Context<Self>) -> Option<impl IntoElement> {
        let s = self.ai_settings.as_ref()?;
        let preset = &PRESETS[s.preset];
        let label = |text: &'static str| div().text_xs().text_color(c(theme::MUTED)).child(text);
        Some(
            div()
                .id("ai-backdrop")
                .absolute()
                .inset_0()
                .flex()
                .items_center()
                .justify_center()
                .bg(gpui_kit::black().opacity(0.5))
                .on_click(cx.listener(|this, _, _, cx| {
                    this.ai_settings = None;
                    cx.notify();
                }))
                .child(
                    div()
                        .id("ai-settings")
                        .w(px(480.))
                        .p_4()
                        .flex()
                        .flex_col()
                        .gap_3()
                        .rounded_lg()
                        .bg(c(theme::PANEL))
                        .border_1()
                        .border_color(c(theme::BORDER))
                        .shadow_lg()
                        .on_click(|_, _, cx| cx.stop_propagation())
                        .child(div().text_base().font_weight(FontWeight::BOLD).child("AI settings"))
                        .child(div().text_sm().text_color(c(theme::MUTED)).child(
                            "Bring your own key: requests go straight from GitChrono to your provider. \
                             Diffs are trimmed, lockfiles skipped and obvious secrets redacted first.",
                        ))
                        .child(label("Provider"))
                        .child(div().flex().flex_wrap().gap_1().children(PRESETS.iter().enumerate().map(|(i, p)| {
                            button(("ai-preset", i))
                                .label(p.name)
                                .small()
                                .when(i == s.preset, |b| b.primary())
                                .when(i != s.preset, |b| b.ghost())
                                .on_click(cx.listener(move |this, _, window, cx| this.pick_preset(i, window, cx)))
                        })))
                        .child(label("Model"))
                        .child(Input::new(&s.model))
                        .child(label("Endpoint"))
                        .child(Input::new(&s.base_url))
                        .when(preset.needs_key, |d| d.child(label("API key")).child(Input::new(&s.key).mask_toggle()))
                        .when_some(s.error.clone(), |d, e| d.child(div().text_sm().text_color(c(theme::DEL)).child(e)))
                        .child(
                            div()
                                .flex()
                                .gap_2()
                                .when(s.has_stored_key, |d| {
                                    d.child(button("ai-remove-key").label("Remove key").small().ghost().on_click(
                                        cx.listener(|this, _, window, cx| this.remove_ai_key(window, cx)),
                                    ))
                                })
                                .child(div().flex_1())
                                .child(button("ai-cancel").label("Cancel").small().ghost().on_click(cx.listener(
                                    |this, _, _, cx| {
                                        this.ai_settings = None;
                                        cx.notify();
                                    },
                                )))
                                .child(
                                    button("ai-save")
                                        .label("Save")
                                        .small()
                                        .primary()
                                        .on_click(cx.listener(|this, _, _, cx| this.save_ai_settings(cx))),
                                ),
                        ),
                ),
        )
    }
}
