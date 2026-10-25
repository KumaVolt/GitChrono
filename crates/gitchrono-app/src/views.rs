//! Rendering for the repository window (everything but the diff view).

use std::time::{SystemTime, UNIX_EPOCH};

use gitchrono_core::{Commit, FileChange, FileStatus, Operation, RefKind, RefLabel};
use gitchrono_graph::Row;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Input, Textarea};
use gpui_kit::component::menu::ContextMenuExt as _;
use gpui_kit::component::{Disableable as _, Sizable as _};
use gpui_kit::prelude::*;
use gpui_kit::*;

use crate::file_tree::{self, TreeRow};
use crate::graph_paint::{self, Node, ROW_HEIGHT};
use crate::menus;
use crate::theme::{self, c};
use crate::watcher::Change;
use crate::workspace::{DiffSource, Selection, Workspace, WorkspaceEvent};

pub fn relative_time(secs: i64) -> String {
    let now = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(secs);
    let d = (now - secs).max(0);
    let (n, unit) = match d {
        0..60 => return "just now".into(),
        60..3600 => (d / 60, "minute"),
        3600..86_400 => (d / 3600, "hour"),
        86_400..2_592_000 => (d / 86_400, "day"),
        2_592_000..31_536_000 => (d / 2_592_000, "month"),
        _ => (d / 31_536_000, "year"),
    };
    format!("{n} {unit}{} ago", if n == 1 { "" } else { "s" })
}

fn ref_badge(label: &RefLabel, color: Hsla) -> Div {
    let (prefix, fg) = match label.kind {
        RefKind::Tag => ("◆ ", c(theme::WARN).into()),
        RefKind::Stash => ("≡ ", c(theme::MUTED).into()),
        RefKind::Remote => ("☁ ", color),
        RefKind::Head => ("➤ ", color),
        RefKind::Local if label.current => ("✓ ", color),
        RefKind::Local => ("", color),
    };
    div()
        .flex_none()
        .px_1p5()
        .h(px(18.))
        .flex()
        .items_center()
        .rounded_sm()
        .border_1()
        .border_color(fg)
        .bg(fg.opacity(0.15))
        .text_xs()
        .text_color(c(theme::TEXT))
        .when(label.current, |d| d.font_weight(FontWeight::BOLD))
        .child(format!("{prefix}{}", label.name))
}

fn status_letter(change: &FileChange) -> Div {
    use gitchrono_core::FileStatus::*;
    let color = match change.status {
        Added | Untracked => theme::ADD,
        Deleted => theme::DEL,
        Conflicted => theme::WARN,
        _ => theme::ACCENT,
    };
    div().w(px(16.)).flex_none().text_xs().font_weight(FontWeight::BOLD).text_color(c(color)).child(change.status.letter())
}

fn section_header(title: &str, count: usize) -> Div {
    div()
        .flex_1()
        .px_3()
        .pt_3()
        .pb_1()
        .flex()
        .justify_between()
        .text_xs()
        .font_weight(FontWeight::BOLD)
        .text_color(c(theme::MUTED))
        .child(title.to_uppercase())
        .child(count.to_string())
}

impl Workspace {
    fn render_toolbar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let st = &self.status;
        let busy = self.busy.is_some();
        let branch = st.branch.clone().unwrap_or_else(|| "detached HEAD".into());
        let tool = |id: &'static str, label: String| Button::new(id).label(label).ghost().small().disabled(busy);

