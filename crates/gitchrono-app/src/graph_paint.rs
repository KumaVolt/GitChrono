//! Paints one row of the commit graph into a canvas.

use gitchrono_graph::Row;
use gpui_kit::*;

use crate::theme;

pub const ROW_HEIGHT: f32 = 26.;
/// Lanes are this wide while the graph fits in `MAX_GUTTER`, then narrow down
/// to `MIN_LANE_WIDTH` so wide histories show more lanes before clipping.
const LANE_WIDTH: f32 = 16.;
const MIN_LANE_WIDTH: f32 = 10.;
const MAX_GUTTER: f32 = 360.;
const PAD: f32 = 10.;
const NODE_RADIUS: f32 = 5.5;

#[derive(Clone, Copy)]
pub enum Node {
    Commit { merge: bool },
    /// The uncommitted-changes row: hollow node joined to HEAD below it.
    Wip,
}

fn lane_x(bounds: &Bounds<Pixels>, lane: usize, lane_width: f32) -> Pixels {
    bounds.left() + px(PAD + lane as f32 * lane_width + lane_width / 2.)
}

/// Graph column width and the lane width to paint with.
pub fn gutter(max_lane: usize) -> (Pixels, f32) {
    let lanes = (max_lane + 1) as f32;
    let lane_width = ((MAX_GUTTER - PAD * 2.) / lanes).clamp(MIN_LANE_WIDTH, LANE_WIDTH);
    (px((PAD * 2. + lanes * lane_width).min(MAX_GUTTER)), lane_width)
}

pub fn row_canvas(row: Row, node: Node, lane_width: f32) -> impl IntoElement {
    canvas(
        |_, _, _| {},
        move |bounds, _, window, _| {
            window.with_content_mask(Some(ContentMask { bounds }), |window| {
                paint_row(bounds, &row, node, lane_width, window);
            });
        },
    )
    .size_full()
}

fn segment(window: &mut Window, from: Point<Pixels>, to: Point<Pixels>, color: Hsla) {
    let mut path = PathBuilder::stroke(px(2.));
    path.move_to(from);
    if from.x == to.x {
        path.line_to(to);
    } else {
        let mid = (from.y + to.y) / 2.;
        path.cubic_bezier_to(to, point(from.x, mid), point(to.x, mid));
    }
    if let Ok(path) = path.build() {
        window.paint_path(path, color);
    }
}

fn paint_row(bounds: Bounds<Pixels>, row: &Row, node: Node, lane_width: f32, window: &mut Window) {
    let lane_x = |bounds: &Bounds<Pixels>, lane: usize| lane_x(bounds, lane, lane_width);
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
        let (from, to) = match (e.to_node, e.from_node) {
            (true, _) => (point(lane_x(&bounds, e.from_lane), top), point(node_x, mid)),
            (_, true) => (point(node_x, mid), point(lane_x(&bounds, e.to_lane), bottom)),
            _ => (point(lane_x(&bounds, e.from_lane), top), point(lane_x(&bounds, e.to_lane), bottom)),
        };
        segment(window, from, to, color);
    }

    let merge = matches!(node, Node::Commit { merge: true });
    paint_node(window, node_x, mid, theme::lane(row.color), merge);
}

fn paint_node(window: &mut Window, x: Pixels, y: Pixels, color: Hsla, hollow: bool) {
    let r = if hollow { NODE_RADIUS - 1. } else { NODE_RADIUS };
    let bounds = Bounds::new(point(x - px(r), y - px(r)), size(px(r * 2.), px(r * 2.)));
    let fill: Hsla = if hollow { theme::c(theme::BG).into() } else { color };
    window.paint_quad(quad(bounds, px(r), fill, px(2.), color, BorderStyle::default()));
}
