//! Paints one row of the commit graph into a canvas.

use gitchrono_core::{Commit, RefKind};
use gitchrono_graph::{Graph, Row};
use gpui_kit::*;

use crate::theme;

pub const ROW_HEIGHT: f32 = 28.;
const LANE_WIDTH: f32 = 20.;
/// Wider graphs clip here rather than squeezing lanes together.
const MAX_GUTTER: f32 = 340.;
const PAD: f32 = 10.;
/// Narrowest graph column: one dot per commit, as GitKraken does when collapsed.
pub const MIN_GUTTER: f32 = 28.;
const NODE_RADIUS: f32 = 6.;
/// Half the side of a stash's box, which holds the stash tray icon.
pub const STASH_BOX: f32 = 8.;
/// Corner radius of the elbow joining a node to another lane.
const ELBOW: f32 = 8.;

#[derive(Clone, Copy)]
pub enum Node {
    /// `labelled`: the row has ref labels, joined to the node by a line.
    /// `stash`: the commit is a stash, drawn as a dashed box joined to its base
    /// by a dashed line. `stash_lines`: one bit per lane whose line through
    /// this row is a stash's (see [`stash_lines`]).
    /// `wip_line`: the line from the WIP row to HEAD, when it crosses this row.
    Commit { merge: bool, labelled: bool, stash: bool, stash_lines: u64, wip_line: Option<WipLine> },
    /// The uncommitted-changes row: a dashed ring, joined to HEAD if there is one.
    Wip { attached: bool },
}

/// The dashed line from the WIP row down to HEAD, which needn't be the next
/// commit: it runs in HEAD's lane until it `ends` at HEAD's node.
#[derive(Clone, Copy)]
pub struct WipLine {
    pub lane: usize,
    pub color: usize,
    pub ends: bool,
}

/// Narrowest graph column that still draws lanes (below it: dots only).
pub const MIN_FULL_GUTTER: f32 = PAD * 2. + LANE_WIDTH * 2.;

/// A node's centre, from the left of the graph column.
pub fn node_x(lane: usize) -> f32 {
    PAD + lane as f32 * LANE_WIDTH + LANE_WIDTH / 2.
}

fn lane_x(bounds: &Bounds<Pixels>, lane: usize) -> Pixels {
    bounds.left() + px(PAD + lane as f32 * LANE_WIDTH + LANE_WIDTH / 2.)
}

pub fn gutter_width(max_lane: usize) -> Pixels {
    // A single lane still needs room for its line, or linear histories lose it.
    px((PAD * 2. + (max_lane + 1) as f32 * LANE_WIDTH).clamp(MIN_FULL_GUTTER, MAX_GUTTER))
}

/// Per row, a bit for each lane carrying a stash's line down to its base, so
/// the line stays dashed all the way rather than just leaving the stash.
pub fn stash_lines(graph: &Graph, commits: &[Commit]) -> Vec<u64> {
    let mut lines = vec![0u64; graph.rows.len()];
    let stashes = commits.iter().enumerate().filter(|(_, c)| c.refs.iter().any(|r| r.kind == RefKind::Stash));
    for (row, _) in stashes {
        let Some(lane) = graph.rows.get(row).map(|r| r.lane).filter(|l| *l < 64) else { continue };
        // Lanes only move at nodes, so the line holds its lane until it arrives.
        for (r, below) in graph.rows.iter().enumerate().skip(row + 1) {
            let Some(e) = below.edges.iter().find(|e| e.from_lane == lane && !e.from_node) else { break };
            lines[r] |= 1 << lane;
            if e.to_node {
                break;
            }
        }
    }
    lines
}

pub fn row_canvas(row: Row, node: Node) -> impl IntoElement {
    canvas(
        |_, _, _| {},
        move |bounds, _, window, _| {
            window.with_content_mask(Some(ContentMask { bounds }), |window| {
                paint_row(bounds, &row, node, window);
            });
        },
    )
    .size_full()
}

/// How a line between two lanes bends.
#[derive(Clone, Copy, PartialEq)]
enum Bend {
    /// S-curve across the row (a lane shifting sideways).
    Curve,
    /// Horizontal out of the node, then a rounded turn down (merge or fork).
    OutThenDown,
    /// Down the lane, then a rounded turn into the node (a branch's parent).
    DownThenIn,
}

fn segment(window: &mut Window, from: Point<Pixels>, to: Point<Pixels>, color: Hsla, bend: Bend, dashed: bool) {
    let mut path = PathBuilder::stroke(px(2.));
    if dashed {
        path = path.dash_array(&[px(3.), px(3.)]);
    }
    path.move_to(from);
    let r = px(ELBOW).min((to.x - from.x).abs()).min((to.y - from.y).abs());
    let dir = if to.x > from.x { 1. } else { -1. };
    match bend {
        _ if from.x == to.x => path.line_to(to),
        Bend::Curve => {
            let mid = (from.y + to.y) / 2.;
            path.cubic_bezier_to(to, point(from.x, mid), point(to.x, mid));
        }
        Bend::OutThenDown => {
            let corner = point(to.x, from.y);
            path.line_to(point(to.x - r * dir, from.y));
            path.cubic_bezier_to(point(to.x, from.y + r), corner, corner);
            path.line_to(to);
        }
        Bend::DownThenIn => {
            let corner = point(from.x, to.y);
            path.line_to(point(from.x, to.y - r));
            path.cubic_bezier_to(point(from.x + r * dir, to.y), corner, corner);
            path.line_to(to);
        }
    }
    if let Ok(path) = path.build() {
        window.paint_path(path, color);
    }
}

