//! Embedded terminal: `alacritty_terminal` runs the PTY and VT parser, and we
//! paint its grid on a gpui canvas.

use std::borrow::Cow;
use std::cell::Cell;
use std::rc::Rc;
use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

use alacritty_terminal::event::{Event as TermEvent, EventListener, WindowSize};
use alacritty_terminal::event_loop::{EventLoop, EventLoopSender, Msg};
use alacritty_terminal::grid::{Dimensions, Scroll};
use alacritty_terminal::index::{Column, Line, Point as GridPoint, Side};
use alacritty_terminal::selection::{Selection, SelectionType};
use alacritty_terminal::sync::FairMutex;
use alacritty_terminal::term::cell::Flags;
use alacritty_terminal::term::test::TermSize;
use alacritty_terminal::term::{Config, Term, TermMode};
use alacritty_terminal::tty;
use alacritty_terminal::vte::ansi::{Color as AnsiColor, NamedColor, Rgb};
use anyhow::Result;
use gpui_kit::*;

use crate::theme;

actions!(terminal, [SendTab, SendShiftTab]);

/// Bind keys the app root would otherwise claim (Tab moves focus there).
pub fn init(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("tab", SendTab, Some("Terminal")),
        KeyBinding::new("shift-tab", SendShiftTab, Some("Terminal")),
    ]);
}

const FONT_SIZE: f32 = 13.;
const LINE_HEIGHT: f32 = 1.35;

#[cfg(target_os = "macos")]
const FONT_FAMILY: &str = "Menlo";
#[cfg(target_os = "windows")]
const FONT_FAMILY: &str = "Consolas";
#[cfg(not(any(target_os = "macos", target_os = "windows")))]
const FONT_FAMILY: &str = "DejaVu Sans Mono";

#[derive(Clone)]
struct Listener(smol::channel::Sender<TermEvent>);

impl EventListener for Listener {
    fn send_event(&self, event: TermEvent) {
        self.0.try_send(event).ok();
    }
}

/// Where the last frame put the grid, for mapping mouse positions to cells.
#[derive(Clone, Copy)]
struct GridLayout {
    origin: Point<Pixels>,
    cell_width: Pixels,
    line_height: Pixels,
    cols: usize,
    rows: usize,
    display_offset: i32,
}

pub struct Terminal {
    term: Arc<FairMutex<Term<Listener>>>,
    sender: EventLoopSender,
    pub focus: FocusHandle,
    pub title: SharedString,
    pub exited: bool,
    /// Fractional scroll-wheel lines carried between events.
    scroll_rest: f32,
    layout: Rc<Cell<Option<GridLayout>>>,
    selecting: bool,
    _events: Task<()>,
}

/// A started shell, ready to be wrapped in a [`Terminal`] view.
pub struct Spawned {
    term: Arc<FairMutex<Term<Listener>>>,
    sender: EventLoopSender,
    rx: smol::channel::Receiver<TermEvent>,
}

impl Terminal {
    pub fn spawn(cwd: &Path) -> Result<Spawned> {
        let (tx, rx) = smol::channel::unbounded();
        let listener = Listener(tx);
        let size = WindowSize { num_lines: 24, num_cols: 80, cell_width: 8, cell_height: 18 };
        let options = tty::Options {
            shell: None,
            working_directory: Some(cwd.to_path_buf()),
            drain_on_exit: true,
            env: HashMap::from([
                ("TERM".into(), "xterm-256color".into()),
                ("COLORTERM".into(), "truecolor".into()),
                ("TERM_PROGRAM".into(), "GitChrono".into()),
            ]),
            #[cfg(target_os = "windows")]
            escape_args: true,
        };
        let pty = tty::new(&options, size, 0)?;
        let term = Arc::new(FairMutex::new(Term::new(Config::default(), &TermSize::new(80, 24), listener.clone())));
        let event_loop = EventLoop::new(term.clone(), listener, pty, true, false)?;
        let sender = event_loop.channel();
        event_loop.spawn();
        Ok(Spawned { term, sender, rx })
    }

