//! File history (right panel, diff per commit in the center) and blame
//! (center, one row per line with the commit that last touched it).

use std::sync::Arc;

use gitchrono_core::{BlameLine, FileCommit};
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::Sizable as _;
use gpui_kit::prelude::*;
use gpui_kit::*;

use crate::theme::{self, c};
use crate::views::relative_time;
use crate::workspace::{DiffSource, Workspace};

const HISTORY_LIMIT: usize = 2000;
const BLAME_ROW: f32 = 20.;

pub struct FileHistory {
    pub path: String,
    pub commits: Vec<FileCommit>,
    pub selected: Option<usize>,
}

pub struct BlameView {
    pub path: String,
    pub lines: Arc<Vec<BlameLine>>,
}

impl Workspace {
    pub fn show_file_history(&mut self, path: String, cx: &mut Context<Self>) {
        let Some(repo) = self.repo.clone() else { return };
        let target = path.clone();
        let task = cx.background_spawn(async move { repo.file_history(&target, HISTORY_LIMIT) });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            this.update(cx, |this, cx| {
                match result {
                    Ok(commits) => {
                        this.file_history = Some(FileHistory { path, commits, selected: None });
                        this.select_history(0, cx);
                    }
                    Err(e) => this.error = Some(format!("{e:#}").into()),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    pub fn select_history(&mut self, index: usize, cx: &mut Context<Self>) {
        let Some(history) = &mut self.file_history else { return };
        let Some(commit) = history.commits.get(index) else { return };
        history.selected = Some(index);
        let title = format!("{} @ {}", commit.path, &commit.id[..7]);
        let source = DiffSource::Commit { id: commit.id.clone(), path: commit.path.clone() };
        self.blame = None;
        self.show_diff(title, source, cx);
    }

    pub fn close_file_history(&mut self, cx: &mut Context<Self>) {
        self.file_history = None;
        self.diff = None;
        cx.notify();
    }

    pub fn show_blame(&mut self, path: String, cx: &mut Context<Self>) {
        let Some(repo) = self.repo.clone() else { return };
        let target = path.clone();
        let task = cx.background_spawn(async move { repo.blame(&target, None) });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            this.update(cx, |this, cx| {
                match result {
                    Ok(lines) => {
                        this.diff = None;
                        this.blame = Some(BlameView { path, lines: Arc::new(lines) });
                    }
                    Err(e) => this.error = Some(format!("{e:#}").into()),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    pub fn close_blame(&mut self, cx: &mut Context<Self>) {
        self.blame = None;
        cx.notify();
    }

    pub fn render_file_history(&self, history: &FileHistory, cx: &mut Context<Self>) -> impl IntoElement {
        let path = history.path.clone();
        div()
            .size_full()
            .flex()
            .flex_col()
            .child(
                div()
                    .flex_none()
                    .p_3()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .border_b_1()
                    .border_color(c(theme::BORDER))
                    .child(div().text_xs().text_color(c(theme::FAINT)).child(format!("FILE HISTORY · {} commits", history.commits.len())))
                    .child(div().text_sm().font_weight(FontWeight::BOLD).truncate().child(history.path.clone()))
                    .child(
                        div()
                            .flex()
                            .gap_2()
                            .child(Button::new("history-blame").label("Blame").xsmall().ghost().on_click(cx.listener(
                                move |this, _, _, cx| this.show_blame(path.clone(), cx),
                            )))
                            .child(div().flex_1())
                            .child(
                                Button::new("history-close")
                                    .label("Close")
                                    .xsmall()
                                    .ghost()
                                    .on_click(cx.listener(|this, _, _, cx| this.close_file_history(cx))),
                            ),
                    ),
            )
            .child(div().id("history-list").flex_1().overflow_y_scroll().children(history.commits.iter().enumerate().map(|(i, commit)| {
                let selected = history.selected == Some(i);
                let renamed = commit.path != history.path;
                div()
                    .id(("history", i))
                    .px_3()
                    .py_1p5()
                    .flex()
                    .flex_col()
                    .gap_0p5()
                    .cursor_pointer()
                    .border_b_1()
                    .border_color(c(theme::BORDER))
                    .when(selected, |d| d.bg(c(theme::SELECTED)))
                    .when(!selected, |d| d.hover(|d| d.bg(c(theme::HOVER))))
                    .child(div().text_sm().truncate().child(commit.summary.clone()))
                    .child(div().text_xs().text_color(c(theme::MUTED)).child(format!(
                        "{} · {} · {}",
                        commit.author_name,
                        relative_time(commit.time),
                        &commit.id[..7]
                    )))
                    .when(renamed, |d| d.child(div().text_xs().text_color(c(theme::WARN)).child(format!("as {}", commit.path))))
                    .on_click(cx.listener(move |this, _, _, cx| this.select_history(i, cx)))
            })))
    }

    pub fn render_blame(&self, blame: &BlameView, cx: &mut Context<Self>) -> impl IntoElement {
        let lines = blame.lines.clone();
        let header = div()
            .h(px(36.))
            .flex_none()
            .flex()
            .items_center()
            .gap_2()
            .px_3()
            .bg(c(theme::PANEL))
            .border_b_1()
            .border_color(c(theme::BORDER))
            .child(div().text_xs().text_color(c(theme::FAINT)).child("BLAME"))
            .child(div().flex_1().min_w_0().truncate().text_sm().font_weight(FontWeight::BOLD).child(blame.path.clone()))
            .child(Button::new("blame-close").label("Close").xsmall().ghost().on_click(cx.listener(|this, _, _, cx| this.close_blame(cx))));
        let list = uniform_list(
            "blame",
            lines.len(),
            cx.processor(move |_, range: std::ops::Range<usize>, _, cx| {
                range.map(|i| blame_row(&lines, i, cx)).collect::<Vec<_>>()
            }),
        )
        .flex_1();
        div().flex_1().min_w_0().h_full().flex().flex_col().bg(c(theme::BG)).child(header).child(list)
    }
}

fn blame_row(lines: &[BlameLine], i: usize, cx: &mut Context<Workspace>) -> AnyElement {
    let line = &lines[i];
    let block_start = i == 0 || lines[i - 1].id != line.id;
    // Alternate shading per block so neighbouring commits stay distinguishable.
    let block_index = lines[..=i].windows(2).filter(|w| w[0].id != w[1].id).count();
    let id = line.id.clone();
    let uncommitted = line.id.bytes().all(|b| b == b'0');
    div()
        .id(("blame", i))
        .h(px(BLAME_ROW))
        .w_full()
        .flex()
        .items_center()
        .text_xs()
        .when(block_index % 2 == 1, |d| d.bg(c(theme::PANEL)))
        .when(block_start && i > 0, |d| d.border_t_1().border_color(c(theme::BORDER)))
        .child(
            div()
                .id(("blame-gutter", i))
                .w(px(280.))
                .flex_none()
                .px_2()
                .flex()
                .gap_2()
                .overflow_hidden()
                .whitespace_nowrap()
                .text_color(c(theme::MUTED))
                .cursor_pointer()
                .hover(|d| d.text_color(c(theme::TEXT)))
                .when(block_start, |d| {
                    d.child(div().flex_none().text_color(c(theme::ACCENT)).child(if uncommitted { "uncommitted".to_string() } else { line.id[..7].to_string() }))
                        .child(div().flex_none().w(px(90.)).truncate().child(line.author_name.clone()))
                        .child(div().flex_1().truncate().text_color(c(theme::FAINT)).child(line.summary.clone()))
                })
                .on_click(cx.listener(move |this, _, _, cx| {
                    if !uncommitted {
                        this.blame = None;
                        this.select_commit_id(&id, cx);
                    }
                })),
        )
        .child(div().w(px(44.)).flex_none().pr_2().text_right().text_color(c(theme::FAINT)).child(line.line_no.to_string()))
        .child(div().flex_1().min_w_0().whitespace_nowrap().overflow_hidden().font_family(MONO).text_color(c(theme::TEXT)).child(line.text.replace('\t', "    ")))
        .into_any_element()
}

#[cfg(target_os = "macos")]
const MONO: &str = "Menlo";
#[cfg(target_os = "windows")]
const MONO: &str = "Consolas";
#[cfg(not(any(target_os = "macos", target_os = "windows")))]
const MONO: &str = "DejaVu Sans Mono";
