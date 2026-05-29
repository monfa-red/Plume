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

    /// Grow the rectangle by `pad` on every side — an obstacle inflated by
    /// `clearance` is the region a wire must stay out of.
    pub fn inflate(self, pad: f64) -> Self {
        Self {
            min_x: self.min_x - pad,
            min_y: self.min_y - pad,
            max_x: self.max_x + pad,
            max_y: self.max_y + pad,
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

// ───────────────── axis-aligned segment & box math ─────────────────
//
// Wire segments are always axis-aligned, so each is exactly its own bounding box
// and distances reduce to interval arithmetic. The validator (and later the
// router) share these primitives so "how far apart" is computed one way only.

/// One axis-aligned segment between two bends.
pub type Seg = (Pt, Pt);

/// `Some(true)` = horizontal, `Some(false)` = vertical, `None` = zero-length or
/// (never, for our routes) diagonal.
pub fn orient(s: Seg) -> Option<bool> {
    let ((ax, ay), (bx, by)) = s;
    match (close(ay, by), close(ax, bx)) {
        (true, false) => Some(true),
        (false, true) => Some(false),
        _ => None,
    }
}

/// Do the closed intervals `[a0,a1]` and `[b0,b1]` overlap on more than a point?
pub fn range_overlap(a0: f64, a1: f64, b0: f64, b1: f64) -> bool {
    let lo = a0.min(a1).max(b0.min(b1));
    let hi = a0.max(a1).min(b0.max(b1));
    hi - lo > EPS
}

/// Is `t` within `[a,b]` (order-free), with an epsilon of slack?
pub fn within(t: f64, a: f64, b: f64) -> bool {
    t >= a.min(b) - EPS && t <= a.max(b) + EPS
}

/// Two same-orientation segments lying on one line with overlapping extent.
pub fn collinear_overlap(a: Seg, b: Seg) -> bool {
    let (((ax0, ay0), (ax1, ay1)), ((bx0, by0), (bx1, by1))) = (a, b);
    match (orient(a), orient(b)) {
        (Some(true), Some(true)) => close(ay0, by0) && range_overlap(ax0, ax1, bx0, bx1),
        (Some(false), Some(false)) => close(ax0, bx0) && range_overlap(ay0, ay1, by0, by1),
        _ => false,
    }
}

/// True if two axis-aligned segments meet — a perpendicular crossing or a
/// collinear overlap.
pub fn segments_intersect(a: Seg, b: Seg) -> bool {
    match (orient(a), orient(b)) {
        (Some(x), Some(y)) if x == y => collinear_overlap(a, b),
        (Some(_), Some(_)) => {
            let (h, v) = if orient(a) == Some(true) {
                (a, b)
            } else {
                (b, a)
            };
            let ((hx0, hy), (hx1, _)) = h;
            let ((vx, vy0), (_, vy1)) = v;
            within(vx, hx0, hx1) && within(hy, vy0, vy1)
        }
        _ => false,
    }
}

/// A segment as its (degenerate) bounding box.
pub fn seg_box(s: Seg) -> Rect {
    let ((ax, ay), (bx, by)) = s;
    Rect {
        min_x: ax.min(bx),
        min_y: ay.min(by),
        max_x: ax.max(bx),
        max_y: ay.max(by),
    }
}

/// Euclidean distance between two axis-aligned rectangles — 0 if they touch or
/// overlap, else the straight gap between them.
pub fn boxes_distance(a: Rect, b: Rect) -> f64 {
    let gap_x = (a.min_x - b.max_x).max(b.min_x - a.max_x).max(0.0);
    let gap_y = (a.min_y - b.max_y).max(b.min_y - a.max_y).max(0.0);
    (gap_x * gap_x + gap_y * gap_y).sqrt()
}

/// Distance from an (axis-aligned) segment to a rectangle.
pub fn seg_rect_distance(rect: Rect, seg: Seg) -> f64 {
    boxes_distance(rect, seg_box(seg))
}

/// Distance between two (axis-aligned) segments.
pub fn seg_seg_distance(a: Seg, b: Seg) -> f64 {
    boxes_distance(seg_box(a), seg_box(b))
}

/// Does the segment cross into the rectangle's open interior (not merely touch a
/// side)? This is the B1 "node overlap" test.
pub fn rect_penetrated_by(rect: Rect, seg: Seg) -> bool {
    let inside_x = |x: f64| rect.min_x + EPS < x && x < rect.max_x - EPS;
    let inside_y = |y: f64| rect.min_y + EPS < y && y < rect.max_y - EPS;
    let b = seg_box(seg);
    match orient(seg) {
        // horizontal at y=b.min_y: strictly between top/bottom, and its x-extent
        // reaches into the open interior
        Some(true) => inside_y(b.min_y) && b.max_x > rect.min_x + EPS && b.min_x < rect.max_x - EPS,
        Some(false) => {
            inside_x(b.min_x) && b.max_y > rect.min_y + EPS && b.min_y < rect.max_y - EPS
        }
        None => false,
    }
}

/// Do the two segments cross perpendicularly at a point interior to both? This
/// is a genuine B3 crossing — not a T-junction or a shared port.
pub fn perp_crossing(a: Seg, b: Seg) -> bool {
    let (h, v) = match (orient(a), orient(b)) {
        (Some(true), Some(false)) => (seg_box(a), seg_box(b)),
        (Some(false), Some(true)) => (seg_box(b), seg_box(a)),
        _ => return false, // parallel or degenerate
    };
    // They meet at (v.x, h.y); a true crossing is interior to both.
    h.min_x + EPS < v.min_x
        && v.min_x < h.max_x - EPS
        && v.min_y + EPS < h.min_y
        && h.min_y < v.max_y - EPS
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
    fn boxes_distance_is_zero_when_touching_or_overlapping() {
        let a = rect(0.0, 0.0, 10.0, 10.0);
        assert_eq!(boxes_distance(a, rect(10.0, 0.0, 20.0, 10.0)), 0.0); // edge-touch
        assert_eq!(boxes_distance(a, rect(5.0, 5.0, 15.0, 15.0)), 0.0); // overlap
    }

    #[test]
    fn boxes_distance_axis_gap() {
        let a = rect(0.0, 0.0, 10.0, 10.0);
        assert!((boxes_distance(a, rect(20.0, 0.0, 30.0, 10.0)) - 10.0).abs() < 1e-9);
    }

    #[test]
    fn boxes_distance_diagonal_gap() {
        let a = rect(0.0, 0.0, 10.0, 10.0);
        let d = boxes_distance(a, rect(20.0, 20.0, 30.0, 30.0));
        assert!((d - (200.0_f64).sqrt()).abs() < 1e-9, "got {d}");
    }

    #[test]
    fn penetration_distinguishes_pierce_from_graze() {
        let r = rect(0.0, 0.0, 10.0, 10.0);
        assert!(rect_penetrated_by(r, ((-5.0, 5.0), (15.0, 5.0)))); // pierces across
        assert!(rect_penetrated_by(r, ((5.0, -5.0), (5.0, 15.0)))); // pierces vertically
        assert!(!rect_penetrated_by(r, ((-5.0, 0.0), (15.0, 0.0)))); // runs along top edge
        assert!(!rect_penetrated_by(r, ((-5.0, 20.0), (15.0, 20.0)))); // clear of the box
    }

    #[test]
    fn perp_crossing_only_for_true_interior_crossings() {
        let horiz = ((0.0, 5.0), (10.0, 5.0));
        assert!(perp_crossing(horiz, ((5.0, 0.0), (5.0, 10.0)))); // X
        assert!(!perp_crossing(horiz, ((0.0, 5.0), (0.0, 10.0)))); // shares an endpoint (T)
        assert!(!perp_crossing(horiz, ((0.0, 0.0), (10.0, 0.0)))); // parallel
    }
}
