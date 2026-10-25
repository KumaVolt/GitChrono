//! Settings page (⌘,): git identity, defaults and commit signing for every
//! repository or just the active one, plus the SSH keys in `~/.ssh`.

use std::path::PathBuf;

use gitchrono_core::setup::{self, GitSetup, Scope, SshKey};
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::{Disableable as _, Sizable as _};
use gpui_kit::prelude::*;
use gpui_kit::*;

use crate::projects;
use crate::shell::Shell;
use crate::theme::{self, c};

pub struct Settings {
    scope: Scope,
    /// The active tab's repository, offered as the per-repository scope.
    repo: Option<PathBuf>,
    /// Values as read, so saving only touches what changed.
    loaded: GitSetup,
    /// What git uses when this scope leaves a value unset.
    inherited: GitSetup,
    name: Entity<InputState>,
    email: Entity<InputState>,
    default_branch: Entity<InputState>,
    signing_key: Entity<InputState>,
    pull_rebase: Option<bool>,
    sign_commits: Option<bool>,
    signing_format: String,
    keys: Vec<SshKey>,
    passphrase: Entity<InputState>,
    busy: Option<&'static str>,
    message: Option<(bool, SharedString)>,
    /// Shown after a key is generated: offer to add it on GitHub.
    created: bool,
}

fn ssh_dir() -> PathBuf {
    dirs::home_dir().unwrap_or_default().join(".ssh")
}

impl Shell {
    pub fn open_settings(&mut self, repo: Option<PathBuf>, window: &mut Window, cx: &mut Context<Self>) {
        let input = |window: &mut Window, cx: &mut Context<Self>| cx.new(|cx| InputState::new(window, cx).autocorrect(false));
        let mut settings = Settings {
            scope: Scope::Global,
            repo,
            loaded: GitSetup::default(),
            inherited: GitSetup::default(),
            name: input(window, cx),
            email: input(window, cx),
            default_branch: input(window, cx),
            signing_key: input(window, cx),
            pull_rebase: None,
            sign_commits: None,
            signing_format: String::new(),
            keys: setup::ssh_keys(&ssh_dir()),
            passphrase: cx.new(|cx| InputState::new(window, cx).masked(true).placeholder("Passphrase (optional)").autocorrect(false)),
            busy: None,
            message: None,
            created: false,
        };
        load_scope(&mut settings, Scope::Global, window, cx);
        self.settings = Some(settings);
        cx.notify();
    }

    pub fn close_settings(&mut self, cx: &mut Context<Self>) {
        self.settings = None;
        cx.notify();
    }

    fn settings_scope(&mut self, scope: Scope, window: &mut Window, cx: &mut Context<Self>) {
        let Some(s) = &mut self.settings else { return };
        load_scope(s, scope, window, cx);
        s.message = None;
        cx.notify();
    }

    fn save_settings(&mut self, cx: &mut Context<Self>) {
        let Some(s) = &mut self.settings else { return };
        let value = |input: &Entity<InputState>| input.read(cx).value().trim().to_string();
        let after = GitSetup {
            name: value(&s.name),
            email: value(&s.email),
            default_branch: value(&s.default_branch),
            pull_rebase: s.pull_rebase,
            sign_commits: s.sign_commits,
            signing_format: s.signing_format.clone(),
            signing_key: value(&s.signing_key),
        };
        if after.sign_commits == Some(true) && after.signing_key.is_empty() && setup::read_setup(&Scope::Global).signing_key.is_empty() {
            s.message = Some((false, "Pick a signing key first (\"Use for signing\" on a key below).".into()));
            cx.notify();
            return;
        }
        match setup::write_setup(&s.scope, &s.loaded, &after) {
            Ok(()) => self.settings = None,
            Err(e) => s.message = Some((false, format!("{e:#}").into())),
        }
        cx.notify();
    }

    fn use_key_for_signing(&mut self, key: &SshKey, window: &mut Window, cx: &mut Context<Self>) {
        let Some(s) = &mut self.settings else { return };
        let path = projects::short_path(&key.public);
        s.signing_key.update(cx, |i, cx| i.set_value(path, window, cx));
        s.signing_format = "ssh".into();
        s.sign_commits = Some(true);
        s.message = Some((true, "Signing set to this key. Save to apply.".into()));
        cx.notify();
    }

