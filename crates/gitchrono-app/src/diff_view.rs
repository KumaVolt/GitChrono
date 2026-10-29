//! Unified diff view with hunk- and line-level stage / unstage / discard.

use gitchrono_core::DiffLineKind;
use gpui_kit::component::button::ButtonVariants as _;
use gpui_kit::component::Sizable as _;
use gpui_kit::prelude::*;
use gpui_kit::*;

use crate::highlight::{Highlights, display_text};
use crate::theme::{self, c};
use crate::workspace::{DiffView, Workspace};
use crate::ui::button;

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
                            button(("hunk-apply", h))
                                .label(if staged { "Unstage hunk" } else { "Stage hunk" })
                                .xsmall()
                                .outline()
                                .on_click(cx.listener(move |this, _, _, cx| this.apply_partial(vec![h], false, false, cx))),
                        )
                        .when(!staged, |d| {
                            d.child(
                                button(("hunk-discard", h))
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

            if self.diff_split {
                for (left, right) in split_pairs(&hunk.lines) {
                    rows.push(
                        div()
                            .flex()
                            .w_full()
                            .child(self.split_cell(view, h, left, partial.is_some(), true, cx))
                            .child(div().w_px().flex_none().h_full().bg(c(theme::BORDER)))
                            .child(self.split_cell(view, h, right, partial.is_some(), false, cx))
                            .into_any_element(),
                    );
                }
                continue;
            }
            for (l, line) in hunk.lines.iter().enumerate() {
                let (bg, sign) = line_style(line.kind);
                let pickable = partial.is_some() && line.kind != DiffLineKind::Context;
                let picked = view.selected.contains(&(h, l));
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
                        .child(line_number(line.old_no))
                        .child(line_number(line.new_no))
                        .child(div().w(px(14.)).flex_none().child(sign))
                        .child(code(&view.highlights, h, l, &line.content))
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
                    button("lines-apply")
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
                        button("lines-discard")
                            .label(format!("Discard {selected} {noun}"))
                            .xsmall()
                            .ghost()
                            .on_click(cx.listener(|this, _, window, cx| this.confirm_discard(None, window, cx))),
                    )
                })
                .child(button("lines-clear").label("Clear").xsmall().ghost().on_click(cx.listener(|this, _, _, cx| {
                    if let Some(view) = &mut this.diff {
                        view.selected.clear();
                    }
                    cx.notify();
                })))
            })
            .child(
                button("diff-layout")
                    .label(if self.diff_split { "Unified" } else { "Split" })
                    .xsmall()
                    .ghost()
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.diff_split = !this.diff_split;
                        cx.notify();
                    })),
            )
            .child(div().text_xs().text_color(c(theme::ADD)).child(format!("+{}", view.diff.additions)))
            .child(div().text_xs().text_color(c(theme::DEL)).child(format!("-{}", view.diff.deletions)))
            .child(button("close-diff").label("Close").xsmall().ghost().on_click(cx.listener(|this, _, _, cx| {
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

    /// One side of a split row; `line` is the index within the hunk, if any.
    fn split_cell(&self, view: &DiffView, h: usize, line: Option<usize>, partial: bool, left: bool, cx: &mut Context<Self>) -> AnyElement {
        let Some(l) = line else {
            return div().flex_1().min_w_0().bg(c(theme::PANEL)).into_any_element();
        };
        let diff_line = &view.diff.hunks[h].lines[l];
        let (bg, sign) = line_style(diff_line.kind);
        let pickable = partial && diff_line.kind != DiffLineKind::Context;
        let picked = view.selected.contains(&(h, l));
        let number = if left { diff_line.old_no } else { diff_line.new_no };
        div()
            .id((if left { "sl" } else { "sr" }, h * 1_000_000 + l))
            .flex_1()
            .min_w_0()
            .flex()
            .overflow_hidden()
            .when_some(bg, |d, bg| d.bg(c(bg)))
            .when(picked, |d| d.bg(c(theme::SELECTED)))
            .when(pickable, |d| {
                d.cursor_pointer().hover(|s| s.bg(c(theme::HOVER))).on_click(cx.listener(move |this, _, _, cx| this.toggle_line(h, l, cx)))
            })
            .child(line_number(number))
            .child(div().w(px(14.)).flex_none().child(sign))
            .child(code(&view.highlights, h, l, &diff_line.content))
            .into_any_element()
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

fn line_style(kind: DiffLineKind) -> (Option<u32>, &'static str) {
    match kind {
        DiffLineKind::Add => (Some(theme::ADD_BG), "+"),
        DiffLineKind::Del => (Some(theme::DEL_BG), "-"),
        DiffLineKind::Context => (None, " "),
    }
}

fn line_number(n: Option<u32>) -> Div {
    div().w(px(44.)).flex_none().text_right().pr_2().text_color(c(theme::FAINT)).child(n.map(|n| n.to_string()).unwrap_or_default())
}

/// The line's text, syntax-colored when highlights exist.
fn code(highlights: &Highlights, h: usize, l: usize, content: &str) -> Div {
    let text = display_text(content);
    let styled = match highlights.get(&(h, l)) {
        Some(spans) => StyledText::new(text)
            .with_highlights(spans.iter().map(|(range, color)| (range.clone(), HighlightStyle { color: Some(*color), ..Default::default() }))),
        None => StyledText::new(text),
    };
    div().whitespace_nowrap().child(styled)
}

/// Rows for a split view: context on both sides, runs of deletions paired
/// with the additions that follow them.
fn split_pairs(lines: &[gitchrono_core::DiffLine]) -> Vec<(Option<usize>, Option<usize>)> {
    let mut rows = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        if lines[i].kind == DiffLineKind::Context {
            rows.push((Some(i), Some(i)));
            i += 1;
            continue;
        }
        let dels: Vec<usize> = (i..lines.len()).take_while(|&j| lines[j].kind == DiffLineKind::Del).collect();
        let after = i + dels.len();
        let adds: Vec<usize> = (after..lines.len()).take_while(|&j| lines[j].kind == DiffLineKind::Add).collect();
        for k in 0..dels.len().max(adds.len()) {
            rows.push((dels.get(k).copied(), adds.get(k).copied()));
        }
        i = after + adds.len();
    }
    rows
}

#[cfg(test)]
mod tests {
    use super::split_pairs;
    use gitchrono_core::parse_patch;

    #[test]
    fn pairs_deletions_with_following_additions() {
        let patch = "diff --git a/f b/f\n--- a/f\n+++ b/f\n@@ -1,4 +1,4 @@\n a\n-b\n-c\n+B\n d\n+e\n";
        let diff = parse_patch("f", patch);
        let rows = split_pairs(&diff.hunks[0].lines);
        assert_eq!(rows, vec![(Some(0), Some(0)), (Some(1), Some(3)), (Some(2), None), (Some(4), Some(4)), (None, Some(5))]);
    }
}