    pub fn new(spawned: Spawned, cx: &mut Context<Self>) -> Self {
        let Spawned { term, sender, rx } = spawned;
        let events = cx.spawn(async move |this, cx| {
            while let Ok(first) = rx.recv().await {
                let mut batch = vec![first];
                while let Ok(next) = rx.try_recv() {
                    batch.push(next);
                }
                let alive = this.update(cx, |this, cx| {
                    for event in batch {
                        this.handle_event(event, cx);
                    }
                    cx.notify();
                });
                if alive.is_err() {
                    break;
                }
            }
        });

        Self {
            term,
            sender,
            focus: cx.focus_handle(),
            title: "Terminal".into(),
            exited: false,
            scroll_rest: 0.,
            layout: Rc::new(Cell::new(None)),
            selecting: false,
            _events: events,
        }
    }

    fn handle_event(&mut self, event: TermEvent, cx: &mut Context<Self>) {
        match event {
            TermEvent::PtyWrite(text) => self.write(text.into_bytes()),
            TermEvent::Title(title) => self.title = title.into(),
            TermEvent::ResetTitle => self.title = "Terminal".into(),
            TermEvent::ClipboardStore(_, text) => cx.write_to_clipboard(ClipboardItem::new_string(text)),
            TermEvent::ClipboardLoad(_, format) => {
                let text = cx.read_from_clipboard().and_then(|c| c.text()).unwrap_or_default();
                self.write(format(&text).into_bytes());
            }
            TermEvent::TextAreaSizeRequest(format) => {
                let term = self.term.lock();
                let size = WindowSize {
                    num_lines: term.screen_lines() as u16,
                    num_cols: term.columns() as u16,
                    cell_width: 8,
                    cell_height: 18,
                };
                drop(term);
                self.write(format(size).into_bytes());
            }
            TermEvent::Exit | TermEvent::ChildExit(_) => self.exited = true,
            _ => {}
        }
    }

    fn write(&self, bytes: impl Into<Cow<'static, [u8]>>) {
        self.sender.send(Msg::Input(bytes.into())).ok();
    }

    /// Send user input and jump back to the live screen.
    fn input(&self, bytes: impl Into<Cow<'static, [u8]>>) {
        let mut term = self.term.lock();
        term.scroll_display(Scroll::Bottom);
        term.selection = None;
        drop(term);
        self.write(bytes);
    }

    fn paste_text(&self, text: &str) {
        let text = text.replace("\r\n", "\r").replace('\n', "\r");
        if self.term.lock().mode().contains(TermMode::BRACKETED_PASTE) {
            self.input(format!("\x1b[200~{}\x1b[201~", text.replace('\x1b', "")).into_bytes());
        } else {
            self.input(text.into_bytes());
        }
    }

    fn on_key(&mut self, event: &KeyDownEvent, _window: &mut Window, cx: &mut Context<Self>) {
        let ks = &event.keystroke;
        let m = ks.modifiers;
        if m.platform {
            match ks.key.as_str() {
                "v" => {
                    if let Some(text) = cx.read_from_clipboard().and_then(|c| c.text()) {
                        self.paste_text(&text);
                    }
                }
                "k" => self.input(&b"\x0c"[..]),
                "c" => match self.term.lock().selection_to_string().filter(|t| !t.is_empty()) {
                    Some(text) => cx.write_to_clipboard(ClipboardItem::new_string(text)),
                    None => return,
                },
                "a" => {
                    let mut term = self.term.lock();
                    let top = GridPoint::new(Line(-(term.history_size() as i32)), Column(0));
                    let bottom = GridPoint::new(Line(term.screen_lines() as i32 - 1), Column(term.columns() - 1));
                    let mut selection = Selection::new(SelectionType::Simple, top, Side::Left);
                    selection.update(bottom, Side::Right);
                    term.selection = Some(selection);
                    cx.notify();
                }
                // Let app shortcuts (⌘T, ⌘W…) through.
                _ => return,
            }
            cx.stop_propagation();
            return;
        }
        let app_cursor = self.term.lock().mode().contains(TermMode::APP_CURSOR);
        if let Some(bytes) = key_bytes(&ks.key, ks.key_char.as_deref(), m, app_cursor) {
            self.input(bytes);
            cx.stop_propagation();
        }
    }

