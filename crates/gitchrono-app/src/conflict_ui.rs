//! Conflict editor, filling the window: the two sides next to each other with
//! a checkbox per conflict, and the merged output below, as in GitKraken.

use std::ops::Range;

use gitchrono_core::{ConflictFile, Line, OutputRow, Resolution, SideRow};
use gpui_kit::component::button::ButtonVariants as _;
use gpui_kit::component::Sizable as _;
use gpui_kit::prelude::*;
use gpui_kit::*;

use crate::highlight::{self, Spans};
use crate::theme::{self, c};
use crate::watcher::Change;
use crate::workspace::Workspace;
use crate::ui::{ButtonExt as _, button};

#[cfg(target_os = "macos")]
const MONO: &str = "Menlo";
#[cfg(target_os = "windows")]
const MONO: &str = "Consolas";
#[cfg(not(any(target_os = "macos", target_os = "windows")))]
const MONO: &str = "DejaVu Sans Mono";

const OURS: u32 = 0x12566b;
const THEIRS: u32 = 0x6b5512;
const ROW: f32 = 19.;
/// Width of a character in the code font at the size used here.
const CHAR_WIDTH: f32 = 7.3;

fn side_color(ours: bool) -> Hsla {
    c(if ours { OURS } else { THEIRS }).into()
}

pub struct ConflictView {
    pub file: ConflictFile,
    pub choices: Vec<Option<Resolution>>,
    rows: Vec<SideRow>,
    output: Vec<OutputRow>,
    /// Colored spans per line of each side; empty without a grammar.
    ours_spans: Vec<Spans>,
    theirs_spans: Vec<Spans>,
    /// The conflict the arrows and the "of N" are at.
    current: usize,
    sides_scroll: UniformListScrollHandle,
    output_scroll: UniformListScrollHandle,
    /// How far the panes are scrolled sideways: code doesn't wrap.
    sides_x: Pixels,
    output_x: Pixels,
    widest: Pixels,
}

impl ConflictView {
    fn new(file: ConflictFile, ours_spans: Vec<Spans>, theirs_spans: Vec<Spans>) -> Self {
        let choices = vec![None; file.conflict_count()];
        let rows = file.side_rows();
        let output = file.output_rows(&choices);
        let widest = [true, false].iter().flat_map(|ours| file.side_text(*ours)).map(|l| highlight::display_text(l).chars().count()).max().unwrap_or(0);
        let view = Self {
            file,
            choices,
            rows,
            output,
            ours_spans,
            theirs_spans,
            current: 0,
            sides_scroll: UniformListScrollHandle::new(),
            output_scroll: UniformListScrollHandle::new(),
            sides_x: px(0.),
            output_x: px(0.),
            widest: px(widest as f32 * CHAR_WIDTH),
        };
        view.show_current();
        view
    }

    /// Bring the current conflict into view in both panes.
    fn show_current(&self) {
        if let Some(row) = self.rows.iter().position(|r| r.conflict == Some(self.current)) {
            self.sides_scroll.scroll_to_item(row, ScrollStrategy::Center);
        }
        if let Some(row) = self.output.iter().position(|r| r.conflict == Some(self.current)) {
            self.output_scroll.scroll_to_item(row, ScrollStrategy::Center);
        }
    }

    fn choices_changed(&mut self) {
        self.output = self.file.output_rows(&self.choices);
        self.show_current();
    }

    fn resolved(&self) -> usize {
        self.choices.iter().filter(|c| c.is_some()).count()
    }
}

