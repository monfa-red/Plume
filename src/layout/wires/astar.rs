//! A* over the visibility grid — shortest orthogonal path with a bend penalty.
//!
//! The search state is `(x-index, y-index, arrival-direction)`: tracking the
//! direction lets us charge `BEND` whenever the path turns, so routes prefer
//! fewer corners (spec §6 ranks fewer bends above raw length). A caller-
//! supplied `surcharge` adds a per-segment cost — used in Step 2.5 to push
//! wires off each other; pass `&|_, _| 0.0` for plain shortest-path.
//!
//! `dead_code` is allowed until the orchestrator wires this in (Step 2 Task 2.4).
#![allow(dead_code)]

use super::geometry::AbsBbox;
use super::grid::{edge_clear, Grid};
use std::cmp::{Ordering, Reverse};
use std::collections::{BTreeMap, BinaryHeap};

const EPS: f64 = 0.5;
/// Cost of a 90° turn, in pixel-equivalent units. Biases toward fewer bends.
const BEND: f64 = 20.0;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Dir {
    E,
    W,
    S,
    N,
}

impl Dir {
    fn code(self) -> u8 {
        match self {
            Dir::E => 0,
            Dir::W => 1,
            Dir::S => 2,
            Dir::N => 3,
        }
    }
}

const START: u8 = 4; // sentinel arrival-direction for the source node

/// Total-ordered f64 wrapper so the cost can key a `BinaryHeap`.
#[derive(PartialEq)]
struct Ord64(f64);
impl Eq for Ord64 {}
impl PartialOrd for Ord64 {
    fn partial_cmp(&self, o: &Self) -> Option<Ordering> {
        Some(self.cmp(o))
    }
}
impl Ord for Ord64 {
    fn cmp(&self, o: &Self) -> Ordering {
        self.0.total_cmp(&o.0)
    }
}

fn index_of(coords: &[f64], v: f64) -> Option<usize> {
    coords.iter().position(|c| (c - v).abs() < EPS)
}

fn manhattan(a: (f64, f64), b: (f64, f64)) -> f64 {
    (a.0 - b.0).abs() + (a.1 - b.1).abs()
}

/// Route from `src` to `tgt` (both must be grid coordinates) avoiding
/// `obstacles`. Returns the collapsed orthogonal polyline, or `None` if the
/// goal is unreachable or an endpoint is off-grid.
pub fn route(
    grid: &Grid,
    src: (f64, f64),
    tgt: (f64, f64),
    obstacles: &[AbsBbox],
    surcharge: &dyn Fn((f64, f64), (f64, f64)) -> f64,
) -> Option<Vec<(f64, f64)>> {
    let (si, sj) = (index_of(&grid.xs, src.0)?, index_of(&grid.ys, src.1)?);
    let (gi, gj) = (index_of(&grid.xs, tgt.0)?, index_of(&grid.ys, tgt.1)?);
    let coord = |i: usize, j: usize| (grid.xs[i], grid.ys[j]);
    if (si, sj) == (gi, gj) {
        return Some(vec![src]);
    }
    let goal = coord(gi, gj);

    // Memoization keyed by (i, j, arrival-dir) — BTreeMap keeps the router
    // deterministic (no HashMap iteration in any decision).
    let mut dist: BTreeMap<(usize, usize, u8), f64> = BTreeMap::new();
    let mut came: BTreeMap<(usize, usize, u8), (usize, usize, u8)> = BTreeMap::new();
    let mut heap: BinaryHeap<Reverse<(Ord64, usize, usize, u8)>> = BinaryHeap::new();

    dist.insert((si, sj, START), 0.0);
    heap.push(Reverse((
        Ord64(manhattan(coord(si, sj), goal)),
        si,
        sj,
        START,
    )));

    while let Some(Reverse((Ord64(_), i, j, dcode))) = heap.pop() {
        if (i, j) == (gi, gj) {
            return Some(reconstruct(&came, (i, j, dcode), grid));
        }
        let g = dist.get(&(i, j, dcode)).copied().unwrap_or(f64::INFINITY);
        let here = coord(i, j);
        for (ni, nj, ndir) in neighbors(i, j, grid) {
            let there = coord(ni, nj);
            if !edge_clear(here, there, obstacles) {
                continue;
            }
            let bend = if dcode != START && dcode != ndir.code() {
                BEND
            } else {
                0.0
            };
            let ng = g + manhattan(here, there) + bend + surcharge(here, there);
            let key = (ni, nj, ndir.code());
            if ng + EPS < dist.get(&key).copied().unwrap_or(f64::INFINITY) {
                dist.insert(key, ng);
                came.insert(key, (i, j, dcode));
                heap.push(Reverse((
                    Ord64(ng + manhattan(there, goal)),
                    ni,
                    nj,
                    ndir.code(),
                )));
            }
        }
    }
    None
}

