//! Command palette (⌘P / ⌘⇧P): every action, open tabs, recent repositories
//! and branches, fuzzy-filtered.

use std::rc::Rc;

use gitchrono_core::{PullMode, Repo};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::prelude::*;
use gpui_kit::*;

use crate::fuzzy;
use crate::shell::Shell;
use crate::theme::{self, c};
use crate::watcher::Change;
use crate::workspace::Workspace;

actions!(palette, [PaletteUp, PaletteDown, PaletteClose]);

pub fn init(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("up", PaletteUp, Some("Palette > Input")),
        KeyBinding::new("down", PaletteDown, Some("Palette > Input")),
        KeyBinding::new("escape", PaletteClose, Some("Palette > Input")),
    ]);
}

const MAX_SHOWN: usize = 14;

type Run = Rc<dyn Fn(&mut Shell, &mut Window, &mut Context<Shell>)>;

#[derive(Clone)]
pub struct Item {
    pub label: SharedString,
    pub detail: SharedString,
    pub kind: &'static str,
    run: Run,
}

impl Item {
    fn new(kind: &'static str, label: impl Into<SharedString>, run: impl Fn(&mut Shell, &mut Window, &mut Context<Shell>) + 'static) -> Self {
        Self { label: label.into(), detail: SharedString::default(), kind, run: Rc::new(run) }
    }

    fn detail(mut self, detail: impl Into<SharedString>) -> Self {
        self.detail = detail.into();
        self
    }
}

pub struct Palette {
    input: Entity<InputState>,
    items: Vec<Item>,
    /// Indices into `items`, best match first.
    shown: Vec<usize>,
    selected: usize,
    _sub: Subscription,
}

/// An item that runs `f` against the active repository tab.
fn repo_action(
    label: &str,
    f: impl Fn(&mut Workspace, &mut Window, &mut Context<Workspace>) + 'static,
) -> Item {
    Item::new("Action", label.to_string(), move |shell, window, cx| {
        if let Some(ws) = shell.active_workspace() {
            ws.update(cx, |ws, cx| f(ws, window, cx));
        }
    })
}

fn git_op(label: &str, busy: &'static str, change: Change, op: impl Fn(&Repo) -> anyhow::Result<()> + Send + Sync + Copy + 'static) -> Item {
    repo_action(label, move |ws, _, cx| ws.run(busy, change, cx, op))
}

impl Shell {
    pub fn open_palette(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let items = self.palette_items(cx);
        let input = cx.new(|cx| InputState::new(window, cx).placeholder("Type a command, branch or repository…").autocorrect(false));
        let sub = cx.subscribe_in(&input, window, |this, _, event: &InputEvent, window, cx| match event {
            InputEvent::Change => this.filter_palette(cx),
            InputEvent::PressEnter { .. } => this.run_palette(window, cx),
            _ => {}
        });
        input.update(cx, |i, cx| i.focus(window, cx));
        let shown = (0..items.len()).collect();
        self.palette = Some(Palette { input, items, shown, selected: 0, _sub: sub });
        cx.notify();
    }

    pub fn close_palette(&mut self, cx: &mut Context<Self>) {
        self.palette = None;
        cx.notify();
    }

    fn filter_palette(&mut self, cx: &mut Context<Self>) {
        let Some(p) = &mut self.palette else { return };
        let query = p.input.read(cx).value().to_string();
        let mut scored: Vec<(i32, usize)> = p
            .items
            .iter()
            .enumerate()
            .filter_map(|(i, item)| {
                let label = fuzzy::score(&query, &item.label);
                // Details (paths) only count on a literal substring hit, so
                // scattered letters across a long path don't match.
                let lower = query.trim().to_lowercase();
                let detail = (lower.len() >= 2 && item.detail.to_lowercase().contains(&lower)).then_some(5);
                label.max(detail).map(|s| (s, i))
            })
            .collect();
        scored.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
        p.shown = scored.into_iter().map(|(_, i)| i).collect();
        p.selected = 0;
        cx.notify();
    }

    fn move_palette(&mut self, delta: isize, cx: &mut Context<Self>) {
        let Some(p) = &mut self.palette else { return };
        let n = p.shown.len().min(MAX_SHOWN) as isize;
        if n > 0 {
            p.selected = (p.selected as isize + delta).rem_euclid(n) as usize;
            cx.notify();
        }
    }

