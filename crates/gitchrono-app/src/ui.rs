//! Building blocks shared by the views.

use gpui_kit::component::Disableable as _;
use gpui_kit::component::button::Button;
use gpui_kit::prelude::*;
use gpui_kit::*;

use crate::theme;

/// A kit button with the pointing-hand cursor. The kit keeps the arrow on
/// everything but link buttons; use this instead of `Button::new`.
pub fn button(id: impl Into<ElementId>) -> Button {
    Button::new(id).cursor_pointer()
}

/// Change counts as (text, color), most urgent first, leaving out zeros.
pub fn change_parts(conflicted: u32, modified: u32, added: u32, deleted: u32) -> Vec<(String, u32)> {
    [(conflicted, "⚠", theme::DEL), (modified, "✎", theme::WARN), (added, "+", theme::ADD), (deleted, "−", theme::DEL)]
        .into_iter()
        .filter(|(n, ..)| *n > 0)
        .map(|(n, sign, color)| (format!("{sign}{n}"), color))
        .collect()
}

pub trait ButtonExt {
    /// `disabled`, with the arrow cursor while disabled. The kit re-applies
    /// our hand cursor in its disabled state, so plain `disabled` keeps it.
    fn disabled_if(self, disabled: bool) -> Self;
}

impl ButtonExt for Button {
    fn disabled_if(self, disabled: bool) -> Self {
        let button = self.disabled(disabled);
        if disabled { button.cursor_default() } else { button.cursor_pointer() }
    }
}