fn neighbors(i: usize, j: usize, grid: &Grid) -> Vec<(usize, usize, Dir)> {
    let mut out = Vec::with_capacity(4);
    if i + 1 < grid.xs.len() {
        out.push((i + 1, j, Dir::E));
    }
    if i > 0 {
        out.push((i - 1, j, Dir::W));
    }
    if j + 1 < grid.ys.len() {
        out.push((i, j + 1, Dir::S));
    }
    if j > 0 {
        out.push((i, j - 1, Dir::N));
    }
    out
}

fn reconstruct(
    came: &BTreeMap<(usize, usize, u8), (usize, usize, u8)>,
    goal_state: (usize, usize, u8),
    grid: &Grid,
) -> Vec<(f64, f64)> {
    let coord = |i: usize, j: usize| (grid.xs[i], grid.ys[j]);
    let mut pts = vec![coord(goal_state.0, goal_state.1)];
    let mut cur = goal_state;
    while let Some(&prev) = came.get(&cur) {
        pts.push(coord(prev.0, prev.1));
        cur = prev;
    }
    pts.reverse();
    collapse(pts)
}

/// Drop collinear interior points so straight runs are single segments.
fn collapse(pts: Vec<(f64, f64)>) -> Vec<(f64, f64)> {
    if pts.len() < 3 {
        return pts;
    }
    let mut out = vec![pts[0]];
    for i in 1..pts.len() - 1 {
        let a = *out.last().unwrap();
        let b = pts[i];
        let c = pts[i + 1];
        let collinear = ((a.0 - b.0).abs() < EPS && (b.0 - c.0).abs() < EPS)
            || ((a.1 - b.1).abs() < EPS && (b.1 - c.1).abs() < EPS);
        if !collinear {
            out.push(b);
        }
    }
    out.push(*pts.last().unwrap());
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bx(x: f64, y: f64, w: f64, h: f64) -> AbsBbox {
        AbsBbox { x, y, w, h }
    }

    fn no_surcharge() -> impl Fn((f64, f64), (f64, f64)) -> f64 {
        |_, _| 0.0
    }

    fn orthogonal(path: &[(f64, f64)]) -> bool {
        path.windows(2)
            .all(|w| (w[0].0 - w[1].0).abs() < EPS || (w[0].1 - w[1].1).abs() < EPS)
    }

    #[test]
    fn straight_when_clear() {
        let world = bx(-100.0, -100.0, 200.0, 200.0);
        let grid = Grid::build(&[], world, &[], &[]);
        // y = 0 is the world midline; x = ±100 are world edges.
        let path = route(&grid, (-100.0, 0.0), (100.0, 0.0), &[], &no_surcharge()).unwrap();
        assert_eq!(path, vec![(-100.0, 0.0), (100.0, 0.0)]);
    }

    #[test]
    fn routes_around_a_box() {
        let world = bx(-100.0, -100.0, 200.0, 200.0);
        let obs = [bx(-10.0, -10.0, 20.0, 20.0)]; // blocks the y=0 straight shot
        let grid = Grid::build(&obs, world, &[], &[]);
        let path = route(&grid, (-100.0, 0.0), (100.0, 0.0), &obs, &no_surcharge()).unwrap();
        assert_eq!(path.first(), Some(&(-100.0, 0.0)));
        assert_eq!(path.last(), Some(&(100.0, 0.0)));
        assert!(orthogonal(&path), "not orthogonal: {path:?}");
        // every segment clears the obstacle
        assert!(
            path.windows(2).all(|w| edge_clear(w[0], w[1], &obs)),
            "path crosses obstacle: {path:?}"
        );
        // it had to detour, so more than a straight line
        assert!(path.len() > 2, "expected a detour: {path:?}");
    }

    #[test]
    fn none_when_endpoint_off_grid() {
        let world = bx(-100.0, -100.0, 200.0, 200.0);
        let grid = Grid::build(&[], world, &[], &[]);
        // x = 37 is not a grid line and no extra coord was supplied.
        assert!(route(&grid, (37.0, 0.0), (100.0, 0.0), &[], &no_surcharge()).is_none());
    }

    #[test]
    fn fewer_bends_preferred() {
        // src and tgt aligned on x; clear column straight down — should be a
        // single vertical segment, no bends.
        let world = bx(-100.0, -100.0, 200.0, 200.0);
        let grid = Grid::build(&[], world, &[], &[]);
        let path = route(&grid, (0.0, -100.0), (0.0, 100.0), &[], &no_surcharge()).unwrap();
        assert_eq!(path, vec![(0.0, -100.0), (0.0, 100.0)]);
    }
}
