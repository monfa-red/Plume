//! Single-wire routing: A\* over the orthogonal visibility grid.
//!
//! One wire at a time, obstacles only (other wires arrive in later phases). The
//! cost is bends first, then length (WIRING B4/B5): a large per-bend penalty
//! makes "fewest turns" win, with length breaking the remainder. Every grid edge
//! keeps ≥ clearance from obstacles, so any route A\* returns is B1- and
//! wire-node-B2-clean by construction. Returns `None` only when the ports are
//! boxed in — the caller falls back so a wire still draws.

use super::geometry::{clean, Pt, Rect};
use super::graph::Grid;
use crate::ast::Side;
use std::cmp::{Ordering, Reverse};
use std::collections::BinaryHeap;

/// Per-bend cost, far larger than any diagram span, so a route never trades a
/// turn for shorter length — fewest bends first, length only to break ties.
const BEND: f64 = 100_000.0;

/// Route one wire from port `pa` (leaving `side_a`) to port `pb` (entering
/// `side_b`), around `obstacles`, keeping ≥ `clearance`. Ports are supplied by
/// `ports` (slot-assigned), not derived here, so a wire attaches wherever its
/// slot sits — not always the side midpoint. Other wires are not consulted: a
/// crossing penalty belongs in the global nudge pass (Phase 4), since a greedy
/// per-wire dodge only shuffles crossings onto later wires or folds a wire onto
/// itself (A5).
pub fn route(
    pa: Pt,
    side_a: Side,
    pb: Pt,
    side_b: Side,
    obstacles: &[Rect],
    clearance: f64,
) -> Option<Vec<Pt>> {
    let grid = Grid::build(obstacles, clearance, &[pa, pb]);
    let (ai, aj) = grid.index_of(pa)?;
    let (bi, bj) = grid.index_of(pb)?;
    astar(
        &grid,
        (ai, aj),
        outward(side_a),
        (bi, bj),
        inward(side_b),
        pa,
    )
}

/// Route a wire from a node back to itself (E3): an orthogonal loop that leaves
/// the `exit` side, steps ≥ `clearance` clear, and returns to the adjacent `ret`
/// side. Both ports sit a corner inset (= clearance) from the shared corner, so
/// the loop wraps that corner without touching the node. A `ret` side parallel to
/// `exit` is normalised to an adjacent one (the loop can only wrap a corner).
pub fn self_loop(r: Rect, exit: Side, ret: Side, clearance: f64) -> Vec<Pt> {
    let ret = adjacent_to(exit, ret);
    let d = clearance.max(1.0);
    let inset = clearance
        .min((r.max_x - r.min_x) / 2.0)
        .min((r.max_y - r.min_y) / 2.0)
        .max(0.0);

    let n = |s: Side| -> Pt {
        match s {
            Side::Right => (1.0, 0.0),
            Side::Left => (-1.0, 0.0),
            Side::Top => (0.0, -1.0),
            Side::Bottom => (0.0, 1.0),
        }
    };
    let vertical = |s: Side| matches!(s, Side::Left | Side::Right);
    let side_x = |s: Side| {
        if matches!(s, Side::Right) {
            r.max_x
        } else {
            r.min_x
        }
    };
    let side_y = |s: Side| {
        if matches!(s, Side::Bottom) {
            r.max_y
        } else {
            r.min_y
        }
    };

    // The corner where the exit and return sides meet, then both ports inset
    // along their sides from it, stepped out by `d`, joined past the corner.
    let k = if vertical(exit) {
        (side_x(exit), side_y(ret))
    } else {
        (side_x(ret), side_y(exit))
    };
    let (nex, ney) = n(exit);
    let (nrx, nry) = n(ret);
    let p_e = (k.0 - nrx * inset, k.1 - nry * inset);
    let p_r = (k.0 - nex * inset, k.1 - ney * inset);
    let a = (p_e.0 + nex * d, p_e.1 + ney * d);
    let c = (p_r.0 + nrx * d, p_r.1 + nry * d);
    let b = (k.0 + nex * d + nrx * d, k.1 + ney * d + nry * d);
    clean(vec![p_e, a, b, c, p_r])
}

