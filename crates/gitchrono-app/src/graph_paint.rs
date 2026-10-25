//! Paints one row of the commit graph into a canvas.

use gitchrono_graph::Row;
use gpui_kit::*;

use crate::theme;

pub const ROW_HEIGHT: f32 = 28.;
const LANE_WIDTH: f32 = 20.;
/// Wider graphs clip here rather than squeezing lanes together.
const MAX_GUTTER: f32 = 340.;
const PAD: f32 = 10.;
const NODE_RADIUS: f32 = 6.;
/// Corner radius of the elbow joining a node to another lane.
const ELBOW: f32 = 8.;

#[derive(Clone, Copy)]
pub enum Node {
    /// `labelled`: the row has ref labels, joined to the node by a line.
    Commit { merge: bool, labelled: bool },
    /// The uncommitted-changes row: hollow node joined to HEAD below it.
    Wip,
}

fn lane_x(bounds: &Bounds<Pixels>, lane: usize) -> Pixels {
    bounds.left() + px(PAD + lane as f32 * LANE_WIDTH + LANE_WIDTH / 2.)
}

pub fn gutter_width(max_lane: usize) -> Pixels {
    px((PAD * 2. + (max_lane + 1) as f32 * LANE_WIDTH).min(MAX_GUTTER))
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

fn segment(window: &mut Window, from: Point<Pixels>, to: Point<Pixels>, color: Hsla, bend: Bend) {
    let mut path = PathBuilder::stroke(px(2.));
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
    let node_x = lane_x(&bounds, row.lane);

    if let Node::Wip = node {
        let color: Hsla = theme::c(theme::MUTED).into();
        let mut dashed = PathBuilder::stroke(px(2.)).dash_array(&[px(3.), px(3.)]);
        dashed.move_to(point(node_x, mid));
        dashed.line_to(point(node_x, bottom));
        if let Ok(p) = dashed.build() {
            window.paint_path(p, color);
        }
        paint_node(window, node_x, mid, color, true);
        return;
    }

    for e in &row.edges {
        let color = theme::lane(e.color);
        let (from, to, bend) = match (e.to_node, e.from_node) {
            (true, _) => (point(lane_x(&bounds, e.from_lane), top), point(node_x, mid), Bend::DownThenIn),
            (_, true) => (point(node_x, mid), point(lane_x(&bounds, e.to_lane), bottom), Bend::OutThenDown),
            _ => (point(lane_x(&bounds, e.from_lane), top), point(lane_x(&bounds, e.to_lane), bottom), Bend::Curve),
        };
        segment(window, from, to, color, bend);
    }

    if let Node::Commit { labelled: true, .. } = node {
        let mut line = PathBuilder::stroke(px(1.));
        line.move_to(point(bounds.left(), mid));
        line.line_to(point(node_x, mid));
        if let Ok(path) = line.build() {
            window.paint_path(path, theme::lane(row.color).opacity(0.6));
        }
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
