//! Orthogonal visibility grid for single-wire routing.
//!
//! "Interesting lines" (the libavoid model): a wire that must clear every
//! obstacle by `clearance` only ever needs to turn on the obstacles'
//! clearance-inflated edges, or at its own ports. We lay a sparse grid through
//! exactly those x / y lines. A grid edge is usable when it doesn't cross any
//! inflated obstacle's interior — so *any* path over the grid keeps ≥ clearance
//! from every obstacle (B1 + the wire-node half of B2 hold by construction).

use super::geometry::{rect_penetrated_by, Pt, Rect, EPS};

pub struct Grid {
    pub xs: Vec<f64>,
    pub ys: Vec<f64>,
    inflated: Vec<Rect>,
}

impl Grid {
    /// Build the grid from the obstacles (inflated by `clearance`) and the
    /// coordinates that must be reachable — the wire's two ports.
    pub fn build(obstacles: &[Rect], clearance: f64, ports: &[Pt]) -> Self {
        let inflated: Vec<Rect> = obstacles.iter().map(|r| r.inflate(clearance)).collect();
        let mut xs = Vec::with_capacity(inflated.len() * 2 + ports.len());
        let mut ys = Vec::with_capacity(inflated.len() * 2 + ports.len());
        for r in &inflated {
            xs.push(r.min_x);
            xs.push(r.max_x);
            ys.push(r.min_y);
            ys.push(r.max_y);
        }
        for p in ports {
            xs.push(p.0);
            ys.push(p.1);
        }
        // A turning line halfway between every pair of ports. Two endpoints on
        // facing sides at different slots need a jog line *in the gap between
        // them* to make a clean two-bend route; obstacle edges never fall there.
        for (i, p) in ports.iter().enumerate() {
            for q in &ports[i + 1..] {
                xs.push((p.0 + q.0) / 2.0);
                ys.push((p.1 + q.1) / 2.0);
            }
        }
        Self {
            xs: sorted_unique(xs),
            ys: sorted_unique(ys),
            inflated,
        }
    }

    pub fn nx(&self) -> usize {
        self.xs.len()
    }

    pub fn ny(&self) -> usize {
        self.ys.len()
    }

    pub fn point(&self, i: usize, j: usize) -> Pt {
        (self.xs[i], self.ys[j])
    }

    /// The grid indices of a point known to sit on a grid line (e.g. a port).
    pub fn index_of(&self, p: Pt) -> Option<(usize, usize)> {
        let i = self.xs.iter().position(|&x| (x - p.0).abs() < EPS)?;
        let j = self.ys.iter().position(|&y| (y - p.1).abs() < EPS)?;
        Some((i, j))
    }

    /// A grid edge is free when the straight segment between its endpoints
    /// enters no inflated obstacle's interior (running along a boundary is fine).
    pub fn edge_free(&self, a: Pt, b: Pt) -> bool {
        !self.inflated.iter().any(|r| rect_penetrated_by(*r, (a, b)))
    }
}

fn sorted_unique(mut v: Vec<f64>) -> Vec<f64> {
    v.sort_by(|a, b| a.total_cmp(b));
    v.dedup_by(|a, b| (*a - *b).abs() < EPS);
    v
}

#[cfg(test)]
mod tests {
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
    fn grid_brackets_obstacle_edges_and_ports() {
        let g = Grid::build(
            &[rect(0.0, 0.0, 40.0, 40.0)],
            10.0,
            &[(-50.0, 20.0), (90.0, 20.0)],
        );
        // inflated edges at ±(−10, 50); ports at x = −50, 90 and y = 20
        assert!(
            g.index_of((-50.0, 20.0)).is_some(),
            "source port is a grid node"
        );
        assert!(
            g.index_of((90.0, 20.0)).is_some(),
            "target port is a grid node"
        );
        assert!(
            g.index_of((-10.0, 50.0)).is_some(),
            "inflated corner is a grid node"
        );
    }

    #[test]
    fn edge_through_obstacle_blocked_along_boundary_free() {
        let g = Grid::build(&[rect(0.0, 0.0, 40.0, 40.0)], 10.0, &[]);
        assert!(
            !g.edge_free((-50.0, 20.0), (90.0, 20.0)),
            "a line through the inflated interior is blocked"
        );
        assert!(
            g.edge_free((-50.0, -10.0), (90.0, -10.0)),
            "running along the inflated top edge keeps exactly clearance"
        );
        assert!(
            g.edge_free((-50.0, -30.0), (90.0, -30.0)),
            "well clear is free"
        );
    }
}
