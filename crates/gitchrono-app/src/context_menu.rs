//! Right-click menus drawn by the operating system: an `NSMenu` on macOS, a
//! Win32 menu on Windows, and gpui-component's drawn menu where there is no
//! native one (Linux).
//!
//! Native menus report the chosen item as an action, not a callback, so the
//! open menu's callbacks are kept here and [`MenuChoice`] carries the index of
//! the one to run.

use std::rc::Rc;

use gpui_kit::component::native_menu::NativeMenu;
use gpui_kit::prelude::*;
use gpui_kit::*;

type Run = Rc<dyn Fn(&mut Window, &mut App)>;

pub struct MenuItem {
    label: SharedString,
    disabled: bool,
    run: Run,
}

impl MenuItem {
    pub fn new(label: impl Into<SharedString>, run: impl Fn(&mut Window, &mut App) + 'static) -> Self {
        Self { label: label.into(), disabled: false, run: Rc::new(run) }
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }
}

enum Entry {
    Item(MenuItem),
    Separator,
    Submenu(SharedString, Menu),
}

#[derive(Default)]
pub struct Menu {
    entries: Vec<Entry>,
}

impl FluentBuilder for Menu {}

impl Menu {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn item(mut self, item: MenuItem) -> Self {
        self.entries.push(Entry::Item(item));
        self
    }

    /// A divider, never first and never two in a row.
    pub fn separator(mut self) -> Self {
        if !matches!(self.entries.last(), None | Some(Entry::Separator)) {
            self.entries.push(Entry::Separator);
        }
        self
    }

    pub fn submenu(mut self, label: impl Into<SharedString>, menu: Menu) -> Self {
        self.entries.push(Entry::Submenu(label.into(), menu));
        self
    }

    fn build(self, runs: &mut Vec<Run>) -> NativeMenu {
        let mut entries = self.entries;
        while matches!(entries.last(), Some(Entry::Separator)) {
            entries.pop();
        }
        entries.into_iter().fold(NativeMenu::new(), |menu, entry| match entry {
            Entry::Separator => menu.separator(),
            Entry::Submenu(label, sub) => menu.submenu(label, sub.build(runs)),
            Entry::Item(item) => {
                runs.push(item.run);
                menu.menu_with_disabled(item.label, item.disabled, Box::new(MenuChoice(runs.len() - 1)))
            }
        })
    }

    /// Show the menu at `position` (window coordinates).
    pub fn show(self, position: Point<Pixels>, window: &mut Window, cx: &mut App) {
        let mut runs = Vec::new();
        let menu = self.build(&mut runs);
        cx.set_global(OpenMenu(runs));
        menu.show(position, window, cx);
    }
}

/// The callbacks of the menu last shown, by [`MenuChoice`] index.
#[derive(Default)]
struct OpenMenu(Vec<Run>);

impl Global for OpenMenu {}

/// The item picked from the open menu. Built here, never from a keymap.
#[derive(Clone, Debug, PartialEq)]
pub struct MenuChoice(usize);

impl Action for MenuChoice {
    fn boxed_clone(&self) -> Box<dyn Action> {
        Box::new(self.clone())
    }

    fn partial_eq(&self, action: &dyn Action) -> bool {
        action.as_any().downcast_ref::<Self>() == Some(self)
    }

    fn name(&self) -> &'static str {
        Self::name_for_type()
    }

    fn name_for_type() -> &'static str {
        "gitchrono::MenuChoice"
    }

    fn build(_: serde_json::Value) -> anyhow::Result<Box<dyn Action>> {
        anyhow::bail!("menu choices can't be bound to keys")
    }
}

/// Run the chosen item. The window's root handles [`MenuChoice`] with this.
pub fn choose(choice: &MenuChoice, window: &mut Window, cx: &mut App) {
    let run = cx.try_global::<OpenMenu>().and_then(|menu| menu.0.get(choice.0).cloned());
    if let Some(run) = run {
        run(window, cx);
    }
}

pub trait RightClickMenu: InteractiveElement + Sized {
    /// Show the menu `build` makes when the element is right-clicked.
    fn right_click_menu(self, build: impl Fn(&mut Window, &mut App) -> Menu + 'static) -> Self {
        self.on_mouse_down(MouseButton::Right, move |event, window, cx| {
            cx.stop_propagation();
            build(window, cx).show(event.position, window, cx);
        })
    }
}

impl<E: InteractiveElement> RightClickMenu for E {}

#[cfg(test)]
mod tests {
    use super::{Menu, MenuItem};

    fn labels(menu: &Menu) -> Vec<&str> {
        menu.entries
            .iter()
            .map(|e| match e {
                super::Entry::Item(item) => item.label.as_ref(),
                super::Entry::Separator => "—",
                super::Entry::Submenu(label, _) => label.as_ref(),
            })
            .collect()
    }

    #[test]
    fn separators_only_go_between_items() {
        let menu = Menu::new().separator().item(MenuItem::new("a", |_, _| {})).separator().separator().item(MenuItem::new("b", |_, _| {}));
        assert_eq!(labels(&menu), ["a", "—", "b"]);
    }

    #[test]
    fn every_item_gets_its_own_choice() {
        let sub = Menu::new().item(MenuItem::new("c", |_, _| {}));
        let menu = Menu::new().item(MenuItem::new("a", |_, _| {})).separator().submenu("more", sub).item(MenuItem::new("b", |_, _| {})).separator();
        let mut runs = Vec::new();
        let native = menu.build(&mut runs);
        assert_eq!(runs.len(), 3, "a, c inside the submenu, b");
        assert!(!native.is_empty());
    }
}