    fn run_palette_item(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(p) = self.palette.take() else { return };
        cx.notify();
        if let Some(item) = p.shown.get(index).and_then(|&i| p.items.get(i)) {
            (item.run)(self, window, cx);
        }
    }

    fn run_palette(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let selected = self.palette.as_ref().map_or(0, |p| p.selected);
        self.run_palette_item(selected, window, cx);
    }

    fn palette_items(&self, cx: &App) -> Vec<Item> {
        let mut items = vec![
            Item::new("Action", "Open repository or folder…", |s, w, cx| s.prompt_open(w, cx)).detail("⌘O"),
            Item::new("Action", "Clone repository…", |s, w, cx| s.open_clone(w, cx)),
            Item::new("Action", "New repository (git init)…", |s, w, cx| s.prompt_init(w, cx)),
            Item::new("Action", "New tab", |s, w, cx| s.new_tab(w, cx)).detail("⌘T"),
            Item::new("Action", "Close tab", |s, w, cx| {
                let active = s.active;
                s.close_tab(active, w, cx)
            })
            .detail("⌘W"),
            Item::new("Action", "Settings: git identity, SSH keys, signing…", |s, w, cx| s.open_settings(s.active_repo_path(), w, cx)).detail("⌘,"),
        ];

        if let Some(ws) = self.active_workspace() {
            let w = ws.read(cx);
            items.extend([
                git_op("Fetch", "Fetching", Change::Refs, |r| r.fetch()),
                repo_action("Pull", |ws, _, cx| {
                    let mode = Workspace::pull_mode(cx);
                    ws.run("Pulling", Change::Refs, cx, move |r| r.pull(mode));
                }),
                git_op("Pull (rebase)", "Pulling", Change::Refs, |r| r.pull(PullMode::Rebase)),
                repo_action("Push", |ws, _, cx| ws.push(cx)),
                repo_action("Create branch…", |ws, w, cx| ws.prompt_create_branch(w, cx)),
                repo_action("Stash changes…", |ws, w, cx| ws.prompt_stash(w, cx)),
                repo_action("Create worktree…", |ws, w, cx| ws.prompt_new_worktree(w, cx)),
                repo_action("Pop stash", |ws, _, cx| ws.apply_stash(0, true, cx)),
                git_op("Stage all changes", "Staging", Change::Status, |r| r.stage_all()),
                git_op("Unstage all changes", "Unstaging", Change::Status, |r| r.unstage_all()),
                repo_action("Search commits", |ws, w, cx| ws.open_search(w, cx)).detail("⌘F"),
                repo_action("Toggle terminal", |ws, w, cx| ws.toggle_terminal(w, cx)).detail("⌥T"),
                repo_action("New terminal", |ws, w, cx| ws.new_terminal(w, cx)),
                repo_action("Write commit message with AI", |ws, w, cx| {
                    ws.select_wip(cx);
                    ws.generate_commit_message(w, cx)
                }),
                repo_action("AI settings…", |ws, w, cx| ws.open_ai_settings(w, cx)),
                repo_action("Show changes (WIP)", |ws, _, cx| ws.select_wip(cx)),
                repo_action("Refresh", |ws, _, cx| ws.reload(cx)),
            ]);
            if let Some(last) = w.undo_stack.last() {
                items.push(repo_action(&format!("Undo {}", last.label.to_lowercase()), |ws, _, cx| ws.undo(cx)).detail("⌘Z"));
            }
            if let Some(next) = w.redo_stack.last() {
                items.push(repo_action(&format!("Redo {}", next.label.to_lowercase()), |ws, _, cx| ws.redo(cx)).detail("⌘⇧Z"));
            }
            let open_file = match w.diff.as_ref().map(|d| &d.source) {
                Some(crate::workspace::DiffSource::Commit { path, .. }) => Some(path.clone()),
                Some(crate::workspace::DiffSource::Workdir { change, .. }) => Some(change.path.clone()),
                None => w.blame.as_ref().map(|b| b.path.clone()),
            };
            if let Some(path) = open_file {
                let (h, b) = (path.clone(), path.clone());
                items.push(repo_action(&format!("File history: {path}"), move |ws, _, cx| ws.show_file_history(h.clone(), cx)));
                items.push(repo_action(&format!("Blame: {path}"), move |ws, _, cx| ws.show_blame(b.clone(), cx)));
            }
            let root = w.repo.as_ref().map(|r| r.root().to_path_buf());
            for wt in w.snap.worktrees.iter().filter(|wt| Some(&wt.path) != root.as_ref()) {
                let path = wt.path.clone();
                let branch = wt.branch.clone().unwrap_or_else(|| "detached".into());
                items.push(
                    Item::new("Worktree", format!("Open worktree {}", crate::projects::display_name(&path)), move |s, w, cx| {
                        s.open_path(path.clone(), w, cx)
                    })
                    .detail(branch),
                );
            }
            let current = w.status.branch.clone();
            for b in &w.snap.refs.local {
                if Some(&b.name) == current.as_ref() {
                    continue;
                }
                let name = b.name.clone();
                items.push(
                    repo_action(&format!("Checkout {name}"), move |ws, _, cx| {
                        let n = name.clone();
                        ws.run("Checking out", Change::Refs, cx, move |r| r.checkout(&n))
                    })
                    .detail("local branch"),
                );
            }
            for b in &w.snap.refs.remote {
                let name = b.name.clone();
                items.push(
                    repo_action(&format!("Checkout {name}"), move |ws, _, cx| {
                        let n = name.clone();
                        ws.run("Checking out", Change::Refs, cx, move |r| r.checkout_remote(&n))
                    })
                    .detail("remote branch"),
                );
            }
        }

        for (name, path) in self.tab_entries() {
            let target = path.clone();
            items.push(
                Item::new("Tab", format!("Switch to {name}"), move |s, w, cx| s.open_path(target.clone(), w, cx))
                    .detail(crate::projects::short_path(&path)),
            );
        }
        for path in self.known_repos() {
            let target = path.clone();
            items.push(
                Item::new("Repository", format!("Open {}", crate::projects::display_name(&path)), move |s, w, cx| {
                    s.open_path(target.clone(), w, cx)
                })
                .detail(crate::projects::short_path(&path)),
            );
        }
        items
    }