/// A return side must be perpendicular to the exit side (a loop wraps a corner);
/// a parallel one is replaced by the default adjacent side.
fn adjacent_to(exit: Side, ret: Side) -> Side {
    let parallel =
        matches!(exit, Side::Left | Side::Right) == matches!(ret, Side::Left | Side::Right);
    if !parallel {
        ret
    } else if matches!(exit, Side::Left | Side::Right) {
        Side::Top
    } else {
        Side::Right
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Dir {
    E,
    W,
    N,
    S,
}

const DIRS: [Dir; 4] = [Dir::E, Dir::W, Dir::N, Dir::S];

impl Dir {
    fn idx(self) -> usize {
        match self {
            Dir::E => 0,
            Dir::W => 1,
            Dir::N => 2,
            Dir::S => 3,
        }
    }

    fn opposite(self) -> Dir {
        match self {
            Dir::E => Dir::W,
            Dir::W => Dir::E,
            Dir::N => Dir::S,
            Dir::S => Dir::N,
        }
    }
}

/// Outward normal of a side. SVG +y points down, so Top is −y (North).
fn outward(side: Side) -> Dir {
    match side {
        Side::Right => Dir::E,
        Side::Left => Dir::W,
        Side::Top => Dir::N,
        Side::Bottom => Dir::S,
    }
}

/// A wire arrives at its target travelling *into* the side — the opposite of its
/// outward normal — which keeps the final segment perpendicular (A2).
fn inward(side: Side) -> Dir {
    outward(side).opposite()
}

fn astar(
    grid: &Grid,
    start: (usize, usize),
    start_dir: Dir,
    goal: (usize, usize),
    goal_dir: Dir,
    pa: Pt,
) -> Option<Vec<Pt>> {
    let (nx, ny) = (grid.nx(), grid.ny());
    let sid = |i: usize, j: usize, d: Dir| (j * nx + i) * 4 + d.idx();
    let nstates = nx * ny * 4;

    const NONE: usize = usize::MAX;
    const START: usize = usize::MAX - 1;
    let mut g = vec![f64::INFINITY; nstates];
    let mut came = vec![NONE; nstates];
    let mut closed = vec![false; nstates];
    let mut open: BinaryHeap<Reverse<(OrdF, usize)>> = BinaryHeap::new();

    // The forced first step launches perpendicular to the source side (A2).
    let (qi, qj) = step(start.0, start.1, start_dir, grid)?;
    let q = grid.point(qi, qj);
    if !grid.edge_free(pa, q) {
        return None; // boxed in at the port
    }
    let s0 = sid(qi, qj, start_dir);
    g[s0] = dist(pa, q);
    came[s0] = START;
    open.push(Reverse((OrdF(g[s0] + heuristic(grid, qi, qj, goal)), s0)));

    while let Some(Reverse((_, cur))) = open.pop() {
        if closed[cur] {
            continue;
        }
        closed[cur] = true;
        let (i, j, dir) = decode(cur, nx);
        if (i, j) == goal && dir == goal_dir {
            return Some(reconstruct(grid, &came, cur, nx, pa, START));
        }
        let p = grid.point(i, j);
        for d in DIRS {
            if d == dir.opposite() {
                continue; // no 180° reversal
            }
            let Some((ni, nj)) = step(i, j, d, grid) else {
                continue;
            };
            let next = sid(ni, nj, d);
            if closed[next] || !grid.edge_free(p, grid.point(ni, nj)) {
                continue;
            }
            let turn = if d == dir { 0.0 } else { BEND };
            let tentative = g[cur] + dist(p, grid.point(ni, nj)) + turn;
            if tentative + 1e-9 < g[next] {
                g[next] = tentative;
                came[next] = cur;
                open.push(Reverse((
                    OrdF(tentative + heuristic(grid, ni, nj, goal)),
                    next,
                )));
            }
        }
    }
    None
}

fn reconstruct(
    grid: &Grid,
    came: &[usize],
    goal: usize,
    nx: usize,
    pa: Pt,
    start: usize,
) -> Vec<Pt> {
    let mut pts = Vec::new();
    let mut cur = goal;
    loop {
        let (i, j, _) = decode(cur, nx);
        pts.push(grid.point(i, j));
        match came[cur] {
            c if c == start => break,
            c => cur = c,
        }
    }
    pts.push(pa);
    pts.reverse();
    clean(pts)
}

fn step(i: usize, j: usize, d: Dir, grid: &Grid) -> Option<(usize, usize)> {
    match d {
        Dir::E if i + 1 < grid.nx() => Some((i + 1, j)),
        Dir::W if i > 0 => Some((i - 1, j)),
        Dir::N if j > 0 => Some((i, j - 1)), // ys ascend, North is −y
        Dir::S if j + 1 < grid.ny() => Some((i, j + 1)),
        _ => None,
    }
}

fn decode(sid: usize, nx: usize) -> (usize, usize, Dir) {
    let dir = match sid % 4 {
        0 => Dir::E,
        1 => Dir::W,
        2 => Dir::N,
        _ => Dir::S,
    };
    let cell = sid / 4;
    (cell % nx, cell / nx, dir)
}

fn heuristic(grid: &Grid, i: usize, j: usize, goal: (usize, usize)) -> f64 {
    let p = grid.point(i, j);
    let t = grid.point(goal.0, goal.1);
    (p.0 - t.0).abs() + (p.1 - t.1).abs()
}

fn dist(a: Pt, b: Pt) -> f64 {
    (a.0 - b.0).abs() + (a.1 - b.1).abs()
}

/// Total-ordered f64 wrapper so the priority queue is deterministic.
#[derive(PartialEq)]
struct OrdF(f64);
impl Eq for OrdF {}
impl PartialOrd for OrdF {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}
impl Ord for OrdF {
    fn cmp(&self, other: &Self) -> Ordering {
        self.0.total_cmp(&other.0)
    }
}

#[cfg(test)]
mod tests {
    use super::super::geometry::rect_penetrated_by;
    use super::*;

    fn rect(min_x: f64, min_y: f64, max_x: f64, max_y: f64) -> Rect {
        Rect {
            min_x,
            min_y,
            max_x,
            max_y,
        }
    }

    #[test]
    fn self_loop_wraps_the_corner_orthogonally() {
        // A wire from a node back to itself loops out the right side and back to
        // the top (E3 defaults): orthogonal, perpendicular at both ports, and
        // never inside the node.
        let r = rect(0.0, 0.0, 40.0, 40.0);
        let path = self_loop(r, Side::Right, Side::Top, 16.0);
        assert!(path.len() >= 4, "a loop needs several bends, got {path:?}");
        assert!(
            (path.first().unwrap().0 - 40.0).abs() < 1e-9,
            "exits the right edge"
        );
        assert!(
            (path.last().unwrap().1 - 0.0).abs() < 1e-9,
            "returns to the top edge"
        );
        for &(x, y) in &path {
            assert!(
                x >= 40.0 - 1e-9 || y <= 0.0 + 1e-9,
                "vertex ({x},{y}) is inside the node"
            );
        }
        // orthogonal: each segment axis-aligned, alternating with the last
        let mut prev_h: Option<bool> = None;
        for w in path.windows(2) {
            let h = (w[0].1 - w[1].1).abs() < 1e-9;
            let v = (w[0].0 - w[1].0).abs() < 1e-9;
            assert!(h ^ v, "segment {:?}->{:?} not axis-aligned", w[0], w[1]);
            assert_ne!(prev_h, Some(h), "two same-orientation segments in a row");
            prev_h = Some(h);
        }
    }

    #[test]
    fn clear_shot_is_a_straight_segment() {
        let a = rect(0.0, 0.0, 40.0, 40.0);
        let b = rect(100.0, 0.0, 140.0, 40.0);
        let path = route(
            a.port(Side::Right),
            Side::Right,
            b.port(Side::Left),
            Side::Left,
            &[],
            16.0,
        )
        .unwrap();
        assert_eq!(path, vec![(40.0, 20.0), (100.0, 20.0)]);
    }

    #[test]
    fn routes_around_an_obstacle_keeping_clearance() {
        let a = rect(0.0, 0.0, 40.0, 40.0);
        let blocker = rect(70.0, 0.0, 110.0, 40.0);
        let b = rect(140.0, 0.0, 180.0, 40.0);
        let path = route(
            a.port(Side::Right),
            Side::Right,
            b.port(Side::Left),
            Side::Left,
            &[blocker],
            16.0,
        )
        .unwrap();

        assert!(path.len() > 2, "a straight line would pierce the blocker");
        let infl = blocker.inflate(16.0);
        for w in path.windows(2) {
            assert!(
                !rect_penetrated_by(blocker, (w[0], w[1])),
                "must not pierce the blocker"
            );
            assert!(
                !rect_penetrated_by(infl, (w[0], w[1])),
                "must keep ≥ clearance"
            );
        }
        // perpendicular launch and arrival at the ports
        assert_eq!(path[0], (40.0, 20.0));
        assert_eq!(*path.last().unwrap(), (140.0, 20.0));
    }
}