    fn grid_point(&self, position: Point<Pixels>) -> Option<(GridPoint, Side)> {
        let l = self.layout.get()?;
        let x = f32::from(position.x - l.origin.x) / f32::from(l.cell_width);
        let y = f32::from(position.y - l.origin.y) / f32::from(l.line_height);
        let col = (x.max(0.) as usize).min(l.cols - 1);
        let row = (y.max(0.) as usize).min(l.rows - 1) as i32;
        let side = if x.fract() > 0.5 { Side::Right } else { Side::Left };
        Some((GridPoint::new(Line(row - l.display_offset), Column(col)), side))
    }

    fn on_mouse_down(&mut self, event: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        self.focus.focus(window, cx);
        let Some((point, side)) = self.grid_point(event.position) else { return };
        let mut term = self.term.lock();
        match (&mut term.selection, event.modifiers.shift) {
            (Some(selection), true) => selection.update(point, side),
            _ => {
                let ty = match event.click_count {
                    2 => SelectionType::Semantic,
                    3.. => SelectionType::Lines,
                    _ => SelectionType::Simple,
                };
                term.selection = Some(Selection::new(ty, point, side));
            }
        }
        self.selecting = true;
        cx.notify();
    }

    fn on_mouse_move(&mut self, event: &MouseMoveEvent, _window: &mut Window, cx: &mut Context<Self>) {
        if !self.selecting || event.pressed_button != Some(MouseButton::Left) {
            return;
        }
        if let Some((point, side)) = self.grid_point(event.position) {
            if let Some(selection) = &mut self.term.lock().selection {
                selection.update(point, side);
            }
            cx.notify();
        }
    }

    fn on_mouse_up(&mut self, _event: &MouseUpEvent, _window: &mut Window, cx: &mut Context<Self>) {
        self.selecting = false;
        let mut term = self.term.lock();
        // A plain click (no drag) leaves an empty selection behind.
        if term.selection.as_ref().is_some_and(|s| s.is_empty()) {
            term.selection = None;
        }
        cx.notify();
    }

    fn on_scroll(&mut self, event: &ScrollWheelEvent, _window: &mut Window, cx: &mut Context<Self>) {
        let line_height = px(FONT_SIZE * LINE_HEIGHT);
        self.scroll_rest += f32::from(event.delta.pixel_delta(line_height).y) / f32::from(line_height);
        let lines = self.scroll_rest.trunc() as i32;
        if lines == 0 {
            return;
        }
        self.scroll_rest -= lines as f32;
        let mut term = self.term.lock();
        let mode = *term.mode();
        if mode.contains(TermMode::ALT_SCREEN) && mode.contains(TermMode::ALTERNATE_SCROLL) {
            // Full-screen apps (less, vim) get arrow keys instead.
            drop(term);
            let key: &[u8] = if lines > 0 { b"\x1b[A" } else { b"\x1b[B" };
            self.write(key.repeat(lines.unsigned_abs() as usize));
        } else {
            term.scroll_display(Scroll::Delta(lines));
        }
        cx.notify();
    }
}

/// Typed text (including IME and dead-key compositions) goes straight to the PTY.
impl EntityInputHandler for Terminal {
    fn text_for_range(&mut self, _: std::ops::Range<usize>, _: &mut Option<std::ops::Range<usize>>, _: &mut Window, _: &mut Context<Self>) -> Option<String> {
        None
    }

    fn selected_text_range(&mut self, _: bool, _: &mut Window, _: &mut Context<Self>) -> Option<UTF16Selection> {
        Some(UTF16Selection { range: 0..0, reversed: false })
    }

    fn marked_text_range(&self, _: &mut Window, _: &mut Context<Self>) -> Option<std::ops::Range<usize>> {
        None
    }

