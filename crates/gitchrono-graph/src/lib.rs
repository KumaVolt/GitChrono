//! Commit-graph lane layout.
//!
//! Pure transform from an ordered commit list (children before parents) into
//! per-row lane/color/edge data the UI paints. Each row is a fixed-height band
//! with the node at the vertical centre of its lane:
//!   - `to_node` edge:   (from_lane, top)    -> (node lane, centre)  child merging in
//!   - `from_node` edge: (node lane, centre) -> (to_lane, bottom)    parent branching out
//!   - pass-through:     (from_lane, top)    -> (to_lane, bottom)    unrelated lane

use std::collections::HashMap;

pub const COLOR_COUNT: usize = 10;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Edge {
    pub from_lane: usize,
    pub to_lane: usize,
    pub color: usize,
    pub from_node: bool,
    pub to_node: bool,
}

#[derive(Debug, Clone, Default)]
pub struct Row {
    pub lane: usize,
    pub color: usize,
    pub edges: Vec<Edge>,
}

#[derive(Debug, Clone, Default)]
pub struct Graph {
    pub rows: Vec<Row>,
    /// Highest lane index used, for sizing the gutter.
    pub max_lane: usize,
}

/// `commits` yields `(id, parent_ids)` newest first.
pub fn layout<'a, I, P>(commits: I) -> Graph
where
    I: IntoIterator<Item = (&'a str, P)>,
    P: IntoIterator<Item = &'a str>,
{
    let mut rows = Vec::new();
    // lanes[i] = the commit this lane is flowing toward, or None if free.
    let mut lanes: Vec<Option<&'a str>> = Vec::new();
    let mut lane_color: Vec<usize> = Vec::new();
    // Reverse index of lanes so lookups stay O(1) on wide graphs.
    let mut waiting: HashMap<&'a str, Vec<usize>> = HashMap::new();
    let mut next_color = 0usize;
    let mut max_lane = 0usize;

    let alloc = |lanes: &mut Vec<Option<&'a str>>, lane_color: &mut Vec<usize>, id: &'a str| {
        let idx = match lanes.iter().position(Option::is_none) {
            Some(i) => i,
            None => {
                lanes.push(None);
                lane_color.push(0);
                lanes.len() - 1
            }
        };
        lanes[idx] = Some(id);
        idx
    };

    for (id, parents) in commits {
        let mut edges = Vec::new();
        let mut incoming = waiting.remove(id).unwrap_or_default();
        incoming.sort_unstable();

        let (lane, color) = match incoming.first() {
            Some(&l) => (l, lane_color[l]),
            None => {
                let l = alloc(&mut lanes, &mut lane_color, id);
                let c = next_color % COLOR_COUNT;
                next_color += 1;
                lane_color[l] = c;
                (l, c)
            }
        };

        for (i, slot) in lanes.iter_mut().enumerate() {
            if i == lane {
                continue;
            }
            match *slot {
                None => {}
                Some(target) if target == id => {
                    edges.push(Edge { from_lane: i, to_lane: lane, color: lane_color[i], from_node: false, to_node: true });
                    *slot = None;
                }
                Some(_) => {
                    edges.push(Edge { from_lane: i, to_lane: i, color: lane_color[i], from_node: false, to_node: false });
                }
            }
        }
        if !incoming.is_empty() {
            edges.push(Edge { from_lane: lane, to_lane: lane, color, from_node: false, to_node: true });
        }

        lanes[lane] = None;
        for (pi, parent) in parents.into_iter().enumerate() {
            if pi == 0 {
                // First parent continues straight down, unless another lane already
                // waits for it: then the two join here, into the lower lane, instead
                // of running side by side to the parent's row (as `git log --graph`).
                let others = waiting.get(parent).cloned().unwrap_or_default();
                let target = others.iter().copied().chain([lane]).min().unwrap_or(lane);
                edges.push(Edge { from_lane: lane, to_lane: target, color: if target == lane { color } else { lane_color[target] }, from_node: true, to_node: false });
                if target == lane {
                    lanes[lane] = Some(parent);
                    for &other in &others {
                        // Its pass-through bends into our lane on this row.
                        if let Some(e) = edges.iter_mut().find(|e| e.from_lane == other && e.to_lane == other && !e.from_node && !e.to_node) {
                            e.to_lane = lane;
                        }
                        lanes[other] = None;
                    }
                }
                waiting.insert(parent, vec![target]);
            } else {
                let (plane, pcolor) = match waiting.get(parent).and_then(|v| v.first()) {
                    Some(&l) => (l, lane_color[l]),
                    None => {
                        let l = alloc(&mut lanes, &mut lane_color, parent);
                        let c = next_color % COLOR_COUNT;
                        next_color += 1;
                        lane_color[l] = c;
                        waiting.entry(parent).or_default().push(l);
                        (l, c)
                    }
                };
                edges.push(Edge { from_lane: lane, to_lane: plane, color: pcolor, from_node: true, to_node: false });
            }
        }

        max_lane = max_lane.max(lane);
        for e in &edges {
            max_lane = max_lane.max(e.from_lane).max(e.to_lane);
        }
        rows.push(Row { lane, color, edges });
    }

    Graph { rows, max_lane }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(commits: &[(&'static str, &[&'static str])]) -> Graph {
        layout(commits.iter().map(|(id, ps)| (*id, ps.iter().copied())))
    }

    #[test]
    fn linear_history_stays_in_lane_zero() {
        let g = run(&[("c", &["b"]), ("b", &["a"]), ("a", &[])]);
        assert!(g.rows.iter().all(|r| r.lane == 0));
        assert_eq!(g.max_lane, 0);
    }

    #[test]
    fn merge_opens_and_closes_a_lane() {
        // m merges f into b; f and b share parent a.
        let g = run(&[("m", &["b", "f"]), ("f", &["a"]), ("b", &["a"]), ("a", &[])]);
        assert_eq!(g.rows[0].lane, 0);
        assert_eq!(g.rows[1].lane, 1, "feature commit gets its own lane");
        assert_eq!(g.rows[2].lane, 0);
        assert_eq!(g.rows[3].lane, 0, "lanes converge on the shared parent");
        assert_eq!(g.max_lane, 1);
    }

    #[test]
    fn two_branch_tips_get_separate_lanes() {
        let g = run(&[("x", &["a"]), ("y", &["a"]), ("a", &[])]);
        assert_eq!(g.rows[0].lane, 0);
        assert_eq!(g.rows[1].lane, 1);
        assert_eq!(g.rows[2].lane, 0);
    }

    #[test]
    fn branches_from_the_same_commit_share_a_lane_early() {
        // Three tips forked from `a`, with unrelated mainline commits in between:
        // y and z join x's lane right away instead of each running down to `a`.
        let g = run(&[("x", &["a"]), ("y", &["a"]), ("z", &["a"]), ("m2", &["m1"]), ("m1", &["a"]), ("a", &[])]);
        assert_eq!((g.rows[0].lane, g.rows[1].lane, g.rows[2].lane), (0, 1, 1), "z reuses the lane y freed");
        assert!(g.rows[1].edges.iter().any(|e| e.from_node && e.from_lane == 1 && e.to_lane == 0), "y bends into x's lane");
        assert_eq!(g.rows[3].lane, 1, "the mainline takes the next free lane");
        assert_eq!(g.max_lane, 1, "never more than two lanes");
    }

    #[test]
    fn a_lower_lane_pulls_in_higher_ones_waiting_for_the_same_parent() {
        // q (lane 1) waits for `a`; then p in lane 0 also needs `a`: q's line bends into lane 0.
        let g = run(&[("m", &["p", "q"]), ("q", &["a"]), ("p", &["a"]), ("a", &[])]);
        assert_eq!(g.rows[2].lane, 0);
        assert!(g.rows[2].edges.iter().any(|e| !e.from_node && !e.to_node && e.from_lane == 1 && e.to_lane == 0));
        assert!(g.rows[3].edges.iter().all(|e| e.from_lane == 0 && e.to_lane == 0), "only lane 0 reaches `a`");
    }
}
