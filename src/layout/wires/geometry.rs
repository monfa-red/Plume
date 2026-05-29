//! Orthogonal geometry for wire routing: edge ports, the dumb straight/L/Z
//! route, polyline cleanup, and point-along-path for label placement.

use crate::ast::Side;

pub type Pt = (f64, f64);

pub const EPS: f64 = 1e-6;

/// An axis-aligned rectangle in absolute scene coordinates.
#[derive(Clone, Copy, Debug)]
pub struct Rect {
    pub min_x: f64,
    pub min_y: f64,
    pub max_x: f64,
    pub max_y: f64,
}

impl Rect {
    pub fn center(&self) -> Pt {
        (
            (self.min_x + self.max_x) / 2.0,
            (self.min_y + self.max_y) / 2.0,
        )
    }

    /// The midpoint of one side — where a single wire attaches.
    pub fn port(&self, side: Side) -> Pt {
        let (cx, cy) = self.center();
        match side {
            Side::Left => (self.min_x, cy),
            Side::Right => (self.max_x, cy),
            Side::Top => (cx, self.min_y),
            Side::Bottom => (cx, self.max_y),
        }
    }
}

/// A side is "horizontal" if its outward normal runs along x (left/right), so
/// the segment touching it is a horizontal line.
pub fn is_horizontal(side: Side) -> bool {
    matches!(side, Side::Left | Side::Right)
}

/// Pick the facing edges for a wire between two rects, by relative geometry:
/// leave and enter on whichever axis dominates. (SVG +y is down.)
pub fn pick_edges(a: Rect, b: Rect) -> (Side, Side) {
    let (acx, acy) = a.center();
    let (bcx, bcy) = b.center();
    let (dx, dy) = (bcx - acx, bcy - acy);
    if dx.abs() >= dy.abs() {
        if dx >= 0.0 {
            (Side::Right, Side::Left)
        } else {
            (Side::Left, Side::Right)
        }
    } else if dy >= 0.0 {
        (Side::Bottom, Side::Top)
    } else {
        (Side::Top, Side::Bottom)
    }
}

/// The dumb router's route between two ports: leaves `p0` perpendicular to its
/// edge, arrives at `p1` perpendicular to its edge, with at most two bends
/// (straight / L / Z). Obstacles and other wires are ignored — that is later
/// phases' job. The result is cleaned so it is strictly orthogonal with no
/// zero-length or collinear vertices.
pub fn dumb_route(p0: Pt, s0: Side, p1: Pt, s1: Side) -> Vec<Pt> {
    let pts = match (is_horizontal(s0), is_horizontal(s1)) {
        // both leave horizontally → a vertical jog at the midpoint x
        (true, true) => {
            let xm = (p0.0 + p1.0) / 2.0;
            vec![p0, (xm, p0.1), (xm, p1.1), p1]
        }
        // both leave vertically → a horizontal jog at the midpoint y
        (false, false) => {
            let ym = (p0.1 + p1.1) / 2.0;
            vec![p0, (p0.0, ym), (p1.0, ym), p1]
        }
        // leave horizontal, arrive vertical → an L with the corner at (p1.x, p0.y)
        (true, false) => vec![p0, (p1.0, p0.1), p1],
        // leave vertical, arrive horizontal → an L with the corner at (p0.x, p1.y)
        (false, true) => vec![p0, (p0.0, p1.1), p1],
    };
    clean(pts)
}

/// Drop consecutive duplicate points and collinear midpoints, so every
/// remaining vertex is a real 90° bend.
pub fn clean(pts: Vec<Pt>) -> Vec<Pt> {
    let mut out: Vec<Pt> = Vec::with_capacity(pts.len());
    for p in pts {
        if out.last().is_some_and(|q| same(*q, p)) {
            continue;
        }
        out.push(p);
    }
    let mut i = 1;
    while out.len() >= 3 && i < out.len() - 1 {
        if collinear(out[i - 1], out[i], out[i + 1]) {
            out.remove(i);
        } else {
            i += 1;
        }
    }
    out
}

pub fn same(a: Pt, b: Pt) -> bool {
    (a.0 - b.0).abs() < EPS && (a.1 - b.1).abs() < EPS
}

fn collinear(a: Pt, b: Pt, c: Pt) -> bool {
    (close(a.0, b.0) && close(b.0, c.0)) || (close(a.1, b.1) && close(b.1, c.1))
}

pub fn close(a: f64, b: f64) -> bool {
    (a - b).abs() < EPS
}

fn dist(a: Pt, b: Pt) -> f64 {
    ((a.0 - b.0).powi(2) + (a.1 - b.1).powi(2)).sqrt()
}

pub fn length(path: &[Pt]) -> f64 {
    path.windows(2).map(|w| dist(w[0], w[1])).sum()
}

/// Point at fraction `t` (0..1) along the polyline, with the unit tangent there.
pub fn point_at(path: &[Pt], t: f64) -> (Pt, Pt) {
    match path {
        [] => ((0.0, 0.0), (1.0, 0.0)),
        [p] => (*p, (1.0, 0.0)),
        _ => {
            let target = t.clamp(0.0, 1.0) * length(path);
            let mut acc = 0.0;
            for w in path.windows(2) {
                let seg = dist(w[0], w[1]);
                if acc + seg >= target || seg < EPS {
                    let f = if seg < EPS { 0.0 } else { (target - acc) / seg };
                    let p = (
                        w[0].0 + (w[1].0 - w[0].0) * f,
                        w[0].1 + (w[1].1 - w[0].1) * f,
                    );
                    return (p, unit((w[1].0 - w[0].0, w[1].1 - w[0].1)));
                }
                acc += seg;
            }
            (path[path.len() - 1], (1.0, 0.0))
        }
    }
}

fn unit(v: Pt) -> Pt {
    let l = (v.0 * v.0 + v.1 * v.1).sqrt();
    if l < EPS {
        (1.0, 0.0)
    } else {
        (v.0 / l, v.1 / l)
    }
}