    fn unmark_text(&mut self, _: &mut Window, _: &mut Context<Self>) {}

    fn replace_text_in_range(&mut self, _: Option<std::ops::Range<usize>>, text: &str, _: &mut Window, cx: &mut Context<Self>) {
        self.input(text.to_string().into_bytes());
        cx.notify();
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        _: Option<std::ops::Range<usize>>,
        _: &str,
        _: Option<std::ops::Range<usize>>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) {
        // Composition in progress; the final text arrives via replace_text_in_range.
    }

    fn bounds_for_range(&mut self, _: std::ops::Range<usize>, bounds: Bounds<Pixels>, _: &mut Window, _: &mut Context<Self>) -> Option<Bounds<Pixels>> {
        Some(bounds)
    }

    fn character_index_for_point(&mut self, _: Point<Pixels>, _: &mut Window, _: &mut Context<Self>) -> Option<usize> {
        None
    }
}

impl Drop for Terminal {
    fn drop(&mut self) {
        self.sender.send(Msg::Shutdown).ok();
    }
}

/// Bytes a key press sends to the PTY (xterm conventions).
fn key_bytes(key: &str, key_char: Option<&str>, m: Modifiers, app_cursor: bool) -> Option<Vec<u8>> {
    let modifier_code = 1 + u8::from(m.shift) + 2 * u8::from(m.alt) + 4 * u8::from(m.control);
    let cursor = |c: char| -> Vec<u8> {
        match (m.alt, c) {
            // Option+←/→ jump words like macOS terminals.
            (true, 'D') if !m.control && !m.shift => b"\x1bb".to_vec(),
            (true, 'C') if !m.control && !m.shift => b"\x1bf".to_vec(),
            _ if modifier_code > 1 => format!("\x1b[1;{modifier_code}{c}").into_bytes(),
            _ if app_cursor => format!("\x1bO{c}").into_bytes(),
            _ => format!("\x1b[{c}").into_bytes(),
        }
    };
    let tilde = |n: u8| -> Vec<u8> {
        if modifier_code > 1 { format!("\x1b[{n};{modifier_code}~") } else { format!("\x1b[{n}~") }.into_bytes()
    };
    let bytes = match key {
        "enter" => b"\r".to_vec(),
        "backspace" if m.alt || m.control => b"\x17".to_vec(),
        "backspace" => b"\x7f".to_vec(),
        "tab" if m.shift => b"\x1b[Z".to_vec(),
        "tab" => b"\t".to_vec(),
        "escape" => b"\x1b".to_vec(),
        "up" => cursor('A'),
        "down" => cursor('B'),
        "right" => cursor('C'),
        "left" => cursor('D'),
        "home" => cursor('H'),
        "end" => cursor('F'),
        "insert" => tilde(2),
        "delete" => tilde(3),
        "pageup" => tilde(5),
        "pagedown" => tilde(6),
        "f1" => b"\x1bOP".to_vec(),
        "f2" => b"\x1bOQ".to_vec(),
        "f3" => b"\x1bOR".to_vec(),
        "f4" => b"\x1bOS".to_vec(),
        "f5" => tilde(15),
        "f6" => tilde(17),
        "f7" => tilde(18),
        "f8" => tilde(19),
        "f9" => tilde(20),
        "f10" => tilde(21),
        "f11" => tilde(23),
        "f12" => tilde(24),
        "space" if m.control => vec![0],
        "space" => b" ".to_vec(),
        _ if m.control && key.len() == 1 => {
            let c = key.as_bytes()[0].to_ascii_lowercase();
            let code = match c {
                b'a'..=b'z' => c - b'a' + 1,
                b'[' | b'3' => 0x1b,
                b'\\' | b'4' => 0x1c,
                b']' | b'5' => 0x1d,
                b'6' => 0x1e,
                b'/' | b'-' | b'7' => 0x1f,
                b'2' | b'@' => 0,
                _ => return None,
            };
            if m.alt { vec![0x1b, code] } else { vec![code] }
        }
        // Alt+letter as Meta when the layout produced no special character.
        _ if m.alt && key.len() == 1 && key_char.is_none_or(|c| c == key) => [b"\x1b", key.as_bytes()].concat(),
        // Printable text arrives through the platform input handler (IME, dead keys).
        _ => return None,
    };
    Some(bytes)
}

