mod ai;
mod diff_view;
mod fuzzy;
mod graph_paint;
mod menus;
mod palette;
mod projects;
mod shell;
mod terminal;
mod theme;
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
        terminal::init(cx);
        shell::init(cx);
        palette::init(cx);
        workspace::init(cx);

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