        div()
            .h(px(44.))
            .flex_none()
            .flex()
            .items_center()
            .gap_1()
            .px_3()
            .bg(c(theme::PANEL))
            .border_b_1()
            .border_color(c(theme::BORDER))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .mr_4()
                    .child(
                        div()
                            .text_xs()
                            .text_color(c(theme::MUTED))
                            .child(self.repo.as_ref().map(|r| r.name()).unwrap_or_else(|| "GitChrono".into())),
                    )
                    .child(div().text_sm().font_weight(FontWeight::BOLD).child(branch)),
            )
            .when(self.repo.is_some(), |bar| {
                let undo = self.undo_stack.last().map(|e| e.label.clone());
                let redo = self.redo_stack.last().map(|e| e.label.clone());
                bar.child(
                    Button::new("undo")
                        .label("↶")
                        .ghost()
                        .small()
                        .tooltip(undo.clone().map_or("Nothing to undo".to_string(), |l| format!("Undo {} (⌘Z)", l.to_lowercase())))
                        .disabled(undo.is_none() || busy)
                        .on_click(cx.listener(|this, _, _, cx| this.undo(cx))),
                )
                .child(
                    Button::new("redo")
                        .label("↷")
                        .ghost()
                        .small()
                        .tooltip(redo.clone().map_or("Nothing to redo".to_string(), |l| format!("Redo {} (⌘⇧Z)", l.to_lowercase())))
                        .disabled(redo.is_none() || busy)
                        .on_click(cx.listener(|this, _, _, cx| this.redo(cx))),
                )
            })
            .child(Button::new("open").label("Open").ghost().small().on_click(cx.listener(|this, _, _, cx| this.prompt_open(cx))))
            .when(self.repo.is_some(), |bar| {
                bar.child(tool("fetch", "Fetch".into()).on_click(cx.listener(|this, _, _, cx| {
                    this.load_pull_requests(cx);
                    this.run("Fetching", Change::Refs, cx, |r| r.fetch());
                })))
                .child(
                    tool("pull", if st.behind > 0 { format!("Pull ↓{}", st.behind) } else { "Pull".into() })
                        .on_click(cx.listener(|this, _, _, cx| this.run("Pulling", Change::Refs, cx, |r| r.pull(false)))),
                )
                .child(
                    tool("push", if st.ahead > 0 { format!("Push ↑{}", st.ahead) } else { "Push".into() })
                        .on_click(cx.listener(|this, _, _, cx| this.run("Pushing", Change::Refs, cx, |r| r.push(false)))),
                )
                .child(tool("branch", "Branch".into()).on_click(cx.listener(|this, _, window, cx| this.prompt_create_branch(window, cx))))
                .child(tool("stash", "Stash".into()).disabled(busy || st.is_clean()).on_click(cx.listener(|this, _, window, cx| this.prompt_stash(window, cx))))
                .child(tool("terminal", "Terminal".into()).on_click(cx.listener(|this, _, window, cx| this.toggle_terminal(window, cx))))
                .child(tool("pop", "Pop".into()).disabled(busy || self.snap.refs.stashes.is_empty()).on_click(cx.listener(
                    |this, _, _, cx| this.run("Popping stash", Change::Refs, cx, |r| r.stash_pop(0)),
                )))
            })
            .child(div().flex_1())
            .when_some(self.busy.clone(), |bar, msg| bar.child(div().text_xs().text_color(c(theme::MUTED)).child(msg)))
            .child(
                Button::new("ai-settings-open")
                    .label(if self.ai.is_some() { "✨ AI" } else { "✨ Set up AI" })
                    .ghost()
                    .small()
                    .on_click(cx.listener(|this, _, window, cx| this.open_ai_settings(window, cx))),
            )
    }

    fn render_banners(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let op = self.status.operation;
        div()
            .flex_none()
            .when_some(self.error.clone(), |d, err| {
                d.child(
                    div()
                        .id("error")
                        .px_3()
                        .py_2()
                        .flex()
                        .gap_2()
                        .bg(c(theme::DEL_BG))
                        .border_b_1()
                        .border_color(c(theme::DEL))
                        .text_xs()
                        .child(div().flex_1().child(err))
                        .child(Button::new("dismiss").label("Dismiss").ghost().xsmall().on_click(cx.listener(|this, _, _, cx| {
                            this.error = None;
                            cx.notify();
                        }))),
                )
            })
            .when(op != Operation::None, |d| {
                let conflicts = self.status.conflicted.len();
                d.child(
                    div()
                        .px_3()
                        .py_1p5()
                        .flex()
                        .items_center()
                        .gap_2()
                        .bg(c(0x3a3320))
                        .border_b_1()
                        .border_color(c(theme::WARN))
                        .text_xs()
                        .child(div().flex_1().child(format!(
                            "{} in progress — {conflicts} conflicted file(s)",
                            menus::operation_name(op)
                        )))
                        .child(
                            Button::new("op-continue").label("Continue").xsmall().primary().disabled(conflicts > 0).on_click(
                                cx.listener(move |this, _, _, cx| {
                                    this.run("Continuing", Change::Refs, cx, move |r| r.resolve_operation(op, false))
                                }),
                            ),
                        )
                        .child(Button::new("op-abort").label("Abort").xsmall().ghost().on_click(cx.listener(move |this, _, _, cx| {
                            this.run("Aborting", Change::Refs, cx, move |r| r.resolve_operation(op, true))
                        }))),
                )
            })
    }

    fn render_sidebar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let refs = &self.snap.refs;
        let ws = cx.entity().downgrade();
        let current = self.status.branch.clone();
        let ref_filter = self.ref_filter.clone();
        let item = |id: SharedString, label: String, extra: String, target: String, is_current: bool, kind: RefKind, index: usize, cx: &mut Context<Self>| {
            let name = label.clone();
            let filter_state = (ref_filter.hidden.contains(&label), ref_filter.solo.contains(&label));
            let (drag_name, drop_target) = (label.clone(), label.clone());
            let shown = !matches!(kind, RefKind::Local | RefKind::Remote) || ref_filter.shows(&label, is_current);
            let extra = if filter_state.1 { format!("solo {extra}") } else if !shown { format!("hidden {extra}") } else { extra };
            let ws = ws.clone();
            let current = current.clone();
            div()
                .id(id)
                .px_3()
                .py_0p5()
                .flex()
                .gap_1p5()
                .text_sm()
                .text_color(c(if is_current { theme::TEXT } else { theme::MUTED }))
                .when(is_current, |d| d.font_weight(FontWeight::BOLD))
                .when(!shown, |d| d.opacity(0.45))
                .hover(|s| s.bg(c(theme::HOVER)))
                .cursor_pointer()
                .child(div().w(px(12.)).flex_none().text_color(c(theme::ADD)).child(if is_current { "✓" } else { "" }))
                .child(div().truncate().child(label.clone()))
                .child(div().flex_none().text_xs().text_color(c(theme::FAINT)).child(extra))
                .on_click(cx.listener(move |this, ev: &ClickEvent, _, cx| {
                    if ev.click_count() >= 2 {
                        let t = label.clone();
                        match kind {
                            RefKind::Local => this.run("Checking out", Change::Refs, cx, move |r| r.checkout(&t)),
                            RefKind::Remote => this.run("Checking out", Change::Refs, cx, move |r| r.checkout_remote(&t)),
                            _ => {}
                        }
                    } else {
                        this.select_commit_id(&target, cx);
                    }
                }))
                .when(matches!(kind, RefKind::Local | RefKind::Remote), |d| {
                    d.on_drag(crate::drag::DraggedRef { name: drag_name.clone() }, |r, _, _, cx| {
                        cx.new(|_| crate::drag::DragPreview(r.name.clone().into()))
                    })
                })
                .when(kind == RefKind::Local, |d| {
                    let target = drop_target.clone();
                    d.drag_over::<crate::drag::DraggedRef>(|s, _, _, _| s.bg(c(theme::SELECTED)))
                        .on_drop(cx.listener(move |this, dragged: &crate::drag::DraggedRef, _, cx| this.drop_ref(dragged, target.clone(), cx)))
                })
                .context_menu(move |menu, _, _| menus::ref_menu(menu, &ws, &name, kind, current.as_deref(), index, filter_state))
        };

        let mut col = div()
            .id("sidebar")
            .w(px(240.))
            .flex_none()
            .h_full()
            .overflow_y_scroll()
            .bg(c(theme::PANEL))
            .border_r_1()
            .border_color(c(theme::BORDER))
            .pb_4();

        let filter = self.ref_filter(cx);
        let keep = |name: &str| filter.is_empty() || name.to_lowercase().contains(&filter);
        col = col.child(div().p_2().child(Input::new(&self.sidebar_filter).small().cleanable(true)));

        col = col.child(section_header("Local", refs.local.len()));
        for b in refs.local.iter().filter(|b| keep(&b.name)) {
            let extra = match (b.ahead, b.behind) {
                (0, 0) => String::new(),
                (a, 0) => format!("↑{a}"),
                (0, bh) => format!("↓{bh}"),
                (a, bh) => format!("↑{a} ↓{bh}"),
            };
            col = col.child(item(format!("l-{}", b.name).into(), b.name.clone(), extra, b.target.clone(), b.current, RefKind::Local, 0, cx));
        }
        col = col.child(section_header("Remote", refs.remote.len()));
        for b in refs.remote.iter().filter(|b| keep(&b.name)) {
            col = col.child(item(format!("r-{}", b.name).into(), b.name.clone(), String::new(), b.target.clone(), false, RefKind::Remote, 0, cx));
        }
        col = col.child(section_header("Tags", refs.tags.len()));
        for t in refs.tags.iter().filter(|t| keep(&t.name)).take(300) {
            col = col.child(item(format!("t-{}", t.name).into(), t.name.clone(), String::new(), t.target.clone(), false, RefKind::Tag, 0, cx));
        }
        let worktrees = &self.snap.worktrees;
        if worktrees.len() > 1 {
            let root = self.repo.as_ref().map(|r| r.root().to_path_buf()).unwrap_or_default();
            col = col.child(section_header("Worktrees", worktrees.len()));
            for (i, wt) in worktrees.iter().enumerate() {
                let name = crate::projects::display_name(&wt.path);
                if !keep(&name) && !wt.branch.as_deref().is_some_and(|b| keep(b)) {
                    continue;
                }
                let here = wt.path == root || wt.path.canonicalize().ok() == root.canonicalize().ok();
                let branch = wt.branch.clone().unwrap_or_else(|| "detached".into());
                let open = wt.path.clone();
                let for_menu = wt.path.clone();
                let removable = !wt.main && !here;
                let ws = cx.entity().downgrade();
                col = col.child(
                    div()
                        .id(("wt", i))
                        .px_3()
                        .py_0p5()
                        .flex()
                        .gap_1p5()
                        .text_sm()
                        .text_color(c(if here { theme::TEXT } else { theme::MUTED }))
                        .when(here, |d| d.font_weight(FontWeight::BOLD))
                        .hover(|s| s.bg(c(theme::HOVER)))
                        .cursor_pointer()
                        .child(div().w(px(12.)).flex_none().text_color(c(theme::ADD)).child(if here { "✓" } else { "" }))
                        .child(div().truncate().child(name))
                        .child(div().flex_none().text_xs().text_color(c(theme::FAINT)).child(branch))
                        .on_click(cx.listener(move |_, _, _, cx| cx.emit(WorkspaceEvent::OpenPath(open.clone()))))
                        .context_menu(move |menu, _, _| menus::worktree_menu(menu, &ws, for_menu.clone(), removable)),
                );
            }
        }
        if !self.snap.submodules.is_empty() {
            let root = self.repo.as_ref().map(|r| r.root().to_path_buf()).unwrap_or_default();
            col = col.child(section_header("Submodules", self.snap.submodules.len()));
            for (i, sub) in self.snap.submodules.iter().enumerate().filter(|(_, s)| keep(&s.path)) {
                let state = match sub.state {
                    '-' => "not initialized",
                    '+' => "changed",
                    'U' => "conflict",
                    _ => "",
                };
                let full = root.join(&sub.path);
                let (open, update, copy) = (full.clone(), sub.path.clone(), full.to_string_lossy().into_owned());
                let initialized = sub.state != '-';
                let ws = cx.entity().downgrade();
                col = col.child(
                    div()
                        .id(("sub", i))
                        .px_3()
                        .py_0p5()
                        .pl(px(30.))
                        .flex()
                        .gap_1p5()
                        .text_sm()
                        .text_color(c(theme::MUTED))
                        .hover(|s| s.bg(c(theme::HOVER)))
                        .cursor_pointer()
                        .child(div().truncate().child(sub.path.clone()))
                        .child(div().flex_none().text_xs().text_color(c(if state.is_empty() { theme::FAINT } else { theme::WARN })).child(state))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            if initialized {
                                cx.emit(WorkspaceEvent::OpenPath(open.clone()));
                            } else {
                                let p = update.clone();
                                this.run("Initializing submodule", Change::Refs, cx, move |r| r.update_submodule(&p));
                            }
                        }))
                        .context_menu({
                            let (open, update) = (full.clone(), sub.path.clone());
                            move |menu, _, _| menus::submodule_menu(menu, &ws, open.clone(), update.clone(), copy.clone())
                        }),
                );
            }
        }
        if let Some(prs) = &self.pull_requests {
            col = col.child(
                div()
                    .flex()
                    .items_center()
                    .pr_2()
                    .child(section_header("Pull requests", prs.len()))
                    .child(Button::new("pr-new").label("New").xsmall().ghost().on_click(cx.listener(|this, _, _, cx| {
                        let Some(root) = this.repo.as_ref().map(|r| r.root().to_path_buf()) else { return };
                        this.run_inner("Opening pull request form".into(), Change::Refs, false, cx, move |_| crate::github::create_in_browser(&root), |_, _| {});
                    }))),
            );
            for (i, pr) in prs.iter().enumerate().filter(|(_, p)| keep(&p.title) || keep(&p.head_ref_name)) {
                let url = pr.url.clone();
                let number = pr.number;
                let ws = cx.entity().downgrade();
                col = col.child(
                    div()
                        .id(("pr", i))
                        .px_3()
                        .py_0p5()
                        .pl(px(30.))
                        .flex()
                        .gap_1p5()
                        .text_sm()
                        .text_color(c(theme::MUTED))
                        .hover(|s| s.bg(c(theme::HOVER)))
                        .cursor_pointer()
                        .child(div().flex_none().text_color(c(theme::FAINT)).child(format!("#{}", pr.number)))
                        .child(div().truncate().child(pr.title.clone()))
                        .when(pr.is_draft, |d| d.child(div().flex_none().text_xs().text_color(c(theme::FAINT)).child("draft")))
                        .child(div().flex_none().text_xs().text_color(c(theme::FAINT)).child(pr.author.login.clone()))
                        .on_click(cx.listener(move |_, _, _, cx| cx.open_url(&url)))
                        .context_menu({
                            let url = pr.url.clone();
                            move |menu, _, _| menus::pr_menu(menu, &ws, number, url.clone())
                        }),
                );
            }
        }
        col = col.child(section_header("Stashes", refs.stashes.len()));
        for s in refs.stashes.iter().filter(|s| keep(&s.message)) {
            col = col.child(item(format!("s-{}", s.index).into(), s.message.clone(), String::new(), s.target.clone(), false, RefKind::Stash, s.index, cx));
        }
        col
    }

    fn render_graph(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let (gutter, lane_width) = graph_paint::gutter(self.graph.max_lane);
        let header = div()
            .h(px(26.))
            .flex_none()
            .flex()
            .items_center()
            .text_xs()
            .font_weight(FontWeight::BOLD)
            .text_color(c(theme::FAINT))
            .border_b_1()
            .border_color(c(theme::BORDER))
            .child(div().w(px(170.)).px_3().child("BRANCH / TAG"))
            .child(div().w(gutter).flex_none().px_2().overflow_hidden().whitespace_nowrap().child("GRAPH"))
            .child(div().flex_1().px_2().child("COMMIT MESSAGE"))
            .child(div().w(px(150.)).px_2().child("AUTHOR"))
            .child(div().w(px(110.)).px_2().child("DATE"));

        let list = uniform_list(
            "commits",
            self.row_count(),
            cx.processor(move |this, range: std::ops::Range<usize>, _, cx| {
                range.map(|row| this.render_row(row, gutter, lane_width, cx)).collect::<Vec<_>>()
            }),
        )
        .track_scroll(&self.list_scroll)
        .flex_1();

        div()
            .flex_1()
            .min_w_0()
            .h_full()
            .flex()
            .flex_col()
            .bg(c(theme::BG))
            .children(self.render_search_bar(cx))
            .when(!self.ref_filter.is_empty(), |d| {
                let f = &self.ref_filter;
                let text = if f.solo.is_empty() {
                    format!("{} branch{} hidden from the graph", f.hidden.len(), if f.hidden.len() == 1 { "" } else { "es" })
                } else {
                    format!("Graph shows only {}", f.solo.iter().cloned().collect::<Vec<_>>().join(", "))
                };
                d.child(
                    div()
                        .h(px(30.))
                        .flex_none()
                        .flex()
                        .items_center()
                        .gap_2()
                        .px_3()
                        .bg(c(theme::PANEL_ALT))
                        .border_b_1()
                        .border_color(c(theme::BORDER))
                        .text_xs()
                        .text_color(c(theme::MUTED))
                        .child(div().flex_1().child(text))
                        .child(Button::new("filter-clear").label("Show all").xsmall().ghost().on_click(cx.listener(|this, _, _, cx| this.clear_ref_filter(cx)))),
                )
            })
            .child(header)
            .child(list)
    }

    fn render_search_bar(&self, cx: &mut Context<Self>) -> Option<impl IntoElement> {
        let search = self.search.as_ref()?;
        let query_empty = search.input.read(cx).value().trim().is_empty();
        let count = match (query_empty, search.matches.len()) {
            (true, _) => String::new(),
            (false, 0) => "No matches".into(),
            (false, n) => format!("{} of {n}", search.current + 1),
        };
        Some(
            div()
                .key_context("GraphSearch")
                .on_action(cx.listener(|this, _: &gpui_kit::component::input::Escape, _, cx| this.close_search(cx)))
                .h(px(40.))
                .flex_none()
                .flex()
                .items_center()
                .gap_2()
                .px_3()
                .bg(c(theme::PANEL))
                .border_b_1()
                .border_color(c(theme::BORDER))
                .child(div().flex_1().max_w(px(420.)).child(Input::new(&search.input).small()))
                .child(div().text_xs().text_color(c(theme::MUTED)).child(count))
                .child(Button::new("search-prev").label("↑").xsmall().ghost().on_click(cx.listener(|this, _, _, cx| this.step_search(-1, cx))))
                .child(Button::new("search-next").label("↓").xsmall().ghost().on_click(cx.listener(|this, _, _, cx| this.step_search(1, cx))))
                .child(div().flex_1())
                .child(Button::new("search-close").label("Close").xsmall().ghost().on_click(cx.listener(|this, _, _, cx| this.close_search(cx)))),
        )
    }

    fn render_row(&self, row: usize, gutter: Pixels, lane_width: f32, cx: &mut Context<Self>) -> AnyElement {
        let wip = self.has_wip();
        let base = div()
            .id(row)
            .h(px(ROW_HEIGHT))
            .w_full()
            .flex()
            .items_center()
            .text_sm()
            .cursor_pointer()
            .hover(|s| s.bg(c(theme::HOVER)))
            .on_click(cx.listener(move |this, ev: &ClickEvent, _, cx| this.click_row(row, ev.modifiers(), cx)));

        if wip && row == 0 {
            let selected = self.selection == Selection::Wip;
            // Hang the WIP node above whatever HEAD points at, not just the newest commit.
            let head = self.snap.refs.head_id.as_deref();
            let head_lane = self
                .snap
                .commits
                .iter()
                .position(|c| Some(c.id.as_str()) == head)
                .and_then(|i| self.graph.rows.get(i))
                .map_or(0, |r| r.lane);
            let n = self.status.change_count();
            return base
                .when(selected, |d| d.bg(c(theme::SELECTED)))
                .child(div().w(px(170.)).flex_none())
                .child(
                    div()
                        .w(gutter)
                        .h_full()
                        .flex_none()
                        .child(graph_paint::row_canvas(Row { lane: head_lane, ..Default::default() }, Node::Wip, lane_width)),
                )
                .child(
                    div()
                        .flex_1()
                        .px_2()
                        .italic()
                        .text_color(c(theme::MUTED))
                        .child(format!("// WIP — {n} uncommitted change{}", if n == 1 { "" } else { "s" })),
                )
                .into_any_element();
        }

        let index = row - usize::from(wip);
        let commit = &self.snap.commits[index];
        let layout = self.graph.rows[index].clone();
        let color = theme::lane(layout.color);
        let selected = self.selection == Selection::Commit(index) || self.multi.contains(&index);
        let merge = commit.parents.len() > 1;
        let dimmed = self.search.as_ref().is_some_and(|s| !s.matches.is_empty() && !s.hit.contains(&index));
        let ws = cx.entity().downgrade();
        let id = commit.id.clone();
        let current = self.status.branch.clone();

        base.when(selected, |d| d.bg(c(theme::SELECTED)))
            .when(dimmed, |d| d.opacity(0.3))
            .child(
                div()
                    .w(px(170.))
                    .flex_none()
                    .px_2()
                    .flex()
                    .gap_1()
                    .overflow_hidden()
                    .children(commit.refs.iter().take(3).map(|r| ref_badge(r, color)))
                    .when(commit.refs.len() > 3, |d| {
                        d.child(div().text_xs().text_color(c(theme::MUTED)).child(format!("+{}", commit.refs.len() - 3)))
                    }),
            )
            .child(div().w(gutter).h_full().flex_none().overflow_hidden().child(graph_paint::row_canvas(layout, Node::Commit { merge }, lane_width)))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .px_2()
                    .truncate()
                    .text_color(c(if merge { theme::MUTED } else { theme::TEXT }))
                    .child(commit.summary.clone()),
            )
            .child(div().w(px(150.)).flex_none().px_2().truncate().text_color(c(theme::MUTED)).child(commit.author_name.clone()))
            .child(div().w(px(110.)).flex_none().px_2().text_xs().text_color(c(theme::FAINT)).child(relative_time(commit.time)))
            .context_menu(move |menu, _, _| menus::commit_menu(menu, &ws, &id, current.as_deref()))
            .into_any_element()
    }

    fn render_detail(&self, commit: &Commit, cx: &mut Context<Self>) -> impl IntoElement {
        let id = commit.id.clone();
        let short = commit.short_id.clone();
        let ws = cx.entity().downgrade();
        let current = self.status.branch.clone();
        let menu_id = id.clone();

        div()
            .flex()
            .flex_col()
            .size_full()
            .child(
                div()
                    .id("commit-header")
                    .p_3()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .border_b_1()
                    .border_color(c(theme::BORDER))
                    .child(div().text_xs().text_color(c(theme::FAINT)).child(format!("commit {} · right-click for actions", commit.short_id)))
                    .child(div().text_base().font_weight(FontWeight::BOLD).child(commit.summary.clone()))
                    .when(!commit.body.is_empty(), |d| {
                        d.child(div().text_sm().text_color(c(theme::MUTED)).child(commit.body.trim().to_string()))
                    })
                    .child(
                        div()
                            .text_xs()
                            .text_color(c(theme::MUTED))
                            .child(format!("{} <{}> · {}", commit.author_name, commit.author_email, relative_time(commit.time))),
                    )
                    .when_some(self.commit_signature.clone(), |d, sig| {
                        let color = if sig.is_good() { theme::ADD } else if sig.status == 'B' || sig.status == 'R' { theme::DEL } else { theme::WARN };
                        let signer = if sig.signer.is_empty() { String::new() } else { format!(" · {}", sig.signer) };
                        d.child(div().text_xs().text_color(c(color)).child(format!("{} {}{signer}", if sig.is_good() { "✓" } else { "⚠" }, sig.describe())))
                    })
                    .when(!commit.parents.is_empty(), |d| {
                        d.child(div().text_xs().text_color(c(theme::FAINT)).child(format!(
                            "parent{} {}",
                            if commit.parents.len() > 1 { "s" } else { "" },
                            commit.parents.iter().map(|p| &p[..7]).collect::<Vec<_>>().join(" ")
                        )))
                    })
                    .context_menu(move |menu, _, _| menus::commit_menu(menu, &ws, &menu_id, current.as_deref())),
            )
            .child(div().flex().items_center().pr_2().child(section_header("Changed files", self.commit_files.len())).child(self.folders_toggle("folders-commit", cx)))
            .child(div().id("commit-files").flex_1().overflow_y_scroll().children(self.file_rows("commit", &self.commit_files).into_iter().enumerate().map(|(row, tree_row)| {
                let (i, name, depth) = match tree_row {
                    TreeRow::File { index, name, depth } => (index, name, depth),
                    TreeRow::Dir { path, name, depth, collapsed, .. } => {
                        return self.folder_row(("cf", row).into(), "commit", &path, &name, depth, collapsed, cx).into_any_element();
                    }
                };
                let f = &self.commit_files[i];
                let path = f.path.clone();
                let commit_id = id.clone();
                let title = format!("{path} @ {short}");
                div()
                    .id(("cf", row))
                    .pl(px(12. + depth as f32 * 14.))
                    .pr_3()
                    .py_0p5()
                    .flex()
                    .gap_1()
                    .text_sm()
                    .cursor_pointer()
                    .hover(|s| s.bg(c(theme::HOVER)))
                    .child(status_letter(f))
                    .child(div().truncate().child(name))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        let source = DiffSource::Commit { id: commit_id.clone(), path: path.clone() };
                        this.show_diff(title.clone(), source, cx);
                    }))
                    .context_menu({
                        let ws = cx.entity().downgrade();
                        let path = f.path.clone();
                        move |menu, _, _| menus::commit_file_menu(menu, &ws, &path)
                    })
                    .into_any_element()
            })))
    }

    /// The rows of a file list in path or folder view: `(file index, depth)`
    /// for files, plus folder rows in folder view. `key` scopes collapsed folders.
    fn file_rows(&self, key: &str, files: &[FileChange]) -> Vec<TreeRow> {
        if !self.file_tree {
            return files.iter().enumerate().map(|(index, f)| TreeRow::File { index, name: f.path.clone(), depth: 0 }).collect();
        }
        let prefix = format!("{key}:");
        let collapsed = self.collapsed_dirs.iter().filter_map(|d| d.strip_prefix(&prefix)).map(str::to_string).collect();
        let paths: Vec<&str> = files.iter().map(|f| f.path.as_str()).collect();
        file_tree::build(&paths, &collapsed)
    }

    fn folder_row(&self, id: ElementId, key: &'static str, path: &str, name: &str, depth: usize, collapsed: bool, cx: &mut Context<Self>) -> Stateful<Div> {
        let toggle = format!("{key}:{path}");
        div()
            .id(id)
            .pl(px(12. + depth as f32 * 14.))
            .pr_3()
            .py_0p5()
            .flex()
            .items_center()
            .gap_1()
            .text_sm()
            .text_color(c(theme::MUTED))
            .cursor_pointer()
            .hover(|s| s.bg(c(theme::HOVER)))
            .child(div().w(px(16.)).flex_none().text_xs().child(if collapsed { "▸" } else { "▾" }))
            .child(div().flex_1().min_w_0().truncate().child(format!("{name}/")))
            .on_click(cx.listener(move |this, _, _, cx| {
                if !this.collapsed_dirs.remove(&toggle) {
                    this.collapsed_dirs.insert(toggle.clone());
                }
                cx.notify();
            }))
    }

    fn folders_toggle(&self, id: &'static str, cx: &mut Context<Self>) -> Button {
        Button::new(id).label(if self.file_tree { "✓ Folders" } else { "Folders" }).xsmall().ghost().on_click(cx.listener(|this, _, _, cx| {
            this.file_tree = !this.file_tree;
            cx.notify();
        }))
    }

    fn file_list(&self, id: &'static str, files: &[FileChange], staged: bool, cx: &mut Context<Self>) -> impl IntoElement {
        let ws = cx.entity().downgrade();
        let picked = self.picked_files(files, staged);
        let rows = self.file_rows(id, files);
        // Shift-click ranges follow what's on screen.
        let visible: std::rc::Rc<[FileChange]> = file_tree::visible_files(&rows).into_iter().map(|i| files[i].clone()).collect();
        div().id(id).flex_1().min_h(px(60.)).overflow_y_scroll().children(rows.into_iter().enumerate().map(|(row, tree_row)| {
            let (index, name, depth) = match tree_row {
                TreeRow::File { index, name, depth } => (index, name, depth),
                TreeRow::Dir { path, name, depth, files: under, collapsed } => {
                    let under: Vec<FileChange> = under.into_iter().map(|i| files[i].clone()).collect();
                    return self
                        .folder_row((id, row).into(), id, &path, &name, depth, collapsed, cx)
                        .child(
                            Button::new((if staged { "unstage-dir" } else { "stage-dir" }, row))
                                .label(if staged { "Unstage" } else { "Stage" })
                                .xsmall()
                                .ghost()
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    cx.stop_propagation();
                                    this.stage_files(&under, staged, cx);
                                })),
                        )
                        .into_any_element();
                }
            };
            let f = &files[index];
            let change = f.clone();
            let for_menu = f.clone();
            let ws = ws.clone();
            let pos = visible.iter().position(|v| v.path == f.path).unwrap_or_default();
            let visible = visible.clone();
            let open = self.diff.as_ref().is_some_and(|d| {
                matches!(&d.source, DiffSource::Workdir { change, staged: s } if change.path == f.path && *s == staged)
            });
            let in_multi = picked.len() > 1 && picked.iter().any(|p| p.path == f.path);
            // Right-clicking one of several picked files acts on all of them.
            let menu_files = if in_multi { picked.clone() } else { Vec::new() };
            div()
                .id((id, row))
                .pl(px(12. + depth as f32 * 14.))
                .pr_3()
                .py_0p5()
                .flex()
                .items_center()
                .gap_1()
                .text_sm()
                .cursor_pointer()
                .when(open || in_multi, |d| d.bg(c(theme::SELECTED)))
                .when(!(open || in_multi), |d| d.hover(|s| s.bg(c(theme::HOVER))))
                .child(status_letter(f))
                .child(div().flex_1().min_w_0().truncate().child(name))
                .child(
                    Button::new((if staged { "unstage" } else { "stage" }, index))
                        .label(if staged { "Unstage" } else { "Stage" })
                        .xsmall()
                        .ghost()
                        .on_click(cx.listener(move |this, _, _, cx| {
                            cx.stop_propagation();
                            this.stage_files(std::slice::from_ref(&change), staged, cx);
                        })),
                )
                .on_click(cx.listener(move |this, ev: &ClickEvent, _, cx| {
                    if !this.click_file(&visible, pos, staged, ev.modifiers(), cx) {
                        return;
                    }
                    let file = visible[pos].clone();
                    if file.status == FileStatus::Conflicted {
                        this.open_conflict(file.path, cx);
                    } else {
                        this.show_workdir_diff(file, staged, cx);
                    }
                }))
                .context_menu(move |menu, _, _| {
                    if menu_files.is_empty() {
                        menus::file_menu(menu, &ws, &for_menu, staged)
                    } else {
                        menus::files_menu(menu, &ws, &menu_files, staged)
                    }
                })
                .into_any_element()
        }))
    }

    fn render_staging(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let st = &self.status;
        let unstaged: Vec<FileChange> = st.conflicted.iter().chain(&st.unstaged).cloned().collect();
        let can_commit = !st.staged.is_empty() || self.amend;
        let picked_unstaged = self.picked_files(&unstaged, false);
        let picked_staged = self.picked_files(&st.staged, true);
        let pick_button = |id: &'static str, picked: Vec<FileChange>, staged: bool, cx: &mut Context<Self>| {
            (picked.len() > 1).then(|| {
                Button::new(id)
                    .label(format!("{} {}", if staged { "Unstage" } else { "Stage" }, picked.len()))
                    .xsmall()
                    .primary()
                    .on_click(cx.listener(move |this, _, _, cx| this.stage_files(&picked, staged, cx)))
            })
        };
        div()
            .flex()
            .flex_col()
            .size_full()
            .child(
                div()
                    .flex()
                    .items_center()
                    .pr_2()
                    .child(section_header("Unstaged files", unstaged.len()))
                    .child(self.folders_toggle("folders-staging", cx))
                    .children(pick_button("stage-picked", picked_unstaged, false, cx))
                    .child(Button::new("stage-all").label("Stage all").xsmall().ghost().disabled(unstaged.is_empty()).on_click(
                        cx.listener(|this, _, _, cx| this.run("Staging", Change::Status, cx, |r| r.stage_all())),
                    )),
            )
            .child(self.file_list("unstaged", &unstaged, false, cx))
            .child(
                div()
                    .flex()
                    .items_center()
                    .pr_2()
                    .border_t_1()
                    .border_color(c(theme::BORDER))
                    .child(section_header("Staged files", st.staged.len()))
                    .children(pick_button("unstage-picked", picked_staged, true, cx))
                    .child(Button::new("unstage-all").label("Unstage all").xsmall().ghost().disabled(st.staged.is_empty()).on_click(
                        cx.listener(|this, _, _, cx| this.run("Unstaging", Change::Status, cx, |r| r.unstage_all())),
                    )),
            )
            .child(self.file_list("staged", &st.staged, true, cx))
            .child(
                div()
                    .flex_none()
                    .p_3()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .border_t_1()
                    .border_color(c(theme::BORDER))
                    .child(Input::new(&self.summary))
                    .child(Textarea::new(&self.description).h(px(90.)))
                    .when_some(self.ai_usage.clone(), |d, usage| {
                        d.child(div().text_xs().text_color(c(theme::MUTED)).child(format!("Last AI request: {usage}")))
                    })
                    .child(
                        div()
                            .flex()
                            .gap_2()
                            .child(
                                Button::new("ai-commit")
                                    .label(self.ai_busy.clone().unwrap_or_else(|| "✨ Write message".into()))
                                    .small()
                                    .ghost()
                                    .disabled(self.ai_busy.is_some() || st.staged.is_empty())
                                    .on_click(cx.listener(|this, _, window, cx| this.generate_commit_message(window, cx))),
                            )
                            .child(
                                Button::new("amend")
                                    .label(if self.amend { "✓ Amend" } else { "Amend" })
                                    .small()
                                    .ghost()
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.amend = !this.amend;
                                        cx.notify();
                                    })),
                            )
                            .child(
                                Button::new("commit")
                                    .label(if st.staged.is_empty() {
                                        "Commit".to_string()
                                    } else {
                                        format!("Commit {} file(s)", st.staged.len())
                                    })
                                    .small()
                                    .primary()
                                    .flex_1()
                                    .disabled(!can_commit || self.busy.is_some())
                                    .on_click(cx.listener(|this, _, window, cx| this.commit(window, cx))),
                            ),
                    ),
            )
    }

    fn render_prompt(&self, cx: &mut Context<Self>) -> Option<impl IntoElement> {
        let prompt = self.prompt.as_ref()?;
        Some(
            div()
                .id("prompt-backdrop")
                .absolute()
                .inset_0()
                .flex()
                .items_center()
                .justify_center()
                .bg(gpui_kit::black().opacity(0.5))
                .on_click(cx.listener(|this, _, _, cx| this.cancel_prompt(cx)))
                .child(
                    div()
                        .id("prompt")
                        .w(px(420.))
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
                        .child(div().text_base().font_weight(FontWeight::BOLD).child(prompt.title.clone()))
                        .when_some(prompt.message.clone(), |d, m| d.child(div().text_sm().text_color(c(theme::MUTED)).child(m)))
                        .when_some(prompt.input.as_ref(), |d, input| d.child(Input::new(input)))
                        .child(
                            div()
                                .flex()
                                .justify_end()
                                .gap_2()
                                .when(prompt.suggest.is_some() && self.ai.is_some(), |d| {
                                    d.child(
                                        Button::new("prompt-suggest")
                                            .label(self.ai_busy.clone().unwrap_or_else(|| "✨ Suggest".into()))
                                            .small()
                                            .ghost()
                                            .disabled(self.ai_busy.is_some())
                                            .on_click(cx.listener(|this, _, window, cx| this.suggest_for_prompt(window, cx))),
                                    )
                                    .child(div().flex_1())
                                })
                                .child(Button::new("prompt-cancel").label("Cancel").small().ghost().on_click(cx.listener(
                                    |this, _, _, cx| this.cancel_prompt(cx),
                                )))
                                .child(
                                    Button::new("prompt-ok")
                                        .label(prompt.confirm.clone())
                                        .small()
                                        .when(prompt.danger, |b| b.danger())
                                        .when(!prompt.danger, |b| b.primary())
                                        .on_click(cx.listener(|this, _, window, cx| this.submit_prompt(window, cx))),
                                ),
                        ),
                ),
        )
    }

    fn render_multi(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let commits = self.multi_commits();
        let n = commits.len();
        let ids: Vec<String> = commits.iter().map(|c| c.id.clone()).collect();
        let shas = commits.iter().map(|c| c.short_id.clone()).collect::<Vec<_>>().join(" ");
        let has_merge = commits.iter().any(|c| c.parents.len() > 1);
        let oldest_parent = commits.first().and_then(|c| c.parents.first().cloned());
        let (pick, revert) = (ids.clone(), ids.clone());
        div()
            .size_full()
            .flex()
            .flex_col()
            .child(
                div()
                    .p_3()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .border_b_1()
                    .border_color(c(theme::BORDER))
                    .child(div().text_base().font_weight(FontWeight::BOLD).child(format!("{n} commits selected")))
                    .child(div().text_xs().text_color(c(theme::FAINT)).child("⌘-click to add or remove, Shift-click for a range"))
                    .child(
                        div()
                            .flex()
                            .flex_wrap()
                            .gap_1()
                            .child(Button::new("multi-pick").label(format!("Cherry-pick {n}")).small().outline().disabled(has_merge).on_click(cx.listener(
                                move |this, _, _, cx| {
                                    let ids = pick.clone();
                                    this.run("Cherry-picking", Change::Refs, cx, move |r| ids.iter().try_for_each(|id| r.cherry_pick(id)));
                                },
                            )))
                            .child(Button::new("multi-revert").label(format!("Revert {n}")).small().outline().disabled(has_merge).on_click(cx.listener(
                                move |this, _, _, cx| {
                                    let ids = revert.clone();
                                    // Newest first, so each revert applies cleanly on top of the last.
                                    this.run("Reverting", Change::Refs, cx, move |r| ids.iter().rev().try_for_each(|id| r.revert(id)));
                                },
                            )))
                            .when_some(oldest_parent, |d, base| {
                                d.child(Button::new("multi-rebase").label("Interactive rebase…").small().outline().on_click(cx.listener(
                                    move |this, _, window, cx| this.open_rebase_editor(base.clone(), window, cx),
                                )))
                            })
                            .child(Button::new("multi-copy").label("Copy SHAs").small().ghost().on_click(cx.listener(move |_, _, _, cx| {
                                cx.write_to_clipboard(ClipboardItem::new_string(shas.clone()));
                            }))),
                    ),
            )
            .child(div().id("multi-list").flex_1().overflow_y_scroll().children(commits.iter().rev().enumerate().map(|(i, commit)| {
                div()
                    .id(("multi", i))
                    .px_3()
                    .py_1()
                    .flex()
                    .gap_2()
                    .text_sm()
                    .border_b_1()
                    .border_color(c(theme::BORDER))
                    .child(div().flex_none().text_xs().text_color(c(theme::FAINT)).child(commit.short_id.clone()))
                    .child(div().flex_1().truncate().child(commit.summary.clone()))
            })))
    }

    fn render_terminal_panel(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let active = self.terminals.get(self.terminal_index).cloned();
        div()
            .flex_none()
            .h(px(280.))
            .flex()
            .flex_col()
            .border_t_1()
            .border_color(c(theme::BORDER))
            .child(
                div()
                    .h(px(30.))
                    .flex_none()
                    .flex()
                    .items_center()
                    .gap_1()
                    .px_2()
                    .bg(c(theme::PANEL))
                    .border_b_1()
                    .border_color(c(theme::BORDER))
                    .children(self.terminals.iter().enumerate().map(|(i, term)| {
                        let selected = i == self.terminal_index;
                        let exited = term.read(cx).exited;
                        div()
                            .id(("term-tab", i))
                            .flex()
                            .items_center()
                            .gap_2()
                            .px_2()
                            .h(px(22.))
                            .rounded_md()
                            .text_xs()
                            .cursor_pointer()
                            .when(selected, |d| d.bg(c(theme::SELECTED)))
                            .when(!selected, |d| d.text_color(c(theme::MUTED)).hover(|d| d.bg(c(theme::HOVER))))
                            .child(format!("Terminal {}{}", i + 1, if exited { " (exited)" } else { "" }))
                            .child(
                                div()
                                    .id(("term-close", i))
                                    .text_color(c(theme::FAINT))
                                    .hover(|d| d.text_color(c(theme::TEXT)))
                                    .child("×")
                                    .on_click(cx.listener(move |this, _, window, cx| {
                                        cx.stop_propagation();
                                        this.close_terminal(i, window, cx);
                                    })),
                            )
                            .on_click(cx.listener(move |this, _, window, cx| this.select_terminal(i, window, cx)))
                    }))
                    .child(
                        Button::new("term-new")
                            .label("+")
                            .xsmall()
                            .ghost()
                            .on_click(cx.listener(|this, _, window, cx| this.new_terminal(window, cx))),
                    )
                    .child(div().flex_1())
                    .child(
                        Button::new("term-hide")
                            .label("Hide")
                            .xsmall()
                            .ghost()
                            .on_click(cx.listener(|this, _, window, cx| this.toggle_terminal(window, cx))),
                    ),
            )
            .child(div().flex_1().min_h_0().children(active))
    }

    fn render_welcome(&self, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .gap_4()
            .bg(c(theme::BG))
            .child(div().text_3xl().font_weight(FontWeight::BOLD).child("GitChrono"))
            .child(div().text_color(c(theme::MUTED)).child(concat!("A fast, native Git client · v", env!("CARGO_PKG_VERSION"))))
            .child(
                Button::new("welcome-open")
                    .label("Open Repository…")
                    .primary()
                    .on_click(cx.listener(|this, _, _, cx| this.prompt_open(cx))),
            )
            .when_some(self.error.clone(), |d, e| d.child(div().text_sm().text_color(c(theme::DEL)).child(e)))
    }
}

