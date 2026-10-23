//! The main repository window: toolbar, ref sidebar, commit graph, and the
//! right-hand commit/staging panel.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::Result;
use gitchrono_core::{
    Commit, FileChange, FileDiff, Operation, RefKind, RefLabel, Repo, ResetMode, Snapshot,
};
use gitchrono_graph::{Graph, Row};
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Input, InputState, Textarea, TextareaState};
use gpui_kit::component::{Disableable as _, Sizable as _};
use gpui_kit::prelude::*;
use gpui_kit::*;

use crate::graph_paint::{self, Node, ROW_HEIGHT};
use crate::theme::{self, c};

const LOG_LIMIT: usize = 50_000;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Selection {
    None,
    Wip,
    Commit(usize),
}

struct DiffView {
    title: SharedString,
    diff: FileDiff,
}

pub struct Workspace {
    repo: Option<Arc<Repo>>,
    snap: Arc<Snapshot>,
    graph: Arc<Graph>,
    selection: Selection,
    commit_files: Vec<FileChange>,
    diff: Option<DiffView>,
    busy: Option<SharedString>,
    error: Option<SharedString>,
    summary: Entity<InputState>,
    description: Entity<TextareaState>,
    amend: bool,
    list_scroll: UniformListScrollHandle,
    /// `GITCHRONO_SELECT=<row>[:diff]`, applied once after the first load (for headless screenshots).
    debug_select: Option<(usize, bool)>,
    _task: Option<Task<()>>,
}

impl Workspace {
    pub fn new(path: Option<PathBuf>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let summary = cx.new(|cx| InputState::new(window, cx).placeholder("Commit summary"));
        let description = cx.new(|cx| TextareaState::new(window, cx).placeholder("Description"));
        let mut this = Self {
            repo: None,
            snap: Arc::default(),
            graph: Arc::default(),
            selection: Selection::None,
            commit_files: Vec::new(),
            diff: None,
            busy: None,
            error: None,
            summary,
            description,
            amend: false,
            list_scroll: UniformListScrollHandle::new(),
            debug_select: std::env::var("GITCHRONO_SELECT").ok().and_then(|v| {
                let (row, diff) = v.split_once(':').map_or((v.as_str(), false), |(r, _)| (r, true));
                Some((row.parse().ok()?, diff))
            }),
            _task: None,
        };
        if let Some(path) = path {
            this.open(path, cx);
        }
        this
    }

    fn open(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        match Repo::open(&path) {
            Ok(repo) => {
                self.repo = Some(Arc::new(repo));
                self.selection = Selection::None;
                self.diff = None;
                self.reload(cx);
            }
            Err(e) => self.error = Some(format!("{e:#}").into()),
        }
        cx.notify();
    }

