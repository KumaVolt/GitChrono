//! Unified diff view with hunk- and line-level stage / unstage / discard.

use gitchrono_core::DiffLineKind;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::Sizable as _;
use gpui_kit::prelude::*;
use gpui_kit::*;

use crate::theme::{self, c};
use crate::workspace::{DiffView, Workspace};

impl Workspace {
    pub fn render_diff(&self, view: &DiffView, cx: &mut Context<Self>) -> impl IntoElement {
        // `Some(staged)` when this diff supports partial staging.
        let partial = view.source.partial();
        let selected = view.selected.len();
        let mut rows: Vec<AnyElement> = Vec::new();
        if view.diff.binary {
            rows.push(div().p_4().text_color(c(theme::MUTED)).child("Binary file").into_any_element());
        }

        for (h, hunk) in view.diff.hunks.iter().enumerate() {
            rows.push(
                div()
                    .h(px(28.))
                    .px_3()
                    .flex()
                    .items_center()
                    .gap_2()
                    .bg(c(theme::PANEL_ALT))
                    .text_color(c(theme::FAINT))
                    .child(div().flex_1().truncate().child(hunk.header.clone()))
                    .when_some(partial, |d, staged| {
                        d.child(
                            Button::new(("hunk-apply", h))
                                .label(if staged { "Unstage hunk" } else { "Stage hunk" })
                                .xsmall()
                                .outline()
                                .on_click(cx.listener(move |this, _, _, cx| this.apply_partial(vec![h], false, false, cx))),
                        )
                        .when(!staged, |d| {
                            d.child(
                                Button::new(("hunk-discard", h))
                                    .label("Discard hunk")
                                    .xsmall()
                                    .ghost()
                                    .on_click(cx.listener(move |this, _, window, cx| {
                                        this.confirm_discard(Some(h), window, cx);
                                    })),
                            )
                        })
                    })
                    .into_any_element(),
            );

            for (l, line) in hunk.lines.iter().enumerate() {
                let (bg, sign) = match line.kind {
                    DiffLineKind::Add => (Some(theme::ADD_BG), "+"),
                    DiffLineKind::Del => (Some(theme::DEL_BG), "-"),
                    DiffLineKind::Context => (None, " "),
                };
                let pickable = partial.is_some() && line.kind != DiffLineKind::Context;
                let picked = view.selected.contains(&(h, l));
                let num = |n: Option<u32>| {
                    div()
                        .w(px(44.))
                        .flex_none()
                        .text_right()
                        .pr_2()
                        .text_color(c(theme::FAINT))
                        .child(n.map(|n| n.to_string()).unwrap_or_default())
                };
                let content = line.content.trim_end_matches('\r').replace('\t', "    ");
                rows.push(
                    div()
                        .id(("dl", h * 1_000_000 + l))
                        .flex()
                        .border_l_2()
                        .border_color(if picked { c(theme::ACCENT) } else { gpui_kit::transparent_black().into() })
                        .when_some(bg, |d, bg| d.bg(c(bg)))
                        .when(picked, |d| d.bg(c(theme::SELECTED)))
                        .when(pickable, |d| {
                            d.cursor_pointer()
                                .hover(|s| s.bg(c(theme::HOVER)))
                                .on_click(cx.listener(move |this, _, _, cx| this.toggle_line(h, l, cx)))
                        })
                        .child(num(line.old_no))
                        .child(num(line.new_no))
                        .child(div().w(px(14.)).flex_none().child(sign))
                        .child(div().whitespace_nowrap().child(content))
                        .into_any_element(),
                );
            }
        }

        let header = div()
            .h(px(36.))
            .flex_none()
            .flex()
            .items_center()
            .gap_2()
            .px_3()
            .border_b_1()
            .border_color(c(theme::BORDER))
            .child(div().flex_1().min_w_0().truncate().text_sm().font_weight(FontWeight::BOLD).child(view.title.clone()))
            .when(partial.is_some() && selected == 0, |d| {
                d.child(div().text_xs().text_color(c(theme::FAINT)).child("click lines to select"))
            })
            .when_some(partial.filter(|_| selected > 0), |d, staged| {
                let noun = if selected == 1 { "line" } else { "lines" };
                d.child(
                    Button::new("lines-apply")
                        .label(format!("{} {selected} {noun}", if staged { "Unstage" } else { "Stage" }))
                        .xsmall()
                        .primary()
                        .on_click(cx.listener(|this, _, _, cx| {
                            let hunks = this.selected_hunks();
                            this.apply_partial(hunks, true, false, cx);
                        })),
                )
                .when(!staged, |d| {
                    d.child(
                        Button::new("lines-discard")
                            .label(format!("Discard {selected} {noun}"))
                            .xsmall()
                            .ghost()
                            .on_click(cx.listener(|this, _, window, cx| this.confirm_discard(None, window, cx))),
                    )
                })
                .child(Button::new("lines-clear").label("Clear").xsmall().ghost().on_click(cx.listener(|this, _, _, cx| {
                    if let Some(view) = &mut this.diff {
                        view.selected.clear();
                    }
                    cx.notify();
                })))
            })
            .child(div().text_xs().text_color(c(theme::ADD)).child(format!("+{}", view.diff.additions)))
            .child(div().text_xs().text_color(c(theme::DEL)).child(format!("-{}", view.diff.deletions)))
            .child(Button::new("close-diff").label("Close").xsmall().ghost().on_click(cx.listener(|this, _, _, cx| {
                this.diff = None;
                cx.notify();
            })));

        div()
            .flex_1()
            .min_w_0()
            .h_full()
            .flex()
            .flex_col()
            .bg(c(theme::BG))
            .child(header)
            .child(
                div()
                    .id("diff-body")
                    .flex_1()
                    .overflow_scroll()
                    .font_family("Menlo")
                    .text_xs()
                    .text_color(c(theme::TEXT))
                    .children(rows),
            )
    }

    fn selected_hunks(&self) -> Vec<usize> {
        let Some(view) = &self.diff else { return Vec::new() };
        let mut hunks: Vec<usize> = view.selected.iter().map(|(h, _)| *h).collect();
        hunks.sort_unstable();
        hunks.dedup();
        hunks
    }

    /// Discard one hunk (`Some`) or the selected lines (`None`) after confirming.
    fn confirm_discard(&mut self, hunk: Option<usize>, window: &mut Window, cx: &mut Context<Self>) {
        let what = match hunk {
            Some(_) => "this hunk".to_string(),
            None => format!("{} selected line(s)", self.diff.as_ref().map_or(0, |d| d.selected.len())),
        };
        let Some(view) = &self.diff else { return };
        let (hunks, lines_only) = match hunk {
            Some(h) => (vec![h], false),
            None => (self.selected_hunks(), true),
        };
        let selected = lines_only.then_some(&view.selected);
        let Some(patch) = gitchrono_core::partial_patch(&view.diff, &hunks, selected, gitchrono_core::ApplyMode::Discard) else {
            return;
        };
        self.confirm(
            "Discard changes",
            format!("Discard {what} from the working tree? This cannot be undone."),
            "Discard",
            "Discarding",
            window,
            cx,
            move |r| r.apply_patch(&patch, gitchrono_core::ApplyMode::Discard),
        );
    }
}
