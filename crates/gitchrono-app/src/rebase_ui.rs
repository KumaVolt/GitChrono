//! Interactive rebase editor: reorder commits and pick an action for each.

use gitchrono_core::{RebaseAction, RebaseStep, Repo};
use gpui_kit::component::button::ButtonVariants as _;
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::Sizable as _;
use gpui_kit::prelude::*;
use gpui_kit::*;

use crate::theme::{self, c};
use crate::watcher::Change;
use crate::workspace::Workspace;
use crate::ui::{ButtonExt as _, button};

pub struct RebaseEditor {
    base: String,
    steps: Vec<RebaseStep>,
    /// Message editors, one per step (shown for reword).
    messages: Vec<Entity<InputState>>,
}

fn action_color(action: RebaseAction) -> u32 {
    match action {
        RebaseAction::Pick => theme::TEXT,
        RebaseAction::Reword => theme::ACCENT,
        RebaseAction::Squash | RebaseAction::Fixup => theme::WARN,
        RebaseAction::Drop => theme::DEL,
    }
}

impl Workspace {
    /// Edit the commits on HEAD that `base`, a commit or branch, doesn't have.
    pub fn open_rebase_editor(&mut self, base: String, window: &mut Window, cx: &mut Context<Self>) {
        let Some(repo) = self.repo.clone() else { return };
        let target = base.clone();
        let task = cx.background_spawn(async move { repo.rebase_plan(&target) });
        cx.spawn_in(window, async move |this, cx| {
            let result = task.await;
            this.update_in(cx, |this, window, cx| {
                match result {
                    Ok(steps) if steps.is_empty() => {
                        this.error = Some("The current branch has no commits to rebase here.".into());
                    }
                    Ok(steps) => {
                        let messages = steps
                            .iter()
                            .map(|s| {
                                let text = s.message.clone();
                                cx.new(|cx| InputState::new(window, cx).default_value(text))
                            })
                            .collect();
                        this.rebase_editor = Some(RebaseEditor { base, steps, messages });
                    }
                    Err(e) => this.error = Some(format!("{e:#}").into()),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// Drop `id` from the current branch, after a yes.
    pub fn drop_commit(&mut self, id: String, window: &mut Window, cx: &mut Context<Self>) {
        let question = format!("Drop {} from the current branch? The commits after it get new ids.", &id[..7]);
        let target = id.clone();
        self.rewrite(id, Some(("Drop commit", question)), "Dropping", move |r| r.drop_commit(&target), window, cx);
    }

    /// Swap `id` with the commit after it (`up`) or before it.
    pub fn move_commit(&mut self, id: String, up: bool, window: &mut Window, cx: &mut Context<Self>) {
        // The older of the two is the first commit rewritten.
        let parent = self.snap.commits.iter().find(|c| c.id == id).and_then(|c| c.parents.first().cloned());
        let Some(oldest) = (if up { Some(id.clone()) } else { parent }) else { return };
        self.rewrite(oldest, None, "Moving commit", move |r| r.move_commit(&id, up), window, cx);
    }

    /// Run `op`, which rewrites `oldest` and everything after it. A remote
    /// having `oldest` means a force push, so that always asks first; `ask`
    /// is a question to ask regardless.
    fn rewrite(
        &mut self,
        oldest: String,
        ask: Option<(&'static str, String)>,
        busy: &'static str,
        op: impl FnOnce(&Repo) -> anyhow::Result<()> + Send + 'static,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(repo) = self.repo.clone() else { return };
        let task = cx.background_spawn(async move { repo.is_pushed(&oldest).unwrap_or(false) });
        cx.spawn_in(window, async move |this, cx| {
            let pushed = task.await;
            this.update_in(cx, |this, window, cx| {
                let warning = "A remote already has these commits, so the branch will need a force push.";
                match (ask, pushed) {
                    (None, false) => this.run(busy, Change::Refs, cx, op),
                    (Some((title, question)), true) => this.confirm(title, format!("{question} {warning}"), "Continue", busy, window, cx, op),
                    (Some((title, question)), false) => this.confirm(title, question, "Continue", busy, window, cx, op),
                    (None, true) => this.confirm("Rewrite pushed commits", warning.to_string(), "Continue", busy, window, cx, op),
                }
            })
            .ok();
        })
        .detach();
    }

    fn cycle_rebase_action(&mut self, index: usize, cx: &mut Context<Self>) {
        let Some(editor) = &mut self.rebase_editor else { return };
        let step = &mut editor.steps[index];
        let next = RebaseAction::ALL.iter().position(|a| *a == step.action).map_or(0, |i| (i + 1) % RebaseAction::ALL.len());
        step.action = RebaseAction::ALL[next];
        cx.notify();
    }

    fn move_rebase_step(&mut self, index: usize, up: bool, cx: &mut Context<Self>) {
        let Some(editor) = &mut self.rebase_editor else { return };
        let other = if up { index.checked_sub(1) } else { Some(index + 1).filter(|&i| i < editor.steps.len()) };
        if let Some(other) = other {
            editor.steps.swap(index, other);
            editor.messages.swap(index, other);
            cx.notify();
        }
    }

    fn start_rebase(&mut self, cx: &mut Context<Self>) {
        let Some(editor) = self.rebase_editor.take() else { return };
        let mut steps = editor.steps;
        for (step, input) in steps.iter_mut().zip(&editor.messages) {
            step.message = input.read(cx).value().trim().to_string();
        }
        let base = editor.base;
        self.run("Rebasing", Change::Refs, cx, move |r| r.interactive_rebase(&base, &steps));
        cx.notify();
    }

    pub fn render_rebase_editor(&self, cx: &mut Context<Self>) -> Option<impl IntoElement> {
        let editor = self.rebase_editor.as_ref()?;
        let n = editor.steps.len();
        let first_melds = editor.steps.first().is_some_and(|s| matches!(s.action, RebaseAction::Squash | RebaseAction::Fixup));
        Some(
            div()
                .id("rebase-backdrop")
                .absolute()
                .inset_0()
                .flex()
                .items_center()
                .justify_center()
                .bg(gpui_kit::black().opacity(0.5))
                .child(
                    div()
                        .id("rebase-editor")
                        .w(px(720.))
                        .max_h(px(640.))
                        .flex()
                        .flex_col()
                        .rounded_lg()
                        .bg(c(theme::PANEL))
                        .border_1()
                        .border_color(c(theme::BORDER))
                        .shadow_lg()
                        .on_click(|_, _, cx| cx.stop_propagation())
                        .child(
                            div()
                                .p_4()
                                .flex()
                                .flex_col()
                                .gap_1()
                                .border_b_1()
                                .border_color(c(theme::BORDER))
                                .child(div().text_base().font_weight(FontWeight::BOLD).child(format!("Interactive rebase · {n} commits")))
                                .child(div().text_sm().text_color(c(theme::MUTED)).child(
                                    "Oldest first. Click an action to change it, ↑/↓ to reorder. Squash and fixup meld into the commit above.",
                                )),
                        )
                        .child(div().id("rebase-steps").flex_1().overflow_y_scroll().children(editor.steps.iter().enumerate().map(|(i, step)| {
                            let dropped = step.action == RebaseAction::Drop;
                            div()
                                .id(("rebase-step", i))
                                .px_4()
                                .py_1p5()
                                .flex()
                                .items_center()
                                .gap_2()
                                .border_b_1()
                                .border_color(c(theme::BORDER))
                                .child(
                                    button(("rebase-action", i))
                                        .label(step.action.label())
                                        .xsmall()
                                        .outline()
                                        .text_color(c(action_color(step.action)))
                                        .w(px(70.))
                                        .on_click(cx.listener(move |this, _, _, cx| this.cycle_rebase_action(i, cx))),
                                )
                                .child(div().w(px(58.)).flex_none().text_xs().text_color(c(theme::FAINT)).font_family("Menlo").child(step.id[..7].to_string()))
                                .child(if step.action == RebaseAction::Reword {
                                    div().flex_1().min_w_0().child(Input::new(&editor.messages[i]).small()).into_any_element()
                                } else {
                                    div()
                                        .flex_1()
                                        .min_w_0()
                                        .truncate()
                                        .text_sm()
                                        .when(dropped, |d| d.line_through().text_color(c(theme::FAINT)))
                                        .child(step.summary.clone())
                                        .into_any_element()
                                })
                                .child(
                                    button(("rebase-up", i))
                                        .label("↑")
                                        .xsmall()
                                        .ghost()
                                        .disabled_if(i == 0)
                                        .on_click(cx.listener(move |this, _, _, cx| this.move_rebase_step(i, true, cx))),
                                )
                                .child(
                                    button(("rebase-down", i))
                                        .label("↓")
                                        .xsmall()
                                        .ghost()
                                        .disabled_if(i + 1 == n)
                                        .on_click(cx.listener(move |this, _, _, cx| this.move_rebase_step(i, false, cx))),
                                )
                        })))
                        .child(
                            div()
                                .p_3()
                                .flex()
                                .items_center()
                                .gap_2()
                                .when(first_melds, |d| {
                                    d.child(div().text_xs().text_color(c(theme::DEL)).child("The first commit can't be squashed or fixed up."))
                                })
                                .child(div().flex_1())
                                .child(button("rebase-cancel").label("Cancel").small().ghost().on_click(cx.listener(|this, _, _, cx| {
                                    this.rebase_editor = None;
                                    cx.notify();
                                })))
                                .child(
                                    button("rebase-start")
                                        .label("Start rebase")
                                        .small()
                                        .primary()
                                        .disabled_if(first_melds || self.busy.is_some())
                                        .on_click(cx.listener(|this, _, _, cx| this.start_rebase(cx))),
                                ),
                        ),
                ),
        )
    }
}