fn ansi(index: usize) -> Rgb {
    const BASE: [(u8, u8, u8); 16] = [
        (0x1d, 0x21, 0x28),
        (0xe0, 0x6c, 0x75),
        (0x3f, 0xb9, 0x50),
        (0xe3, 0xb3, 0x41),
        (0x2f, 0x7d, 0xf6),
        (0xc6, 0x78, 0xdd),
        (0x15, 0xa0, 0xbf),
        (0xc8, 0xcc, 0xd4),
        (0x5c, 0x63, 0x70),
        (0xff, 0x7b, 0x86),
        (0x56, 0xd3, 0x64),
        (0xf2, 0xcc, 0x60),
        (0x58, 0x9c, 0xff),
        (0xd8, 0x93, 0xec),
        (0x39, 0xc5, 0xe0),
        (0xf5, 0xf6, 0xf8),
    ];
    let (r, g, b) = match index {
        0..16 => BASE[index],
        16..232 => {
            let i = index - 16;
            let level = |v: usize| if v == 0 { 0 } else { (55 + v * 40) as u8 };
            (level(i / 36), level(i / 6 % 6), level(i % 6))
        }
        _ => {
            let v = (8 + (index.min(255) - 232) * 10) as u8;
            (v, v, v)
        }
    };
    Rgb { r, g, b }
}

fn hsla(rgb: Rgb) -> Hsla {
    Rgba { r: rgb.r as f32 / 255., g: rgb.g as f32 / 255., b: rgb.b as f32 / 255., a: 1. }.into()
}

fn resolve(color: AnsiColor, colors: &alacritty_terminal::term::color::Colors, bold: bool) -> Hsla {
    let fg = || hsla(Rgb { r: 0xe6, g: 0xe8, b: 0xec });
    match color {
        AnsiColor::Spec(rgb) => hsla(rgb),
        AnsiColor::Indexed(i) => {
            let i = if bold && i < 8 { i + 8 } else { i } as usize;
            colors[i].map_or_else(|| hsla(ansi(i)), hsla)
        }
        AnsiColor::Named(named) => {
            if let Some(rgb) = colors[named] {
                return hsla(rgb);
            }
            let n = named as usize;
            match named {
                NamedColor::Foreground | NamedColor::BrightForeground | NamedColor::Cursor => fg(),
                NamedColor::DimForeground => fg().opacity(0.7),
                NamedColor::Background => theme::c(theme::BG).into(),
                _ if n < 8 && bold => hsla(ansi(n + 8)),
                _ if n < 16 => hsla(ansi(n)),
                // Dim variants follow the bright/normal ones in the enum.
                _ => hsla(ansi((n - NamedColor::DimBlack as usize) % 8)).opacity(0.7),
            }
        }
    }
}

impl Render for Terminal {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let term = self.term.clone();
        let sender = self.sender.clone();
        let entity = cx.entity();
        let focus = self.focus.clone();
        let layout = self.layout.clone();
        let focused = self.focus.is_focused(window);
        div()
            .id("terminal")
            .key_context("Terminal")
            .track_focus(&self.focus)
            .size_full()
            .px_2()
            .py_1()
            .bg(theme::c(theme::BG))
            .on_key_down(cx.listener(Self::on_key))
            .on_scroll_wheel(cx.listener(Self::on_scroll))
            .on_action(cx.listener(|this, _: &SendTab, _, _| this.input(&b"\t"[..])))
            .on_action(cx.listener(|this, _: &SendShiftTab, _, _| this.input(&b"\x1b[Z"[..])))
            .on_mouse_down(MouseButton::Left, cx.listener(Self::on_mouse_down))
            .on_mouse_move(cx.listener(Self::on_mouse_move))
            .on_mouse_up(MouseButton::Left, cx.listener(Self::on_mouse_up))
            .child(canvas(|_, _, _| {}, move |bounds, (), window, cx| {
                window.handle_input(&focus, ElementInputHandler::new(bounds, entity), cx);
                layout.set(Some(paint_grid(&term, &sender, bounds, focused, window, cx)))
            }).size_full())
    }
}

