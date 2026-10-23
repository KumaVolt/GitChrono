//! GitChrono's dark palette. Kept separate from gpui-component's theme so the
//! graph and list chrome can be tuned without touching widget styling.

use gpui_kit::{Hsla, Rgba, rgb};

pub const BG: u32 = 0x1b1e24;
pub const PANEL: u32 = 0x22262e;
pub const PANEL_ALT: u32 = 0x262a33;
pub const BORDER: u32 = 0x323741;
pub const TEXT: u32 = 0xd5d8de;
pub const MUTED: u32 = 0x8b919c;
pub const FAINT: u32 = 0x5d636e;
pub const HOVER: u32 = 0x2a2f38;
pub const SELECTED: u32 = 0x2c3d5a;
pub const ACCENT: u32 = 0x3d8bfd;
pub const ADD: u32 = 0x3fb950;
pub const DEL: u32 = 0xf0524f;
pub const ADD_BG: u32 = 0x1d3325;
pub const DEL_BG: u32 = 0x3a1f22;
pub const WARN: u32 = 0xe3b341;

/// Lane colors, indexed by `gitchrono_graph::Edge::color`.
const LANES: [u32; gitchrono_graph::COLOR_COUNT] = [
    0x15a0bf, 0x2f7df6, 0x9b4fe0, 0xd23fbf, 0xe0457b, 0xe5534b, 0xf0883e, 0xe3b341, 0x7bc74d, 0x2ec9a0,
];

pub fn c(hex: u32) -> Rgba {
    rgb(hex)
}

pub fn lane(color: usize) -> Hsla {
    rgb(LANES[color % LANES.len()]).into()
}