    fn copy_public_key(&mut self, key: &SshKey, cx: &mut Context<Self>) {
        let Some(s) = &mut self.settings else { return };
        s.message = Some(match std::fs::read_to_string(&key.public) {
            Ok(text) => {
                cx.write_to_clipboard(ClipboardItem::new_string(text.trim().to_string()));
                (true, format!("Copied {} to the clipboard.", projects::short_path(&key.public)).into())
            }
            Err(e) => (false, format!("Can't read {}: {e}", key.public.display()).into()),
        });
        cx.notify();
    }

    fn generate_key(&mut self, cx: &mut Context<Self>) {
        let Some(s) = &mut self.settings else { return };
        let mut comment = s.email.read(cx).value().trim().to_string();
        if comment.is_empty() {
            comment = setup::read_setup(&Scope::Global).email;
        }
        let passphrase = s.passphrase.read(cx).value().to_string();
        let path = setup::free_key_path(&ssh_dir());
        s.busy = Some("generate");
        s.message = None;
        let task = cx.background_spawn(async move { setup::generate_ssh_key(&path, &comment, &passphrase) });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            this.update(cx, |this, cx| {
                let Some(s) = &mut this.settings else { return };
                s.busy = None;
                s.keys = setup::ssh_keys(&ssh_dir());
                match result {
                    Ok(key) => {
                        let text = std::fs::read_to_string(&key.public).unwrap_or_default();
                        cx.write_to_clipboard(ClipboardItem::new_string(text.trim().to_string()));
                        let file = key.private.file_name().map(|f| f.to_string_lossy().into_owned()).unwrap_or_default();
                        // ssh only tries the default file names on its own.
                        let hint = if file == "id_ed25519" { String::new() } else { format!(" Add `IdentityFile {}` to ~/.ssh/config for your host.", projects::short_path(&key.private)) };
                        s.message = Some((true, format!("Created {}; public key copied.{hint}", projects::short_path(&key.private)).into()));
                        s.created = true;
                    }
                    Err(e) => s.message = Some((false, format!("{e:#}").into())),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
        cx.notify();
    }

    fn test_host(&mut self, host: &'static str, cx: &mut Context<Self>) {
        let Some(s) = &mut self.settings else { return };
        s.busy = Some(host);
        s.message = None;
        let task = cx.background_spawn(async move { setup::test_ssh(host) });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            this.update(cx, |this, cx| {
                let Some(s) = &mut this.settings else { return };
                s.busy = None;
                s.message = Some(match result {
                    Ok(text) => (true, text.into()),
                    Err(e) => (false, format!("{host}: {e:#}").into()),
                });
                cx.notify();
            })
            .ok();
        })
        .detach();
        cx.notify();
    }

    pub fn render_settings(&self, cx: &mut Context<Self>) -> Option<impl IntoElement> {
        let s = self.settings.as_ref()?;
        let local = matches!(s.scope, Scope::Local(_));
        let label = |text: &'static str| div().text_xs().text_color(c(theme::MUTED)).child(text);
        // "Pull · inherited: rebase" when this scope leaves it to another config.
        let bool_label = |text: &'static str, current: Option<bool>, inherited: Option<bool>, names: (&str, &str)| {
            let suffix = match (current, inherited) {
                (None, Some(v)) => format!(" · inherited: {}", if v { names.1 } else { names.0 }),
                _ => String::new(),
            };
            div().text_xs().text_color(c(theme::MUTED)).child(format!("{text}{suffix}"))
        };
        let field = |title: &'static str, input: &Entity<InputState>| div().flex_1().flex().flex_col().gap_1().child(label(title)).child(Input::new(input));

        // A row of mutually exclusive buttons; per-repository scope adds "Inherit".
        let choice = |id: &'static str, options: Vec<(&'static str, Option<bool>)>, current: Option<bool>, cx: &mut Context<Self>, pick: fn(&mut Settings, Option<bool>)| {
            div().flex().gap_1().children(
                options
                    .into_iter()
                    .filter(|(_, v)| local || v.is_some())
                    .enumerate()
                    .map(move |(i, (text, value))| {
                        Button::new((id, i))
                            .label(text)
                            .xsmall()
                            .when(value == current, |b| b.primary())
                            .when(value != current, |b| b.ghost())
                            .on_click(cx.listener(move |this, _, _, cx| {
                                if let Some(s) = &mut this.settings {
                                    pick(s, value);
                                    cx.notify();
                                }
                            }))
                    }),
            )
        };