    fn prompt_open(&mut self, cx: &mut Context<Self>) {
        let paths = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some("Open Repository".into()),
        });
        cx.spawn(async move |this, cx| {
            if let Ok(Ok(Some(mut paths))) = paths.await {
                if let Some(path) = paths.pop() {
                    this.update(cx, |this, cx| this.open(path, cx)).ok();
                }
            }
        })
        .detach();
    }

    fn has_wip(&self) -> bool {
        !self.snap.status.is_clean()
    }

    fn row_count(&self) -> usize {
        self.snap.commits.len() + usize::from(self.has_wip())
    }

    /// Reload refs/history/status off the main thread, keeping the selection.
    fn reload(&mut self, cx: &mut Context<Self>) {
        let Some(repo) = self.repo.clone() else { return };
        let selected_id = match self.selection {
            Selection::Commit(i) => self.snap.commits.get(i).map(|c| c.id.clone()),
            _ => None,
        };
        self.busy.get_or_insert_with(|| "Loading…".into());
        cx.notify();
        let task = cx.background_spawn(async move {
            let snap = repo.snapshot(LOG_LIMIT)?;
            let graph = gitchrono_graph::layout(
                snap.commits.iter().map(|c| (c.id.as_str(), c.parents.iter().map(String::as_str))),
            );
            anyhow::Ok((snap, graph))
        });
        self._task = Some(cx.spawn(async move |this, cx| {
            let result = task.await;
            this.update(cx, |this, cx| {
                this.busy = None;
                match result {
                    Ok((snap, graph)) => {
                        this.snap = Arc::new(snap);
                        this.graph = Arc::new(graph);
                        this.selection = match selected_id {
                            Some(id) => this
                                .snap
                                .commits
                                .iter()
                                .position(|c| c.id == id)
                                .map_or(Selection::None, Selection::Commit),
                            None if this.selection == Selection::Wip && !this.has_wip() => Selection::None,
                            None => this.selection,
                        };
                        if let Selection::Commit(i) = this.selection {
                            this.load_commit_files(i, cx);
                        }
                        if let Some((row, diff)) = this.debug_select.take() {
                            this.select_row(row, cx);
                            if diff {
                                this.debug_open_first_diff(cx);
                            }
                        }
                    }
                    Err(e) => this.error = Some(format!("{e:#}").into()),
                }
                cx.notify();
            })
            .ok();
        }));
    }

    /// Run a git write operation in the background, then reload.
    fn run(
        &mut self,
        label: &str,
        cx: &mut Context<Self>,
        op: impl FnOnce(&Repo) -> Result<()> + Send + 'static,
    ) {
        let Some(repo) = self.repo.clone() else { return };
        if self.busy.is_some() {
            return;
        }
        self.busy = Some(format!("{label}…").into());
        self.error = None;
        cx.notify();
        let task = cx.background_spawn(async move { op(&repo) });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            this.update(cx, |this, cx| {
                this.busy = None;
                if let Err(e) = result {
                    this.error = Some(format!("{e:#}").into());
                }
                this.reload(cx);
            })
            .ok();
        })
        .detach();
    }

    fn select_row(&mut self, row: usize, cx: &mut Context<Self>) {
        let wip = self.has_wip();
        self.selection = if wip && row == 0 {
            Selection::Wip
        } else {
            Selection::Commit(row - usize::from(wip))
        };
        self.diff = None;
        if let Selection::Commit(i) = self.selection {
            self.load_commit_files(i, cx);
        }
        cx.notify();
    }

    fn select_commit_id(&mut self, id: &str, cx: &mut Context<Self>) {
        if let Some(i) = self.snap.commits.iter().position(|c| c.id == id) {
            let row = i + usize::from(self.has_wip());
            self.select_row(row, cx);
            self.list_scroll.scroll_to_item(row, ScrollStrategy::Center);
        }
    }

    fn load_commit_files(&mut self, index: usize, cx: &mut Context<Self>) {
        let (Some(repo), Some(commit)) = (self.repo.clone(), self.snap.commits.get(index)) else {
            return;
        };
        let id = commit.id.clone();
        let task = cx.background_spawn(async move { repo.commit_files(&id) });
        cx.spawn(async move |this, cx| {
            let files = task.await;
            this.update(cx, |this, cx| {
                if this.selection == Selection::Commit(index) {
                    this.commit_files = files.unwrap_or_default();
                    cx.notify();
                }
            })
            .ok();
        })
        .detach();
    }

    fn show_diff(&mut self, title: String, cx: &mut Context<Self>, load: impl FnOnce(&Repo) -> Result<FileDiff> + Send + 'static) {
        let Some(repo) = self.repo.clone() else { return };
        let task = cx.background_spawn(async move { load(&repo) });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            this.update(cx, |this, cx| {
                match result {
                    Ok(diff) => this.diff = Some(DiffView { title: title.into(), diff }),
                    Err(e) => this.error = Some(format!("{e:#}").into()),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn commit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let summary = self.summary.read(cx).value().trim().to_string();
        let body = self.description.read(cx).value().trim().to_string();
        let message = if body.is_empty() { summary } else { format!("{summary}\n\n{body}") };
        let amend = self.amend;
        self.summary.update(cx, |s, cx| s.set_value("", window, cx));
        self.description.update(cx, |s, cx| s.set_value("", window, cx));
        self.amend = false;
        self.run("Committing", cx, move |repo| repo.commit(&message, amend));
    }

    fn debug_open_first_diff(&mut self, cx: &mut Context<Self>) {
        match self.selection {
            Selection::Wip => {
                let st = &self.snap.status;
                let (change, staged) = match st.unstaged.first() {
                    Some(c) => (c.clone(), false),
                    None => match st.staged.first() {
                        Some(c) => (c.clone(), true),
                        None => return,
                    },
                };
                self.show_diff(change.path.clone(), cx, move |r| r.workdir_file_diff(&change, staged));
            }
            Selection::Commit(i) => {
                let Some(id) = self.snap.commits.get(i).map(|c| c.id.clone()) else { return };
                self.show_diff(format!("{}", &id[..7]), cx, move |r| {
                    let path = r.commit_files(&id)?.first().map(|f| f.path.clone()).unwrap_or_default();
                    r.commit_file_diff(&id, &path)
                });
            }
            Selection::None => {}
        }
    }

    fn selected_commit(&self) -> Option<&Commit> {
        match self.selection {
            Selection::Commit(i) => self.snap.commits.get(i),
            _ => None,
        }
    }
}

// ---------------------------------------------------------------- rendering

fn relative_time(secs: i64) -> String {
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
        let st = &self.snap.status;
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
                    .child(div().text_xs().text_color(c(theme::MUTED)).child(
                        self.repo.as_ref().map(|r| r.name()).unwrap_or_else(|| "GitChrono".into()),
                    ))
                    .child(div().text_sm().font_weight(FontWeight::BOLD).child(branch)),
            )
            .child(Button::new("open").label("Open").ghost().small().on_click(cx.listener(|this, _, _, cx| this.prompt_open(cx))))
            .when(self.repo.is_some(), |bar| {
                bar.child(tool("fetch", "Fetch".into()).on_click(cx.listener(|this, _, _, cx| {
                    this.run("Fetching", cx, |r| r.fetch());
                })))
                .child(
                    tool("pull", if st.behind > 0 { format!("Pull ↓{}", st.behind) } else { "Pull".into() })
                        .on_click(cx.listener(|this, _, _, cx| this.run("Pulling", cx, |r| r.pull(false)))),
                )
                .child(
                    tool("push", if st.ahead > 0 { format!("Push ↑{}", st.ahead) } else { "Push".into() })
                        .on_click(cx.listener(|this, _, _, cx| this.run("Pushing", cx, |r| r.push(false)))),
                )
                .child(tool("stash", "Stash".into()).disabled(busy || st.is_clean()).on_click(cx.listener(|this, _, _, cx| {
                    this.run("Stashing", cx, |r| r.stash_push(None));
                })))
                .child(tool("pop", "Pop".into()).disabled(busy || self.snap.refs.stashes.is_empty()).on_click(cx.listener(
                    |this, _, _, cx| this.run("Popping stash", cx, |r| r.stash_pop(0)),
                )))
                .child(tool("refresh", "Refresh".into()).on_click(cx.listener(|this, _, _, cx| this.reload(cx))))
            })
            .child(div().flex_1())
            .when_some(self.busy.clone(), |bar, msg| bar.child(div().text_xs().text_color(c(theme::MUTED)).child(msg)))
    }

    fn render_banners(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let op = self.snap.status.operation;
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
                let name = match op {
                    Operation::Merge => "Merge",
                    Operation::Rebase => "Rebase",
                    Operation::CherryPick => "Cherry-pick",
                    Operation::Revert => "Revert",
                    Operation::None => "",
                };
                let conflicts = self.snap.status.conflicted.len();
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
                        .child(div().flex_1().child(format!("{name} in progress — {conflicts} conflicted file(s)")))
                        .child(Button::new("op-continue").label("Continue").xsmall().primary().disabled(conflicts > 0).on_click(
                            cx.listener(move |this, _, _, cx| this.run("Continuing", cx, move |r| r.resolve_operation(op, false))),
                        ))
                        .child(Button::new("op-abort").label("Abort").xsmall().ghost().on_click(
                            cx.listener(move |this, _, _, cx| this.run("Aborting", cx, move |r| r.resolve_operation(op, true))),
                        )),
                )
            })
    }

    fn render_sidebar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let refs = &self.snap.refs;
        let item = |id: SharedString, label: String, target: String, current: bool, kind: RefKind, cx: &mut Context<Self>| {
            let checkout_target = label.clone();
            div()
                .id(id)
                .px_3()
                .py_0p5()
                .flex()
                .gap_1p5()
                .text_sm()
                .text_color(c(if current { theme::TEXT } else { theme::MUTED }))
                .when(current, |d| d.font_weight(FontWeight::BOLD))
                .hover(|s| s.bg(c(theme::HOVER)))
                .cursor_pointer()
                .child(div().w(px(12.)).text_color(c(theme::ADD)).child(if current { "✓" } else { "" }))
                .child(div().truncate().child(label))
                .on_click(cx.listener(move |this, ev: &ClickEvent, _, cx| {
                    if ev.click_count() >= 2 {
                        let t = checkout_target.clone();
                        match kind {
                            RefKind::Local => this.run("Checking out", cx, move |r| r.checkout(&t)),
                            RefKind::Remote => this.run("Checking out", cx, move |r| r.checkout_remote(&t)),
                            _ => {}
                        }
                    } else {
                        this.select_commit_id(&target, cx);
                    }
                }))
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

        col = col.child(section_header("Local", refs.local.len()));
        for b in &refs.local {
            let label = match (b.ahead, b.behind) {
                (0, 0) => b.name.clone(),
                (a, 0) => format!("{}  ↑{a}", b.name),
                (0, bh) => format!("{}  ↓{bh}", b.name),
                (a, bh) => format!("{}  ↑{a} ↓{bh}", b.name),
            };
            let el = item(format!("l-{}", b.name).into(), b.name.clone(), b.target.clone(), b.current, RefKind::Local, cx);
            col = col.child(el.child(div().text_xs().text_color(c(theme::FAINT)).child(label[b.name.len()..].to_string())));
        }
        col = col.child(section_header("Remote", refs.remote.len()));
        for b in &refs.remote {
            col = col.child(item(format!("r-{}", b.name).into(), b.name.clone(), b.target.clone(), false, RefKind::Remote, cx));
        }
        col = col.child(section_header("Tags", refs.tags.len()));
        for t in refs.tags.iter().take(200) {
            col = col.child(item(format!("t-{}", t.name).into(), t.name.clone(), t.target.clone(), false, RefKind::Tag, cx));
        }
        col = col.child(section_header("Stashes", refs.stashes.len()));
        for s in &refs.stashes {
            col = col.child(item(format!("s-{}", s.index).into(), s.message.clone(), s.target.clone(), false, RefKind::Stash, cx));
        }
        col
    }

    fn render_graph(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let gutter = graph_paint::gutter_width(self.graph.max_lane);
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
            .child(div().w(px(200.)).px_3().child("BRANCH / TAG"))
            .child(div().w(gutter).px_2().child("GRAPH"))
            .child(div().flex_1().px_2().child("COMMIT MESSAGE"))
            .child(div().w(px(150.)).px_2().child("AUTHOR"))
            .child(div().w(px(110.)).px_2().child("DATE"));

        let list = uniform_list(
            "commits",
            self.row_count(),
            cx.processor(move |this, range: std::ops::Range<usize>, _, cx| {
                range.map(|row| this.render_row(row, gutter, cx)).collect::<Vec<_>>()
            }),
        )
        .track_scroll(&self.list_scroll)
        .flex_1();

        div().flex_1().min_w_0().h_full().flex().flex_col().bg(c(theme::BG)).child(header).child(list)
    }

    fn render_row(&self, row: usize, gutter: Pixels, cx: &mut Context<Self>) -> AnyElement {
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
            .on_click(cx.listener(move |this, _, _, cx| this.select_row(row, cx)));

        if wip && row == 0 {
            let selected = self.selection == Selection::Wip;
            let head_lane = self.graph.rows.first().map_or(0, |r| r.lane);
            let n = self.snap.status.change_count();
            return base
                .when(selected, |d| d.bg(c(theme::SELECTED)))
                .child(div().w(px(200.)).flex_none())
                .child(
                    div()
                        .w(gutter)
                        .h_full()
                        .flex_none()
                        .child(graph_paint::row_canvas(Row { lane: head_lane, ..Default::default() }, Node::Wip)),
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
        let selected = self.selection == Selection::Commit(index);
        let node = Node::Commit { merge: commit.parents.len() > 1 };

        base.when(selected, |d| d.bg(c(theme::SELECTED)))
            .child(
                div()
                    .w(px(200.))
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
            .child(div().w(gutter).h_full().flex_none().child(graph_paint::row_canvas(layout, node)))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .px_2()
                    .truncate()
                    .text_color(c(if node_is_merge(commit) { theme::MUTED } else { theme::TEXT }))
                    .child(commit.summary.clone()),
            )
            .child(div().w(px(150.)).flex_none().px_2().truncate().text_color(c(theme::MUTED)).child(commit.author_name.clone()))
            .child(div().w(px(110.)).flex_none().px_2().text_xs().text_color(c(theme::FAINT)).child(relative_time(commit.time)))
            .into_any_element()
    }

    fn render_detail(&self, commit: &Commit, cx: &mut Context<Self>) -> impl IntoElement {
        let id = commit.id.clone();
        let short = commit.short_id.clone();
        let action = |label: &'static str| Button::new(label).label(label).xsmall().outline();

        div()
            .flex()
            .flex_col()
            .size_full()
            .child(
                div()
                    .p_3()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .border_b_1()
                    .border_color(c(theme::BORDER))
                    .child(div().text_xs().text_color(c(theme::FAINT)).child(format!("commit {}", commit.short_id)))
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
                    .when(!commit.parents.is_empty(), |d| {
                        d.child(div().text_xs().text_color(c(theme::FAINT)).child(format!(
                            "parent{} {}",
                            if commit.parents.len() > 1 { "s" } else { "" },
                            commit.parents.iter().map(|p| &p[..7]).collect::<Vec<_>>().join(" ")
                        )))
                    })
                    .child(
                        div()
                            .flex()
                            .flex_wrap()
                            .gap_1()
                            .child(action("Checkout").on_click(cx.listener({
                                let id = id.clone();
                                move |this, _, _, cx| {
                                    let id = id.clone();
                                    this.run("Checking out", cx, move |r| r.checkout(&id));
                                }
                            })))
                            .child(action("Branch here").on_click(cx.listener({
                                let id = id.clone();
                                move |this, _, _, cx| {
                                    let id = id.clone();
                                    let name = format!("branch-{}", &id[..7]);
                                    this.run("Creating branch", cx, move |r| r.create_branch(&name, Some(&id), true));
                                }
                            })))
                            .child(action("Cherry-pick").on_click(cx.listener({
                                let id = id.clone();
                                move |this, _, _, cx| {
                                    let id = id.clone();
                                    this.run("Cherry-picking", cx, move |r| r.cherry_pick(&id));
                                }
                            })))
                            .child(action("Revert").on_click(cx.listener({
                                let id = id.clone();
                                move |this, _, _, cx| {
                                    let id = id.clone();
                                    this.run("Reverting", cx, move |r| r.revert(&id));
                                }
                            })))
                            .child(action("Reset (mixed)").on_click(cx.listener({
                                let id = id.clone();
                                move |this, _, _, cx| {
                                    let id = id.clone();
                                    this.run("Resetting", cx, move |r| r.reset(&id, ResetMode::Mixed));
                                }
                            }))),
                    ),
            )
            .child(section_header("Changed files", self.commit_files.len()))
            .child(
                div().id("commit-files").flex_1().overflow_y_scroll().children(self.commit_files.iter().enumerate().map(|(i, f)| {
                    let path = f.path.clone();
                    let commit_id = id.clone();
                    let title = format!("{path} @ {short}");
                    div()
                        .id(("cf", i))
                        .px_3()
                        .py_0p5()
                        .flex()
                        .gap_1()
                        .text_sm()
                        .cursor_pointer()
                        .hover(|s| s.bg(c(theme::HOVER)))
                        .child(status_letter(f))
                        .child(div().truncate().child(f.path.clone()))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            let (path, commit_id) = (path.clone(), commit_id.clone());
                            this.show_diff(title.clone(), cx, move |r| r.commit_file_diff(&commit_id, &path));
                        }))
                })),
            )
    }

    fn file_list(&self, id: &'static str, files: &[FileChange], staged: bool, cx: &mut Context<Self>) -> impl IntoElement {
        div().id(id).flex_1().min_h(px(60.)).overflow_y_scroll().children(files.iter().enumerate().map(|(i, f)| {
            let change = f.clone();
            let for_diff = f.clone();
            let label = if staged { "Unstage" } else { "Stage" };
            div()
                .id((id, i))
                .group("file")
                .px_3()
                .py_0p5()
                .flex()
                .items_center()
                .gap_1()
                .text_sm()
                .cursor_pointer()
                .hover(|s| s.bg(c(theme::HOVER)))
                .child(status_letter(f))
                .child(div().flex_1().min_w_0().truncate().child(f.path.clone()))
                .child(
                    Button::new(("stage", i)).label(label).xsmall().ghost().on_click(cx.listener(move |this, _, _, cx| {
                        let path = change.path.clone();
                        if staged {
                            this.run("Unstaging", cx, move |r| r.unstage(&[&path]));
                        } else {
                            this.run("Staging", cx, move |r| r.stage(&[&path]));
                        }
                    })),
                )
                .on_click(cx.listener(move |this, _, _, cx| {
                    let change = for_diff.clone();
                    let title = format!("{} ({})", change.path, if staged { "staged" } else { "unstaged" });
                    this.show_diff(title, cx, move |r| r.workdir_file_diff(&change, staged));
                }))
        }))
    }

    fn render_staging(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let st = &self.snap.status;
        let unstaged: Vec<FileChange> = st.conflicted.iter().chain(&st.unstaged).cloned().collect();
        let can_commit = !st.staged.is_empty() || self.amend;
        div()
            .flex()
            .flex_col()
            .size_full()
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .pr_2()
                    .child(section_header("Unstaged files", unstaged.len()))
                    .child(Button::new("stage-all").label("Stage all").xsmall().ghost().disabled(unstaged.is_empty()).on_click(
                        cx.listener(|this, _, _, cx| this.run("Staging", cx, |r| r.stage_all())),
                    )),
            )
            .child(self.file_list("unstaged", &unstaged, false, cx))
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .pr_2()
                    .border_t_1()
                    .border_color(c(theme::BORDER))
                    .child(section_header("Staged files", st.staged.len()))
                    .child(Button::new("unstage-all").label("Unstage all").xsmall().ghost().disabled(st.staged.is_empty()).on_click(
                        cx.listener(|this, _, _, cx| this.run("Unstaging", cx, |r| r.unstage_all())),
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
                    .child(
                        div()
                            .flex()
                            .gap_2()
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
                                    .label(if st.staged.is_empty() { "Commit".to_string() } else { format!("Commit {} file(s)", st.staged.len()) })
                                    .small()
                                    .primary()
                                    .flex_1()
                                    .disabled(!can_commit || self.busy.is_some())
                                    .on_click(cx.listener(|this, _, window, cx| this.commit(window, cx))),
                            ),
                    ),
            )
    }

    fn render_diff(&self, view: &DiffView, cx: &mut Context<Self>) -> impl IntoElement {
        let mut rows: Vec<AnyElement> = Vec::new();
        if view.diff.binary {
            rows.push(div().p_4().text_color(c(theme::MUTED)).child("Binary file").into_any_element());
        }
        for (h, hunk) in view.diff.hunks.iter().enumerate() {
            rows.push(
                div().px_3().py_1().bg(c(theme::PANEL_ALT)).text_color(c(theme::FAINT)).child(hunk.header.clone()).into_any_element(),
            );
            for (l, line) in hunk.lines.iter().enumerate() {
                use gitchrono_core::DiffLineKind::*;
                let (bg, sign) = match line.kind {
                    Add => (Some(theme::ADD_BG), "+"),
                    Del => (Some(theme::DEL_BG), "-"),
                    Context => (None, " "),
                };
                let num = |n: Option<u32>| div().w(px(44.)).flex_none().text_right().pr_2().text_color(c(theme::FAINT)).child(n.map(|n| n.to_string()).unwrap_or_default());
                rows.push(
                    div()
                        .id(("dl", h * 100_000 + l))
                        .flex()
                        .when_some(bg, |d, bg| d.bg(c(bg)))
                        .child(num(line.old_no))
                        .child(num(line.new_no))
                        .child(div().w(px(14.)).flex_none().child(sign))
                        .child(div().whitespace_nowrap().child(line.content.replace('\t', "    ")))
                        .into_any_element(),
                );
            }
        }

        div()
            .flex_1()
            .min_w_0()
            .h_full()
            .flex()
            .flex_col()
            .bg(c(theme::BG))
            .child(
                div()
                    .h(px(34.))
                    .flex_none()
                    .flex()
                    .items_center()
                    .gap_2()
                    .px_3()
                    .border_b_1()
                    .border_color(c(theme::BORDER))
                    .child(div().flex_1().truncate().text_sm().font_weight(FontWeight::BOLD).child(view.title.clone()))
                    .child(div().text_xs().text_color(c(theme::ADD)).child(format!("+{}", view.diff.additions)))
                    .child(div().text_xs().text_color(c(theme::DEL)).child(format!("-{}", view.diff.deletions)))
                    .child(Button::new("close-diff").label("Close").xsmall().ghost().on_click(cx.listener(|this, _, _, cx| {
                        this.diff = None;
                        cx.notify();
                    }))),
            )
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
            .child(div().text_color(c(theme::MUTED)).child("A fast, native Git client"))
            .child(Button::new("welcome-open").label("Open Repository…").primary().on_click(cx.listener(|this, _, _, cx| this.prompt_open(cx))))
            .when_some(self.error.clone(), |d, e| d.child(div().text_sm().text_color(c(theme::DEL)).child(e)))
    }
}

fn node_is_merge(commit: &Commit) -> bool {
    commit.parents.len() > 1
}

impl Render for Workspace {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let root = div().size_full().flex().flex_col().bg(c(theme::BG)).text_color(c(theme::TEXT));
        if self.repo.is_none() {
            return root.child(self.render_welcome(cx));
        }

        let center = match &self.diff {
            Some(view) => self.render_diff(view, cx).into_any_element(),
            None => self.render_graph(cx).into_any_element(),
        };
        let right = match self.selection {
            Selection::Wip => Some(self.render_staging(cx).into_any_element()),
            Selection::Commit(_) => self.selected_commit().map(|commit| self.render_detail(commit, cx).into_any_element()),
            Selection::None => None,
        };

        root.child(self.render_toolbar(cx)).child(self.render_banners(cx)).child(
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
    }
}