impl Render for Workspace {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let root = div().relative().size_full().flex().flex_col().bg(c(theme::BG)).text_color(c(theme::TEXT));
        if self.repo.is_none() {
            return root.child(self.render_welcome(cx));
        }

        let center = match (&self.conflict, &self.blame, &self.diff) {
            (Some(conflict), _, _) => self.render_conflict(conflict, cx).into_any_element(),
            (None, Some(blame), _) => self.render_blame(blame, cx).into_any_element(),
            (None, None, Some(view)) => self.render_diff(view, cx).into_any_element(),
            (None, None, None) => self.render_graph(cx).into_any_element(),
        };
        let right = if let Some(history) = &self.file_history {
            Some(self.render_file_history(history, cx).into_any_element())
        } else if self.multi.len() > 1 {
            Some(self.render_multi(cx).into_any_element())
        } else {
            match self.selection {
            Selection::Wip => Some(self.render_staging(cx).into_any_element()),
                Selection::Commit(_) => self.selected_commit().map(|commit| self.render_detail(commit, cx).into_any_element()),
                Selection::None => None,
            }
        };

        root.on_action(cx.listener(|this, _: &crate::workspace::CloseSearch, _, cx| this.close_search(cx)))
            .child(self.render_toolbar(cx))
            .child(self.render_banners(cx))
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .child(self.render_sidebar(cx))
                    .child(center)
                    .when_some(right, |d, right| {
                        d.child(
                            div()
                                .w(px(380.))
                                .flex_none()
                                .h_full()
                                .bg(c(theme::PANEL))
                                .border_l_1()
                                .border_color(c(theme::BORDER))
                                .child(right),
                        )
                    }),
            )
            .when(self.terminal_open, |d| d.child(self.render_terminal_panel(cx)))
            .children(self.render_prompt(cx))
            .children(self.render_ai_settings(cx))
            .children(self.render_rebase_editor(cx))
            .children(self.render_drop_menu(cx))
    }
}