        let scope_tabs = div()
            .flex()
            .gap_1()
            .child(
                Button::new("settings-global")
                    .label("All repositories")
                    .small()
                    .when(!local, |b| b.primary())
                    .when(local, |b| b.ghost())
                    .on_click(cx.listener(|this, _, window, cx| this.settings_scope(Scope::Global, window, cx))),
            )
            .when_some(s.repo.clone(), |d, repo| {
                d.child(
                    Button::new("settings-local")
                        .label(format!("{} only", projects::display_name(&repo)))
                        .small()
                        .when(local, |b| b.primary())
                        .when(!local, |b| b.ghost())
                        .on_click(cx.listener(move |this, _, window, cx| this.settings_scope(Scope::Local(repo.clone()), window, cx))),
                )
            });

        let format_is_ssh = s.signing_format == "ssh";
        let signing_format = div()
            .flex()
            .gap_1()
            .children([("SSH", "ssh"), ("GPG", "openpgp")].into_iter().enumerate().map(|(i, (text, value))| {
                let selected = if value == "ssh" { format_is_ssh } else { !format_is_ssh };
                Button::new(("settings-format", i))
                    .label(text)
                    .xsmall()
                    .when(selected, |b| b.primary())
                    .when(!selected, |b| b.ghost())
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if let Some(s) = &mut this.settings {
                            s.signing_format = value.into();
                            cx.notify();
                        }
                    }))
            }));

        let busy = s.busy.is_some();
        let keys = div().flex().flex_col().rounded_md().border_1().border_color(c(theme::BORDER)).overflow_hidden().children(
            s.keys.iter().enumerate().map(|(i, key)| {
                let (copy, sign) = (key.clone(), key.clone());
                div()
                    .flex()
                    .flex_none()
                    .items_center()
                    .gap_3()
                    .px_3()
                    .py_2()
                    .when(i > 0, |d| d.border_t_1().border_color(c(theme::BORDER)))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .child(
                                div()
                                    .flex()
                                    .gap_2()
                                    .items_baseline()
                                    .child(div().text_sm().font_weight(FontWeight::MEDIUM).child(key.private.file_name().map(|f| f.to_string_lossy().into_owned()).unwrap_or_default()))
                                    .child(div().text_xs().text_color(c(theme::MUTED)).child(key.kind.clone()))
                                    .child(div().text_xs().text_color(c(theme::MUTED)).overflow_hidden().text_ellipsis().whitespace_nowrap().child(key.comment.clone())),
                            )
                            .child(div().text_xs().text_color(c(theme::FAINT)).overflow_hidden().text_ellipsis().whitespace_nowrap().child(key.fingerprint.clone())),
                    )
                    .child(Button::new(("key-copy", i)).label("Copy public key").xsmall().ghost().on_click(cx.listener(move |this, _, _, cx| this.copy_public_key(&copy, cx))))
                    .child(
                        Button::new(("key-sign", i))
                            .label("Use for signing")
                            .xsmall()
                            .ghost()
                            .on_click(cx.listener(move |this, _, window, cx| this.use_key_for_signing(&sign, window, cx))),
                    )
            }),
        );

        let card = |title: &'static str, hint: SharedString| {
            div()
                .flex_none()
                .flex()
                .flex_col()
                .gap_3()
                .p_4()
                .rounded_lg()
                .border_1()
                .border_color(c(theme::BORDER))
                .bg(c(theme::PANEL))
                .child(
                    div()
                        .flex()
                        .items_baseline()
                        .gap_2()
                        .child(div().text_sm().font_weight(FontWeight::BOLD).child(title))
                        .child(div().text_xs().text_color(c(theme::FAINT)).child(hint)),
                )
        };

        let body = div()
            .w_full()
            .max_w(px(820.))
            .flex()
            .flex_col()
            .gap_5()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_3()
                    .child(
                        Button::new("settings-back")
                            .label("← Back")
                            .small()
                            .ghost()
                            .on_click(cx.listener(|this, _, _, cx| this.close_settings(cx))),
                    )
                    .child(div().flex_1().text_2xl().font_weight(FontWeight::BOLD).child("Settings"))
                    .child(scope_tabs),
            )
            .child(div().text_sm().text_color(c(theme::MUTED)).child(if local {
                "Saved in this repository's .git/config. Leave a field empty to use the value for all repositories."
            } else {
                "Saved in ~/.gitconfig, so the terminal and other git tools use the same settings."
            }))
            .child(
                card("Identity", "author of your commits".into())
                    .child(div().flex().gap_3().child(field("Name", &s.name)).child(field("Email", &s.email))),
            )
            .child(
                card("Defaults", SharedString::default()).child(
                    div()
                        .flex()
                        .gap_3()
                        .items_end()
                        .when(!local, |d| d.child(field("Default branch for new repositories", &s.default_branch)))
                        .child(div().flex().flex_col().gap_1().child(bool_label("Pull", s.pull_rebase, s.inherited.pull_rebase, ("merge", "rebase"))).child(choice(
                            "settings-pull",
                            vec![("Inherit", None), ("Merge", Some(false)), ("Rebase", Some(true))],
                            s.pull_rebase,
                            cx,
                            |s, v| s.pull_rebase = v,
                        ))),
                ),
            )
            .child(
                card("Commit signing", "shows as verified on GitHub and GitLab".into()).child(
                    div()
                        .flex()
                        .gap_3()
                        .items_end()
                        .child(div().flex().flex_col().gap_1().child(bool_label("Sign commits", s.sign_commits, s.inherited.sign_commits, ("off", "on"))).child(choice(
                            "settings-sign",
                            vec![("Inherit", None), ("Off", Some(false)), ("On", Some(true))],
                            s.sign_commits,
                            cx,
                            |s, v| s.sign_commits = v,
                        )))
                        .child(div().flex().flex_col().gap_1().child(label("Format")).child(signing_format))
                        .child(field(if format_is_ssh { "Key (path to .pub)" } else { "Key ID" }, &s.signing_key)),
                ),
            )
            .child(
                card("SSH keys", projects::short_path(&ssh_dir()).into())
                    .when(s.keys.is_empty(), |d| {
                        d.child(div().text_sm().text_color(c(theme::MUTED)).child("No keys yet. Generate one to push and pull over SSH."))
                    })
                    .when(!s.keys.is_empty(), |d| d.child(keys))
                    .child(
                        div()
                            .flex()
                            .gap_2()
                            .items_center()
                            .child(div().w(px(220.)).child(Input::new(&s.passphrase).mask_toggle()))
                            .child(
                                Button::new("key-generate")
                                    .label(if s.busy == Some("generate") { "Generating…" } else { "Generate Ed25519 key" })
                                    .small()
                                    .disabled(busy)
                                    .on_click(cx.listener(|this, _, _, cx| this.generate_key(cx))),
                            )
                            .child(div().flex_1())
                            .children(["github.com", "gitlab.com"].into_iter().enumerate().map(|(i, host)| {
                                Button::new(("key-test", i))
                                    .label(if s.busy == Some(host) { "Testing…" } else if i == 0 { "Test GitHub" } else { "Test GitLab" })
                                    .small()
                                    .ghost()
                                    .disabled(busy)
                                    .on_click(cx.listener(move |this, _, _, cx| this.test_host(host, cx)))
                            })),
                    )
                    .when(s.created, |d| {
                        d.child(div().flex().child(
                            Button::new("key-github")
                                .label("Add key on GitHub ↗")
                                .small()
                                .ghost()
                                .on_click(cx.listener(|_, _, _, cx| cx.open_url("https://github.com/settings/ssh/new"))),
                        ))
                    }),
            )
            .when_some(s.message.clone(), |d, (ok, text)| {
                d.child(div().text_sm().text_color(c(if ok { theme::ADD } else { theme::DEL })).child(text))
            })
            .child(self.render_app_prefs(cx))
            .child(
                div()
                    .flex()
                    .gap_2()
                    .child(div().flex_1())
                    .child(Button::new("settings-cancel").label("Cancel").small().ghost().on_click(cx.listener(|this, _, _, cx| this.close_settings(cx))))
                    .child(Button::new("settings-save").label("Save").small().primary().on_click(cx.listener(|this, _, _, cx| this.save_settings(cx)))),
            );

        Some(div().id("settings-page").size_full().overflow_y_scroll().flex().justify_center().items_start().px_8().py_10().bg(c(theme::BG)).child(body))
    }
}

