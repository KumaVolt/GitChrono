//! Drag a branch onto another to merge or rebase.

use gitchrono_core::Repo;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::Sizable as _;
use gpui_kit::prelude::*;
use gpui_kit::*;

use crate::theme::{self, c};
use crate::watcher::Change;
use crate::workspace::{Op, Workspace};

#[derive(Clone)]
pub struct DraggedRef {
    pub name: String,
}

pub struct DragPreview(pub SharedString);

impl Render for DragPreview {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .px_2()
            .py_0p5()
            .rounded_md()
            .text_sm()
            .bg(c(theme::SELECTED))
            .border_1()
            .border_color(c(theme::ACCENT))
            .text_color(c(theme::TEXT))
            .child(self.0.clone())
    }
}

pub struct DropMenu {
    title: SharedString,
    options: Vec<(SharedString, String, Option<Op>)>,
}

fn op(f: impl FnOnce(&Repo) -> anyhow::Result<()> + Send + 'static) -> Option<Op> {
    Some(Box::new(f))
}

impl Workspace {
    /// `source` was dropped on the local branch `target`.
    pub fn drop_ref(&mut self, source: &DraggedRef, target: String, cx: &mut Context<Self>) {
        let source = source.name.clone();
        if source == target {
            return;
        }
        let current = self.status.branch.clone().unwrap_or_default();
        let mut options: Vec<(SharedString, String, Option<Op>)> = Vec::new();
        if target == current {
            let (s1, s2) = (source.clone(), source.clone());
            options.push((format!("Merge {source} into {current}").into(), "Merging".into(), op(move |r| r.merge(&s1))));
            options.push((format!("Rebase {current} onto {source}").into(), "Rebasing".into(), op(move |r| r.rebase(&s2))));
        } else if source == current {
            let (t1, t2, s) = (target.clone(), target.clone(), source.clone());
            options.push((format!("Rebase {current} onto {target}").into(), "Rebasing".into(), op(move |r| r.rebase(&t1))));
            options.push((
                format!("Merge {current} into {target} (checks out {target})").into(),
                "Merging".into(),
                op(move |r| {
                    r.checkout(&t2)?;
                    r.merge(&s)
                }),
            ));
        } else {
            let (t1, s1, t2, s2) = (target.clone(), source.clone(), target.clone(), source.clone());
            options.push((
                format!("Merge {source} into {target} (checks out {target})").into(),
                "Merging".into(),
                op(move |r| {
                    r.checkout(&t1)?;
                    r.merge(&s1)
                }),
            ));
            options.push((
                format!("Rebase {target} onto {source} (checks out {target})").into(),
                "Rebasing".into(),
                op(move |r| {
                    r.checkout(&t2)?;
                    r.rebase(&s2)
                }),
            ));
        }
        self.drop_menu = Some(DropMenu { title: format!("{source} → {target}").into(), options });
        cx.notify();
    }

    fn run_drop_option(&mut self, index: usize, cx: &mut Context<Self>) {
        let Some(mut menu) = self.drop_menu.take() else { return };
        if let Some((_, busy, Some(op))) = menu.options.get_mut(index).map(|(l, b, o)| (l.clone(), b.clone(), o.take())) {
            self.run(busy, Change::Refs, cx, op);
        }
        cx.notify();
    }

    pub fn render_drop_menu(&self, cx: &mut Context<Self>) -> Option<impl IntoElement> {
        let menu = self.drop_menu.as_ref()?;
        Some(
            div()
                .id("drop-backdrop")
                .absolute()
                .inset_0()
                .flex()
                .items_center()
                .justify_center()
                .bg(gpui_kit::black().opacity(0.4))
                .on_click(cx.listener(|this, _, _, cx| {
                    this.drop_menu = None;
                    cx.notify();
                }))
                .child(
                    div()
                        .id("drop-menu")
                        .w(px(440.))
                        .p_4()
                        .flex()
                        .flex_col()
                        .gap_2()
                        .rounded_lg()
                        .bg(c(theme::PANEL))
                        .border_1()
                        .border_color(c(theme::BORDER))
                        .shadow_lg()
                        .on_click(|_, _, cx| cx.stop_propagation())
                        .child(div().text_base().font_weight(FontWeight::BOLD).child(menu.title.clone()))
                        .children(menu.options.iter().enumerate().map(|(i, (label, _, _))| {
                            Button::new(("drop-option", i))
                                .label(label.clone())
                                .small()
                                .when(i == 0, |b| b.primary())
                                .when(i > 0, |b| b.outline())
                                .on_click(cx.listener(move |this, _, _, cx| this.run_drop_option(i, cx)))
                        }))
                        .child(Button::new("drop-cancel").label("Cancel").small().ghost().on_click(cx.listener(|this, _, _, cx| {
                            this.drop_menu = None;
                            cx.notify();
                        }))),
                ),
        )
    }
}
