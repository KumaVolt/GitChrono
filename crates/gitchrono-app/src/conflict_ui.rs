//! Conflict editor: pick ours / theirs / both per conflict, then save and
//! mark the file resolved.

use gitchrono_core::{Chunk, ConflictFile, Resolution};
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::{Disableable as _, Sizable as _};
use gpui_kit::prelude::*;
use gpui_kit::*;

use crate::theme::{self, c};
use crate::watcher::Change;
use crate::workspace::Workspace;

#[cfg(target_os = "macos")]
const MONO: &str = "Menlo";
#[cfg(target_os = "windows")]
const MONO: &str = "Consolas";
#[cfg(not(any(target_os = "macos", target_os = "windows")))]
const MONO: &str = "DejaVu Sans Mono";

const OURS_BG: u32 = 0x1c2d44;
const THEIRS_BG: u32 = 0x2f2142;

pub struct ConflictView {
    pub file: ConflictFile,
    pub choices: Vec<Option<Resolution>>,
}

impl Workspace {
    pub fn open_conflict(&mut self, path: String, cx: &mut Context<Self>) {
        let Some(repo) = self.repo.clone() else { return };
        let task = cx.background_spawn(async move { repo.conflict_file(&path) });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            this.update(cx, |this, cx| {
                match result {
                    Ok(file) => {
                        let choices = vec![None; file.conflict_count()];
                        this.diff = None;
                        this.blame = None;
                        this.conflict = Some(ConflictView { file, choices });
                    }
                    Err(e) => this.error = Some(format!("{e:#}").into()),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn choose(&mut self, index: usize, resolution: Resolution, cx: &mut Context<Self>) {
        if let Some(view) = &mut self.conflict {
            let slot = &mut view.choices[index];
            *slot = if *slot == Some(resolution) { None } else { Some(resolution) };
            cx.notify();
        }
    }

    fn save_conflict(&mut self, cx: &mut Context<Self>) {
        let Some(view) = self.conflict.take() else { return };
        let ConflictView { file, choices } = view;
        self.run_inner("Saving resolution".into(), Change::Status, false, cx, move |r| r.save_resolution(&file, &choices).map(drop), |_, _| {});
        cx.notify();
    }

    fn take_whole_file(&mut self, ours: bool, cx: &mut Context<Self>) {
        let Some(view) = self.conflict.take() else { return };
        let path = view.file.path;
        let label = if ours { "Taking ours" } else { "Taking theirs" };
        self.run_inner(label.into(), Change::Status, false, cx, move |r| r.resolve_whole_file(&path, ours), |_, _| {});
        cx.notify();
    }

    pub fn render_conflict(&self, view: &ConflictView, cx: &mut Context<Self>) -> impl IntoElement {
        let total = view.choices.len();
        let done = view.choices.iter().filter(|c| c.is_some()).count();
        let header = div()
            .h(px(40.))
            .flex_none()
            .flex()
            .items_center()
            .gap_2()
            .px_3()
            .bg(c(theme::PANEL))
            .border_b_1()
            .border_color(c(theme::BORDER))
            .child(div().text_xs().text_color(c(theme::WARN)).child("CONFLICT"))
            .child(div().flex_1().min_w_0().truncate().text_sm().font_weight(FontWeight::BOLD).child(view.file.path.clone()))
            .child(div().text_xs().text_color(c(theme::MUTED)).child(format!("{done} of {total} resolved")))
            .child(Button::new("conflict-ours").label("Take ours").xsmall().ghost().on_click(cx.listener(|this, _, _, cx| this.take_whole_file(true, cx))))
            .child(Button::new("conflict-theirs").label("Take theirs").xsmall().ghost().on_click(cx.listener(|this, _, _, cx| this.take_whole_file(false, cx))))
            .child(
                Button::new("conflict-save")
                    .label(if done == total { "Save & mark resolved" } else { "Save" })
                    .xsmall()
                    .primary()
                    .disabled(self.busy.is_some())
                    .on_click(cx.listener(|this, _, _, cx| this.save_conflict(cx))),
            )
            .child(Button::new("conflict-close").label("Close").xsmall().ghost().on_click(cx.listener(|this, _, _, cx| {
                this.conflict = None;
                cx.notify();
            })));

        let mut body: Vec<AnyElement> = Vec::new();
        let mut index = 0;
        for chunk in &view.file.chunks {
            match chunk {
                Chunk::Common(lines) => {
                    for line in lines {
                        body.push(code_line(line, None).into_any_element());
                    }
                }
                Chunk::Conflict(conflict) => {
                    let i = index;
                    index += 1;
                    let choice = view.choices[i];
                    let option = |id: &'static str, label: &'static str, resolution: Resolution, cx: &mut Context<Self>| {
                        Button::new((id, i))
                            .label(label)
                            .xsmall()
                            .when(choice == Some(resolution), |b| b.primary())
                            .when(choice != Some(resolution), |b| b.outline())
                            .on_click(cx.listener(move |this, _, _, cx| this.choose(i, resolution, cx)))
                    };
                    let side = |label: String, lines: &[String], bg: u32| {
                        div()
                            .flex_1()
                            .min_w_0()
                            .overflow_hidden()
                            .bg(c(bg))
                            .when(choice.is_some(), |d| d.opacity(0.45))
                            .child(div().px_2().py_0p5().text_xs().text_color(c(theme::MUTED)).child(label))
                            .children(lines.iter().map(|l| code_line(l, None)))
                    };
                    let result: Vec<String> = match choice {
                        Some(Resolution::Ours) => conflict.ours.clone(),
                        Some(Resolution::Theirs) => conflict.theirs.clone(),
                        Some(Resolution::OursThenTheirs) => conflict.ours.iter().chain(&conflict.theirs).cloned().collect(),
                        Some(Resolution::TheirsThenOurs) => conflict.theirs.iter().chain(&conflict.ours).cloned().collect(),
                        None => Vec::new(),
                    };
                    body.push(
                        div()
                            .my_1()
                            .border_1()
                            .border_color(c(if choice.is_some() { theme::ADD } else { theme::WARN }))
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap_1()
                                    .px_2()
                                    .py_1()
                                    .bg(c(theme::PANEL_ALT))
                                    .child(div().flex_1().text_xs().text_color(c(theme::MUTED)).child(format!("Conflict {} of {total}", i + 1)))
                                    .child(option("pick-ours", "Ours", Resolution::Ours, cx))
                                    .child(option("pick-theirs", "Theirs", Resolution::Theirs, cx))
                                    .child(option("pick-both", "Ours + theirs", Resolution::OursThenTheirs, cx))
                                    .child(option("pick-both-rev", "Theirs + ours", Resolution::TheirsThenOurs, cx)),
                            )
                            .child(
                                div()
                                    .flex()
                                    .child(side(format!("ours · {}", conflict.ours_label), &conflict.ours, OURS_BG))
                                    .child(div().w_px().bg(c(theme::BORDER)))
                                    .child(side(format!("theirs · {}", conflict.theirs_label), &conflict.theirs, THEIRS_BG)),
                            )
                            .when(choice.is_some(), |d| {
                                d.child(
                                    div()
                                        .bg(c(theme::ADD_BG))
                                        .child(div().px_2().py_0p5().text_xs().text_color(c(theme::ADD)).child("result"))
                                        .children(result.iter().map(|l| code_line(l, None))),
                                )
                            })
                            .into_any_element(),
                    );
                }
            }
        }

        div()
            .flex_1()
            .min_w_0()
            .h_full()
            .flex()
            .flex_col()
            .bg(c(theme::BG))
            .child(header)
            .child(div().id("conflict-body").flex_1().overflow_y_scroll().px_2().py_1().text_xs().children(body))
    }
}

fn code_line(text: &str, color: Option<u32>) -> Div {
    div()
        .px_2()
        .whitespace_nowrap()
        .overflow_hidden()
        .font_family(MONO)
        .text_color(c(color.unwrap_or(theme::TEXT)))
        .child(if text.is_empty() { " ".to_string() } else { text.replace('\t', "    ") })
}