impl Shell {
    /// GitChrono's own preferences; unlike the git settings they apply at once.
    fn render_app_prefs(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let switch = |id: &'static str, on: bool, cx: &mut Context<Self>, set: fn(&mut crate::projects::Session, bool)| {
            div().flex().gap_1().children([("Off", false), ("On", true)].into_iter().enumerate().map(move |(i, (label, value))| {
                Button::new((id, i))
                    .label(label)
                    .xsmall()
                    .when(on == value, |b| b.primary())
                    .when(on != value, |b| b.ghost())
                    .on_click(cx.listener(move |this, _, _, cx| this.set_pref(cx, |s| set(s, value))))
            }))
        };
        let row = |title: &'static str, hint: SharedString| {
            div()
                .flex()
                .items_center()
                .gap_3()
                .child(div().flex_1().flex().flex_col().child(div().text_sm().child(title)).child(div().text_xs().text_color(c(theme::FAINT)).child(hint)))
        };
        div()
            .flex_none()
            .flex()
            .flex_col()
            .gap_3()
            .p_4()
            .rounded_lg()
            .border_1()
            .border_color(c(theme::BORDER))
            .bg(c(theme::PANEL))
            .child(
                div()
                    .flex()
                    .items_baseline()
                    .gap_2()
                    .child(div().text_sm().font_weight(FontWeight::BOLD).child("GitChrono"))
                    .child(div().text_xs().text_color(c(theme::FAINT)).child("applies right away")),
            )
            .child(
                row("Check for updates automatically", self.update_status().unwrap_or_else(|| "New versions come from GitHub releases and are checked before installing.".into()))
                    .child(Button::new("prefs-check-now").label("Check now").xsmall().ghost().on_click(cx.listener(|this, _, _, cx| this.check_for_updates(true, cx))))
                    .child(switch("prefs-updates", self.auto_update_enabled(), cx, |s, v| s.auto_update = Some(v))),
            )
            .child(
                row(
                    "Profile pictures",
                    "From Gravatar, which receives a hash of each author's email; people without one get a pattern. GitHub no-reply addresses use GitHub.".into(),
                )
                .child(Button::new("prefs-gravatar-link").label("Gravatar ↗").xsmall().ghost().on_click(|_, _, cx| cx.open_url("https://gravatar.com")))
                .child(switch("prefs-gravatar", self.session.gravatar, cx, |s, v| s.gravatar = v)),
            )
    }
}