impl Workspace {
    pub fn open_conflict(&mut self, path: String, cx: &mut Context<Self>) {
        let Some(repo) = self.repo.clone() else { return };
        let task = cx.background_spawn(async move {
            let file = repo.conflict_file(&path)?;
            let ours = highlight::highlight_lines(&path, &file.side_text(true));
            let theirs = highlight::highlight_lines(&path, &file.side_text(false));
            anyhow::Ok((file, ours, theirs))
        });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            this.update(cx, |this, cx| {
                match result {
                    Ok((file, ours, theirs)) => {
                        this.diff = None;
                        this.blame = None;
                        this.conflict = Some(ConflictView::new(file, ours, theirs));
                    }
                    Err(e) => this.error = Some(format!("{e:#}").into()),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// Open the next conflicted file once `done` is settled, if there is one.
    fn open_next_conflict(&mut self, done: &str, cx: &mut Context<Self>) {
        // `status` still lists `done`: the refresh comes after this.
        if let Some(next) = self.status.conflicted.iter().find(|f| f.path != done) {
            let path = next.path.clone();
            self.open_conflict(path, cx);
        }
    }

    fn toggle_side(&mut self, index: usize, ours: bool, cx: &mut Context<Self>) {
        let Some(view) = &mut self.conflict else { return };
        view.choices[index] = Resolution::toggle(view.choices[index], ours);
        view.current = index;
        view.choices_changed();
        cx.notify();
    }

    /// The pane headers' checkbox: that side for every conflict, or none.
    fn toggle_side_everywhere(&mut self, ours: bool, cx: &mut Context<Self>) {
        let Some(view) = &mut self.conflict else { return };
        let side = if ours { Resolution::Ours } else { Resolution::Theirs };
        let all = view.choices.iter().all(|c| *c == Some(side));
        view.choices.iter_mut().for_each(|c| *c = (!all).then_some(side));
        view.choices_changed();
        cx.notify();
    }

    fn jump_conflict(&mut self, step: isize, cx: &mut Context<Self>) {
        let Some(view) = &mut self.conflict else { return };
        let total = view.choices.len() as isize;
        if total > 0 {
            view.current = (view.current as isize + step).rem_euclid(total) as usize;
            view.show_current();
            cx.notify();
        }
    }

    fn scroll_sideways(&mut self, delta: Point<Pixels>, output: bool, cx: &mut Context<Self>) {
        let Some(view) = &mut self.conflict else { return };
        if delta.x.abs() <= delta.y.abs() {
            return;
        }
        let x = if output { &mut view.output_x } else { &mut view.sides_x };
        *x = (*x - delta.x).clamp(px(0.), view.widest);
        cx.notify();
    }

    fn save_conflict(&mut self, cx: &mut Context<Self>) {
        let Some(view) = self.conflict.take() else { return };
        let all_chosen = view.resolved() == view.choices.len();
        let ConflictView { file, choices, .. } = view;
        let path = file.path.clone();
        self.run_inner("Saving resolution".into(), Change::Status, false, cx, move |r| r.save_resolution(&file, &choices).map(drop), move |this, ok, cx| {
            if ok && all_chosen {
                this.open_next_conflict(&path, cx);
            }
        });
        cx.notify();
    }

    fn take_whole_file(&mut self, ours: bool, cx: &mut Context<Self>) {
        let Some(view) = self.conflict.take() else { return };
        let path = view.file.path;
        let next = path.clone();
        let label = if ours { "Taking ours" } else { "Taking theirs" };
        self.run_inner(label.into(), Change::Status, false, cx, move |r| r.resolve_whole_file(&path, ours), move |this, ok, cx| {
            if ok {
                this.open_next_conflict(&next, cx);
            }
        });
        cx.notify();
    }

    fn render_header(&self, view: &ConflictView, cx: &mut Context<Self>) -> impl IntoElement {
        let (total, done) = (view.choices.len(), view.resolved());
        let others = self.status.conflicted.iter().filter(|f| f.path != view.file.path).count();
        div()
            .h(px(36.))
            .flex_none()
            .flex()
            .items_center()
            .gap_2()
            .px_3()
            .bg(c(theme::PANEL))
            .border_b_1()
            .border_color(c(theme::BORDER))
            .child(div().flex_none().text_color(c(theme::WARN)).child("⚠"))
            .child(div().min_w_0().truncate().text_sm().font_weight(FontWeight::BOLD).child(view.file.path.clone()))
            .child(div().flex_none().text_xs().text_color(c(theme::MUTED)).child(format!("{done} of {total} resolved")))
            .when(others > 0, |d| d.child(div().flex_none().text_xs().text_color(c(theme::FAINT)).child(format!("{others} more file{} to resolve", if others == 1 { "" } else { "s" }))))
            .child(div().flex_1())
            .child(button("conflict-ours").label("Take ours").xsmall().ghost().tooltip("Use our version of the whole file").on_click(cx.listener(|this, _, _, cx| this.take_whole_file(true, cx))))
            .child(button("conflict-theirs").label("Take theirs").xsmall().ghost().tooltip("Use their version of the whole file").on_click(cx.listener(|this, _, _, cx| this.take_whole_file(false, cx))))
            .child(
                button("conflict-save")
                    .label(if done == total { "Save & mark resolved" } else { "Save" })
                    .xsmall()
                    .primary()
                    .disabled_if(self.busy.is_some())
                    .on_click(cx.listener(|this, _, _, cx| this.save_conflict(cx))),
            )
            .child(button("conflict-close").label("×").xsmall().ghost().tooltip("Close without saving").on_click(cx.listener(|this, _, _, cx| {
                this.conflict = None;
                cx.notify();
            })))
    }

    fn render_pane_header(&self, view: &ConflictView, ours: bool, cx: &mut Context<Self>) -> impl IntoElement {
        let side = if ours { Resolution::Ours } else { Resolution::Theirs };
        let ticked = !view.choices.is_empty() && view.choices.iter().all(|c| *c == Some(side));
        let (letter, name) = if ours { ("A", "Ours") } else { ("B", "Theirs") };
        let label = view.file.conflict_labels().map(|(o, t)| if ours { o } else { t }).filter(|l| !l.is_empty()).map(str::to_string);
        div()
            .id(if ours { "pane-ours" } else { "pane-theirs" })
            .flex_1()
            .min_w_0()
            .h(px(28.))
            .flex()
            .items_center()
            .gap_2()
            .px_2()
            .cursor_pointer()
            .hover(|d| d.bg(c(theme::HOVER)))
            .on_click(cx.listener(move |this, _, _, cx| this.toggle_side_everywhere(ours, cx)))
            .child(checkbox(ticked))
            .child(div().flex_none().w(px(18.)).h(px(18.)).flex().items_center().justify_center().rounded_sm().bg(side_color(ours)).text_xs().font_weight(FontWeight::BOLD).child(letter))
            .child(div().flex_none().text_sm().child(name))
            .children(label.map(|l| div().min_w_0().truncate().text_xs().text_color(c(theme::MUTED)).child(l)))
    }

    pub fn render_conflict(&self, cx: &mut Context<Self>) -> Option<impl IntoElement> {
        let view = self.conflict.as_ref()?;
        let total = view.choices.len();
        let mut sides = uniform_list(
            "conflict-sides",
            view.rows.len(),
            cx.processor(|this, range: Range<usize>, _, cx| {
                let Some(view) = &this.conflict else { return Vec::new() };
                range.map(|i| side_row(view, i, cx).into_any_element()).collect::<Vec<_>>()
            }),
        )
        .track_scroll(&view.sides_scroll)
        .flex_1();
        // Sideways swipes scroll the code, not the rows.
        sides.style().restrict_scroll_to_axis = Some(true);
        let mut output = uniform_list(
            "conflict-output",
            view.output.len(),
            cx.processor(|this, range: Range<usize>, _, cx| {
                let Some(view) = &this.conflict else { return Vec::new() };
                range.map(|i| output_row(view, i, cx).into_any_element()).collect::<Vec<_>>()
            }),
        )
        .track_scroll(&view.output_scroll)
        .flex_1();
        // Sideways swipes scroll the code, not the rows.
        output.style().restrict_scroll_to_axis = Some(true);
        let arrow = |id: &'static str, label: &'static str, step: isize, cx: &mut Context<Self>| {
            button(id).label(label).xsmall().ghost().disabled_if(total < 2).on_click(cx.listener(move |this, _, _, cx| this.jump_conflict(step, cx)))
        };
        let position = if total == 0 { "no conflict markers in this file".to_string() } else { format!("conflict {} of {total}", view.current + 1) };
        Some(
            div()
                .flex_1()
                .min_h_0()
                .flex()
                .flex_col()
                .bg(c(theme::BG))
                .child(self.render_header(view, cx))
                .child(
                    div()
                        .flex_1()
                        .min_h_0()
                        .flex()
                        .flex_col()
                        .child(
                            div()
                                .flex_none()
                                .flex()
                                .border_b_1()
                                .border_color(c(theme::BORDER))
                                .child(self.render_pane_header(view, true, cx))
                                .child(div().w_px().bg(c(theme::BORDER)))
                                .child(self.render_pane_header(view, false, cx)),
                        )
                        .child(
                            div()
                                .id("conflict-sides-pane")
                                .flex_1()
                                .min_h_0()
                                .flex()
                                .flex_col()
                                .on_scroll_wheel(cx.listener(|this, ev: &ScrollWheelEvent, window, cx| this.scroll_sideways(ev.delta.pixel_delta(window.line_height()), false, cx)))
                                .child(sides),
                        ),
                )
                .child(
                    div()
                        .h(px(30.))
                        .flex_none()
                        .flex()
                        .items_center()
                        .gap_1()
                        .px_3()
                        .bg(c(theme::PANEL))
                        .border_t_1()
                        .border_b_1()
                        .border_color(c(theme::BORDER))
                        .child(div().text_sm().font_weight(FontWeight::BOLD).child("Output"))
                        .child(div().flex_1())
                        .child(div().text_xs().text_color(c(theme::ACCENT)).child(position))
                        .child(arrow("conflict-prev", "▲", -1, cx))
                        .child(arrow("conflict-next", "▼", 1, cx)),
                )
                .child(
                    div()
                        .id("conflict-output-pane")
                        .flex_1()
                        .min_h_0()
                        .flex()
                        .flex_col()
                        .on_scroll_wheel(cx.listener(|this, ev: &ScrollWheelEvent, window, cx| this.scroll_sideways(ev.delta.pixel_delta(window.line_height()), true, cx)))
                        .child(output),
                ),
        )
    }
}

fn checkbox(ticked: bool) -> Div {
    div()
        .flex_none()
        .size(px(14.))
        .flex()
        .items_center()
        .justify_center()
        .rounded_sm()
        .border_1()
        .border_color(c(if ticked { theme::ACCENT } else { theme::MUTED }))
        .when(ticked, |d| d.bg(c(theme::ACCENT)))
        .text_xs()
        .text_color(gpui_kit::white())
        .when(ticked, |d| d.child("✓"))
}

fn number(n: Option<usize>) -> Div {
    div().flex_none().w(px(44.)).pr_2().text_right().font_family(MONO).text_xs().text_color(c(theme::FAINT)).children(n.map(|n| n.to_string()))
}

/// One line of code, shifted by the pane's sideways scroll.
fn code(text: &str, spans: Option<&Spans>, offset: Pixels) -> Div {
    let text = highlight::display_text(text);
    let styled = match spans {
        Some(spans) => StyledText::new(text).with_highlights(spans.iter().map(|(range, color)| (range.clone(), HighlightStyle { color: Some(*color), ..Default::default() }))),
        None => StyledText::new(text),
    };
    div()
        .flex_1()
        .min_w_0()
        .h_full()
        .overflow_hidden()
        .flex()
        .items_center()
        .child(div().flex_none().ml(-offset).whitespace_nowrap().font_family(MONO).text_xs().child(styled))
}

fn side_cell(view: &ConflictView, index: usize, ours: bool, cx: &mut Context<Workspace>) -> Stateful<Div> {
    let row = &view.rows[index];
    let line: Option<&Line> = if ours { row.ours.as_ref() } else { row.theirs.as_ref() };
    let spans = if ours { &view.ours_spans } else { &view.theirs_spans };
    let choice = row.conflict.map(|n| view.choices[n]);
    let included = matches!(choice, Some(Some(r)) if r.includes(ours));
    let tint = match choice {
        None => None,
        Some(None) => Some(0.6),
        Some(Some(_)) if included => Some(1.),
        Some(Some(_)) => Some(0.15),
    };
    div()
        .id(("conflict-cell", index * 2 + usize::from(ours)))
        .flex_1()
        .min_w_0()
        .h_full()
        .flex()
        .items_center()
        .overflow_hidden()
        .when_some(tint, |d, alpha| d.bg(side_color(ours).opacity(alpha)))
        .when_some(row.conflict, |d, conflict| d.cursor_pointer().on_click(cx.listener(move |this, _, _, cx| this.toggle_side(conflict, ours, cx))))
        .child(div().flex_none().w(px(22.)).flex().justify_center().when(row.starts_conflict, |d| d.child(checkbox(included))))
        .child(number(line.map(|l| l.number)))
        .children(line.map(|l| code(&l.text, spans.get(l.number - 1), view.sides_x).when(matches!(choice, Some(Some(_))) && !included, |d| d.opacity(0.5))))
}

fn side_row(view: &ConflictView, index: usize, cx: &mut Context<Workspace>) -> impl IntoElement {
    div()
        .h(px(ROW))
        .w_full()
        .flex()
        .child(side_cell(view, index, true, cx))
        .child(div().w_px().h_full().bg(c(theme::BORDER)))
        .child(side_cell(view, index, false, cx))
}

fn output_row(view: &ConflictView, index: usize, cx: &mut Context<Workspace>) -> impl IntoElement {
    let row = &view.output[index];
    let spans = if row.ours { &view.ours_spans } else { &view.theirs_spans };
    let base = div().id(("conflict-output-row", index)).h(px(ROW)).w_full().flex().items_center().child(div().flex_none().w(px(22.)));
    match &row.line {
        Some(line) => base
            .when(row.conflict.is_some(), |d| d.bg(side_color(row.ours).opacity(0.35)))
            .child(number(Some(line.number)))
            .child(code(&line.text, spans.get(row.source.wrapping_sub(1)), view.output_x)),
        None => {
            let conflict = row.conflict.unwrap_or(0);
            base.bg(c(theme::WARN).opacity(0.12))
                .cursor_pointer()
                .on_click(cx.listener(move |this, _, _, cx| {
                    if let Some(view) = &mut this.conflict {
                        view.current = conflict;
                        view.show_current();
                        cx.notify();
                    }
                }))
                .child(number(None))
                .child(div().flex_1().min_w_0().truncate().text_xs().italic().text_color(c(theme::WARN)).child(format!("Conflict {}: tick a side above", conflict + 1)))
        }
    }
}
