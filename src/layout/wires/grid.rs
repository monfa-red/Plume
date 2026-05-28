//! Orthogonal visibility grid for wire routing.
//!
//! Candidate routing lines are derived from scene geometry: each obstacle's
//! clearance-inflated edges, the world frame, any extra coordinates the caller
//! supplies (endpoint attachment points), and the midlines between adjacent
//! lines — channel centres, where tidy bends land. A\* travels the
//! intersections of these lines, so every bend sits on a meaningful coordinate
//! shared across wires.

use super::geometry::AbsBbox;

const EPS: f64 = 0.5;

pub struct Grid {
    /// Sorted, de-duplicated candidate x lines (with channel midlines).
    pub xs: Vec<f64>,
    /// Sorted, de-duplicated candidate y lines (with channel midlines).
    pub ys: Vec<f64>,
}

impl Grid {
    /// Build the candidate lattice. `obstacles` are already inflated by each
    /// shape's clearance; `world` bounds the routable plane; `extra_xs` /
    /// `extra_ys` add caller coordinates (endpoint attachment points).
    pub fn build(
        obstacles: &[AbsBbox],
        world: AbsBbox,
        extra_xs: &[f64],
        extra_ys: &[f64],
    ) -> Grid {
        let mut xs = vec![world.x, world.right()];
        let mut ys = vec![world.y, world.bottom()];
        for o in obstacles {
            xs.push(o.x);
            xs.push(o.right());
            ys.push(o.y);
            ys.push(o.bottom());
        }
        xs.extend_from_slice(extra_xs);
        ys.extend_from_slice(extra_ys);
        Grid {
            xs: finish_axis(xs),
            ys: finish_axis(ys),
        }
    }
}

/// Sort, de-duplicate (within `EPS`), then insert the midpoint between each
/// pair of adjacent coordinates so wires can bend at channel centres.
fn finish_axis(mut v: Vec<f64>) -> Vec<f64> {
    v.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    v.dedup_by(|a, b| (*a - *b).abs() < EPS);
    let mut out = Vec::with_capacity(v.len().saturating_mul(2));
    for i in 0..v.len() {
        out.push(v[i]);
        if i + 1 < v.len() {
            let mid = (v[i] + v[i + 1]) / 2.0;
            if (mid - v[i]).abs() > EPS && (v[i + 1] - mid).abs() > EPS {
                out.push(mid);
            }
        }
    }
    out
}

/// True if the axis-aligned segment `a→b` stays clear of every obstacle.
/// Obstacles are already clearance-inflated; the wire's own endpoint shapes
/// (and their ancestors) must be excluded by the caller.
pub fn edge_clear(a: (f64, f64), b: (f64, f64), obstacles: &[AbsBbox]) -> bool {
    !obstacles.iter().any(|o| pierces(a, b, o))
}

/// True if axis-aligned segment `a→b` has any point strictly inside `o`.
fn pierces(a: (f64, f64), b: (f64, f64), o: &AbsBbox) -> bool {
    let (x_lo, x_hi) = order(a.0, b.0);
    let (y_lo, y_hi) = order(a.1, b.1);
    x_lo < o.right() - EPS && x_hi > o.x + EPS && y_lo < o.bottom() - EPS && y_hi > o.y + EPS
}

fn order(a: f64, b: f64) -> (f64, f64) {
    if a <= b {
        (a, b)
    } else {
        (b, a)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bx(x: f64, y: f64, w: f64, h: f64) -> AbsBbox {
        AbsBbox { x, y, w, h }
    }

    #[test]
    fn build_includes_obstacle_edges_and_world() {
        let world = bx(-100.0, -100.0, 200.0, 200.0); // [-100,100]^2
        let obs = [bx(0.0, 0.0, 20.0, 20.0)]; // edges at 0 and 20
        let g = Grid::build(&obs, world, &[], &[]);
        for want in [-100.0, 0.0, 20.0, 100.0] {
            assert!(
                g.xs.iter().any(|x| (x - want).abs() < EPS),
                "missing x {want} in {:?}",
                g.xs
            );
        }
        // sorted ascending
        assert!(g.xs.windows(2).all(|w| w[0] <= w[1]));
    }

    #[test]
    fn build_inserts_channel_midlines() {
        let world = bx(0.0, 0.0, 100.0, 100.0);
        let g = Grid::build(&[], world, &[], &[]);
        // between world edges 0 and 100, the midline 50 should appear.
        assert!(g.xs.iter().any(|x| (x - 50.0).abs() < EPS), "{:?}", g.xs);
    }

    #[test]
    fn build_merges_extra_coords() {
        let world = bx(0.0, 0.0, 100.0, 100.0);
        let g = Grid::build(&[], world, &[33.0], &[77.0]);
        assert!(g.xs.iter().any(|x| (x - 33.0).abs() < EPS));
        assert!(g.ys.iter().any(|y| (y - 77.0).abs() < EPS));
    }

    #[test]
    fn edge_clear_detects_obstacle() {
        let obs = [bx(-10.0, -10.0, 20.0, 20.0)]; // [-10,10]^2
                                                  // horizontal segment through it
        assert!(!edge_clear((-50.0, 0.0), (50.0, 0.0), &obs));
        // horizontal segment above it
        assert!(edge_clear((-50.0, 50.0), (50.0, 50.0), &obs));
        // vertical segment beside it
        assert!(edge_clear((30.0, -50.0), (30.0, 50.0), &obs));
    }

    #[test]
    fn edge_clear_allows_grazing_the_boundary() {
        let obs = [bx(0.0, 0.0, 20.0, 20.0)];
        // segment running exactly along the right edge (x=20) is clear
        assert!(edge_clear((20.0, -10.0), (20.0, 30.0), &obs));
        // segment running exactly along the top edge (y=0) is clear
        assert!(edge_clear((-10.0, 0.0), (30.0, 0.0), &obs));
    }
}