/// Fills the inputs from `scope`. Empty fields show, as placeholder, the
/// value git uses anyway (from the global, system or an included config).
fn load_scope(s: &mut Settings, scope: Scope, window: &mut Window, cx: &mut App) {
    let values = setup::read_setup(&scope);
    let dir = match &scope {
        Scope::Local(repo) => repo.clone(),
        _ => std::env::temp_dir(),
    };
    let inherited = setup::read_setup(&Scope::Effective(dir));
    let fill = |input: &Entity<InputState>, value: &str, inherited: &str, fallback: &str, window: &mut Window, cx: &mut App| {
        let placeholder = if inherited.is_empty() { fallback.to_string() } else { format!("{inherited} (inherited)") };
        input.update(cx, |i, cx| {
            i.set_value(value.to_string(), window, cx);
            i.set_placeholder(placeholder, window, cx);
        });
    };
    fill(&s.name, &values.name, &inherited.name, "Your Name", window, cx);
    fill(&s.email, &values.email, &inherited.email, "you@example.com", window, cx);
    fill(&s.default_branch, &values.default_branch, &inherited.default_branch, "master (git's default)", window, cx);
    fill(&s.signing_key, &values.signing_key, &inherited.signing_key, "~/.ssh/id_ed25519.pub", window, cx);
    s.pull_rebase = values.pull_rebase;
    s.sign_commits = values.sign_commits;
    s.signing_format = values.signing_format.clone();
    s.loaded = values;
    s.inherited = inherited;
    s.scope = scope;
}