    pub fn render_palette(&self, cx: &mut Context<Self>) -> Option<impl IntoElement> {
        let p = self.palette.as_ref()?;
        Some(
            div()
                .id("palette-backdrop")
                .absolute()
                .inset_0()
                .flex()
                .justify_center()
                .items_start()
                .pt(px(90.))
                .bg(gpui_kit::black().opacity(0.4))
                .on_click(cx.listener(|this, _, _, cx| this.close_palette(cx)))
                .child(
                    div()
                        .id("palette")
                        .key_context("Palette")
                        .on_action(cx.listener(|this, _: &PaletteUp, _, cx| this.move_palette(-1, cx)))
                        .on_action(cx.listener(|this, _: &PaletteDown, _, cx| this.move_palette(1, cx)))
                        .on_action(cx.listener(|this, _: &PaletteClose, _, cx| this.close_palette(cx)))
                        // The input passes Escape on when it has nothing to dismiss itself.
                        .on_action(cx.listener(|this, _: &gpui_kit::component::input::Escape, _, cx| this.close_palette(cx)))
                        .on_click(|_, _, cx| cx.stop_propagation())
                        .w(px(620.))
                        .h_auto()
                        .flex()
                        .flex_col()
                        .rounded_lg()
                        .bg(c(theme::PANEL))
                        .border_1()
                        .border_color(c(theme::BORDER))
                        .shadow_lg()
                        .overflow_hidden()
                        .child(div().p_2().border_b_1().border_color(c(theme::BORDER)).child(Input::new(&p.input).appearance(false)))
                        .when(p.shown.is_empty(), |d| d.child(div().p_4().text_sm().text_color(c(theme::MUTED)).child("No matching commands")))
                        .children(p.shown.iter().take(MAX_SHOWN).enumerate().map(|(row, &i)| {
                            let item = &p.items[i];
                            let selected = row == p.selected;
                            div()
                                .id(("palette-item", row))
                                .flex()
                                .items_center()
                                .gap_3()
                                .px_3()
                                .py_1p5()
                                .cursor_pointer()
                                .when(selected, |d| d.bg(c(theme::SELECTED)))
                                .when(!selected, |d| d.hover(|d| d.bg(c(theme::HOVER))))
                                .child(div().w(px(78.)).flex_none().text_xs().text_color(c(theme::FAINT)).child(item.kind))
                                .child(div().flex_1().min_w_0().truncate().text_sm().child(item.label.clone()))
                                .child(div().flex_none().max_w(px(220.)).truncate().text_xs().text_color(c(theme::FAINT)).child(item.detail.clone()))
                                .on_click(cx.listener(move |this, _, window, cx| this.run_palette_item(row, window, cx)))
                        })),
                ),
        )
    }
}
