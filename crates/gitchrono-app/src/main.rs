mod ai;
mod avatars;
mod conflict_ui;
mod diff_view;
mod drag;
mod file_tree;
mod footer;
mod git_help;
mod fuzzy;
mod github;
mod graph_paint;
mod highlight;
mod history;
mod menus;
mod message;
mod palette;
mod projects;
mod rebase_ui;
mod settings;
mod shell;
mod terminal;
mod theme;
mod update;
mod views;
mod watcher;
mod workspace;

use gpui_kit::component::theme::{Theme, ThemeMode};
use gpui_kit::*;

use shell::Shell;

fn main() {
    let path = std::env::args().nth(1).map(std::path::PathBuf::from);

    gpui_kit::application().run(move |cx| {
        gpui_kit::init(cx);
        Theme::change(ThemeMode::Dark, None, cx);
        // Scrollable boxes (like a long commit message) show their scrollbar.
        Theme::set_scrollbar_mode(gpui_kit::component::scroll::ScrollbarMode::Always, cx);
        terminal::init(cx);
        shell::init(cx);
        palette::init(cx);
        workspace::init(cx);
        avatars::init(projects::Session::load().gravatar_enabled(), cx);
        update::cleanup();

        let bounds = Bounds::centered(None, size(px(1440.), px(900.)), cx);
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            titlebar: Some(TitlebarOptions { title: Some("GitChrono".into()), ..Default::default() }),
            ..Default::default()
        };
        gpui_kit::open_window(options, cx, |window, cx| cx.new(|cx| Shell::new(path, window, cx)))
            .expect("failed to open window");
        cx.activate(true);
    });
}