fn paint_grid(term: &FairMutex<Term<Listener>>, sender: &EventLoopSender, bounds: Bounds<Pixels>, focused: bool, window: &mut Window, cx: &mut App) -> GridLayout {
    let font_size = px(FONT_SIZE);
    let line_height = px((FONT_SIZE * LINE_HEIGHT).round());
    let font = font(FONT_FAMILY);
    let text_system = window.text_system().clone();
    let font_id = text_system.resolve_font(&font);
    let cell_width = text_system.advance(font_id, font_size, 'm').map_or(px(8.), |s| s.width);

    let cols = ((f32::from(bounds.size.width) / f32::from(cell_width)).floor() as usize).max(2);
    let rows = ((f32::from(bounds.size.height) / f32::from(line_height)).floor() as usize).max(1);

    let mut term = term.lock();
    if term.columns() != cols || term.screen_lines() != rows {
        term.resize(TermSize::new(cols, rows));
        let cell = |p: Pixels| f32::from(p).round() as u16;
        let size = WindowSize { num_lines: rows as u16, num_cols: cols as u16, cell_width: cell(cell_width), cell_height: cell(line_height) };
        sender.send(Msg::Resize(size)).ok();
    }
    let content = term.renderable_content();
    let offset = content.display_offset as i32;
    let show_cursor = content.mode.contains(TermMode::SHOW_CURSOR) && offset == 0;
    let selection = content.selection;
    let selection_bg: Hsla = theme::c(theme::SELECTED).into();
    let layout = GridLayout { origin: bounds.origin, cell_width, line_height, cols, rows, display_offset: offset };
    let cursor = content.cursor.point;
    let colors = content.colors;
    let bg_default = theme::c(theme::BG).into();

    let mut lines: Vec<(String, Vec<TextRun>, Vec<(usize, usize, Hsla)>)> = vec![Default::default(); rows];
    for indexed in content.display_iter {
        let row = (indexed.point.line.0 + offset) as usize;
        let Some((text, runs, backgrounds)) = lines.get_mut(row) else { continue };
        let cell = indexed.cell;
        let col = indexed.point.column.0;
        let flags = cell.flags;
        let bold = flags.contains(Flags::BOLD);
        let mut fg = resolve(cell.fg, colors, bold);
        let mut bg = resolve(cell.bg, colors, false);
        if flags.contains(Flags::INVERSE) {
            std::mem::swap(&mut fg, &mut bg);
        }
        if flags.contains(Flags::DIM) {
            fg = fg.opacity(0.7);
        }
        if selection.is_some_and(|sel| sel.contains(indexed.point)) {
            bg = selection_bg;
        }
        if bg != bg_default {
            match backgrounds.last_mut() {
                Some((_, end, color)) if *end == col && *color == bg => *end = col + 1,
                _ => backgrounds.push((col, col + 1, bg)),
            }
        }
        let ch = if flags.intersects(Flags::WIDE_CHAR_SPACER | Flags::LEADING_WIDE_CHAR_SPACER) || flags.contains(Flags::HIDDEN) {
            ' '
        } else if cell.c == '\0' || cell.c == '\t' {
            ' '
        } else {
            cell.c
        };
        let start = text.len();
        text.push(ch);
        let len = text.len() - start;
        let mut run_font = font.clone();
        if bold {
            run_font.weight = FontWeight::BOLD;
        }
        if flags.contains(Flags::ITALIC) {
            run_font.style = FontStyle::Italic;
        }
        let underline = flags.intersects(Flags::ALL_UNDERLINES).then(|| UnderlineStyle { thickness: px(1.), color: Some(fg), wavy: false });
        match runs.last_mut() {
            Some(last) if last.color == fg && last.font == run_font && last.underline == underline => last.len += len,
            _ => runs.push(TextRun { len, font: run_font, color: fg, background_color: None, underline, strikethrough: None }),
        }
    }
    drop(term);

    for (row, (text, runs, backgrounds)) in lines.into_iter().enumerate() {
        let y = bounds.origin.y + line_height * row as f32;
        for (start, end, color) in backgrounds {
            let origin = point(bounds.origin.x + cell_width * start as f32, y);
            window.paint_quad(fill(Bounds::new(origin, size(cell_width * (end - start) as f32, line_height)), color));
        }
        if row == cursor.line.0 as usize && show_cursor {
            let origin = point(bounds.origin.x + cell_width * cursor.column.0 as f32, y);
            let cursor_bounds = Bounds::new(origin, size(cell_width, line_height));
            let color: Hsla = theme::c(theme::ACCENT).into();
            if focused {
                window.paint_quad(fill(cursor_bounds, color.opacity(0.75)));
            } else {
                window.paint_quad(outline(cursor_bounds, color, BorderStyle::Solid));
            }
        }
        if text.trim_end().is_empty() {
            continue;
        }
        let shaped = text_system.shape_line(text.into(), font_size, &runs, Some(cell_width));
        shaped.paint(point(bounds.origin.x, y), line_height, TextAlign::Left, None, window, cx).ok();
    }
    layout
}