fn paint_row(bounds: Bounds<Pixels>, row: &Row, node: Node, window: &mut Window) {
    let top = bounds.top();
    let bottom = bounds.bottom();
    let mid = top + px(ROW_HEIGHT / 2.);
    // Too narrow for two lanes: just the dots, centred.
    if bounds.size.width < px(MIN_FULL_GUTTER) {
        let x = bounds.left() + bounds.size.width / 2.;
        if let Node::Commit { stash: true, .. } = node {
            paint_stash_box(window, x, mid);
            return;
        }
        if let Node::Wip { .. } = node {
            paint_wip_ring(window, x, mid, theme::lane(row.color));
            return;
        }
        paint_node(window, x, mid, theme::lane(row.color), matches!(node, Node::Commit { merge: true, .. }));
        return;
    }
    let node_x = lane_x(&bounds, row.lane);

    if let Node::Wip { attached } = node {
        let color = theme::lane(row.color);
        // The band joining the ring to the message, as on GitKraken's WIP row.
        let band = Bounds::from_corners(point(node_x, top + px(1.)), point(bounds.right(), bottom - px(1.)));
        window.paint_quad(fill(band, color.opacity(0.35)));
        if attached {
            segment(window, point(node_x, mid), point(node_x, bottom), color, Bend::Curve, true);
        }
        paint_wip_ring(window, node_x, mid, color);
        return;
    }

    let Node::Commit { stash, stash_lines, wip_line, .. } = node else { return };
    // Under the commit's own lines, and under any node it passes.
    if let Some(line) = wip_line {
        let x = lane_x(&bounds, line.lane);
        segment(window, point(x, top), point(x, if line.ends { mid } else { bottom }), theme::lane(line.color), Bend::Curve, true);
    }
    for e in &row.edges {
        let dashed = if e.from_node { stash } else { e.from_lane < 64 && stash_lines & (1 << e.from_lane) != 0 };
        let color = if dashed { theme::c(theme::MUTED).into() } else { theme::lane(e.color) };
        let (from, to, bend) = match (e.to_node, e.from_node) {
            (true, _) => (point(lane_x(&bounds, e.from_lane), top), point(node_x, mid), Bend::DownThenIn),
            (_, true) => (point(node_x, mid), point(lane_x(&bounds, e.to_lane), bottom), Bend::OutThenDown),
            _ => (point(lane_x(&bounds, e.from_lane), top), point(lane_x(&bounds, e.to_lane), bottom), Bend::Curve),
        };
        segment(window, from, to, color, bend, dashed);
    }

    if let Node::Commit { labelled: true, .. } = node {
        let mut line = PathBuilder::stroke(px(1.));
        line.move_to(point(bounds.left(), mid));
        line.line_to(point(node_x, mid));
        if let Ok(path) = line.build() {
            let color = if stash { theme::c(theme::MUTED).into() } else { theme::lane(row.color) };
            window.paint_path(path, color.opacity(0.6));
        }
    }
    if stash {
        paint_stash_box(window, node_x, mid);
        return;
    }
    let merge = matches!(node, Node::Commit { merge: true, .. });
    paint_node(window, node_x, mid, theme::lane(row.color), merge);
}

fn paint_node(window: &mut Window, x: Pixels, y: Pixels, color: Hsla, hollow: bool) {
    let r = if hollow { NODE_RADIUS - 1. } else { NODE_RADIUS };
    let bounds = Bounds::new(point(x - px(r), y - px(r)), size(px(r * 2.), px(r * 2.)));
    let fill: Hsla = if hollow { theme::c(theme::BG).into() } else { color };
    window.paint_quad(quad(bounds, px(r), fill, px(2.), color, BorderStyle::default()));
}

/// The WIP row's node: a ring of dashes, a little larger than a commit's dot.
fn paint_wip_ring(window: &mut Window, x: Pixels, y: Pixels, color: Hsla) {
    let r = px(NODE_RADIUS + 2.);
    let bounds = Bounds::new(point(x - r, y - r), size(r * 2., r * 2.));
    window.paint_quad(quad(bounds, r, theme::c(theme::BG), px(2.), color, BorderStyle::Dashed));
}

/// A stash's node: a dashed box, which the row fills with the stash tray icon.
fn paint_stash_box(window: &mut Window, x: Pixels, y: Pixels) {
    let bounds = Bounds::new(point(x - px(STASH_BOX), y - px(STASH_BOX)), size(px(STASH_BOX * 2.), px(STASH_BOX * 2.)));
    let color: Hsla = theme::c(theme::MUTED).into();
    window.paint_quad(quad(bounds, px(3.), theme::c(theme::BG), px(1.5), color, BorderStyle::Dashed));
}