#[cfg(test)]
mod tests {
    use super::{Listener, Msg, Term, Terminal, key_bytes};
    use alacritty_terminal::grid::Dimensions;
    use gpui_kit::Modifiers;
    use alacritty_terminal::index::{Column, Line, Point};

    fn screen_text(term: &Term<Listener>) -> String {
        let mut out = String::new();
        for line in 0..term.screen_lines() as i32 {
            for col in 0..term.columns() {
                out.push(term.grid()[Point::new(Line(line), Column(col))].c);
            }
            out.push('\n');
        }
        out
    }

    #[test]
    #[cfg(unix)]
    fn runs_a_shell_command() {
        let dir = std::env::temp_dir();
        let spawned = Terminal::spawn(&dir).expect("spawn shell");
        // Arithmetic proves the shell ran it rather than us seeing our own echo.
        spawned.sender.send(Msg::Input(b"echo gitchrono-$((40+2))\r".to_vec().into())).unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        loop {
            if screen_text(&spawned.term.lock()).contains("gitchrono-42") {
                break;
            }
            assert!(std::time::Instant::now() < deadline, "no output:\n{}", screen_text(&spawned.term.lock()));
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        spawned.sender.send(Msg::Shutdown).ok();
    }

    #[test]
    fn maps_keys_like_xterm() {
        let none = Modifiers::default();
        let ctrl = Modifiers { control: true, ..Default::default() };
        let alt = Modifiers { alt: true, ..Default::default() };
        let shift = Modifiers { shift: true, ..Default::default() };
        assert_eq!(key_bytes("enter", None, none, false).unwrap(), b"\r");
        assert_eq!(key_bytes("c", Some("c"), ctrl, false).unwrap(), [3]);
        assert_eq!(key_bytes("up", None, none, false).unwrap(), b"\x1b[A");
        assert_eq!(key_bytes("up", None, none, true).unwrap(), b"\x1bOA");
        assert_eq!(key_bytes("left", None, alt, false).unwrap(), b"\x1bb");
        assert_eq!(key_bytes("right", None, shift, false).unwrap(), b"\x1b[1;2C");
        assert_eq!(key_bytes("tab", None, shift, false).unwrap(), b"\x1b[Z");
        assert_eq!(key_bytes("delete", None, none, false).unwrap(), b"\x1b[3~");
        assert!(key_bytes("a", Some("A"), shift, false).is_none(), "text goes through the input handler");
        assert!(key_bytes("shift", None, shift, false).is_none());
    }
}
