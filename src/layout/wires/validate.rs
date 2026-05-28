//! Independent routing validator — checks the contract R1–R6 from the
//! rules-spec against the final polylines. Shares no decision code with the
//! router: it re-derives every check from geometry, reusing only the scene
//! index (data) and the clearance oracle.
//!
//! See `docs/superpowers/specs/2026-05-28-wire-routing-rules-design.md`.

use super::geometry::AbsBbox;
use super::oracle;
use super::scene::SceneIndex;
use crate::layout::ir::{PlacedNode, RoutedWire};
use crate::resolve::{AttrMap, VarTable};

const EPS: f64 = 0.5;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Rule {
    Orthogonality,  // R1
    ShapeClearance, // R2
    WireSpacing,    // R3
    Crossing,       // R4
    Attachment,     // R5
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    Invariant,
    Error,
    Warning,
}

impl Rule {
    pub fn id(self) -> &'static str {
        match self {
            Rule::Orthogonality => "R1",
            Rule::ShapeClearance => "R2",
            Rule::WireSpacing => "R3",
            Rule::Crossing => "R4",
            Rule::Attachment => "R5",
        }
    }
    pub fn severity(self) -> Severity {
        match self {
            Rule::Orthogonality | Rule::Crossing | Rule::Attachment => Severity::Invariant,
            Rule::ShapeClearance => Severity::Error,
            Rule::WireSpacing => Severity::Warning,
        }
    }
}

#[derive(Debug, Clone)]
pub struct Violation {
    pub rule: Rule,
    pub detail: String,
}

fn order(a: f64, b: f64) -> (f64, f64) {
    if a <= b {
        (a, b)
    } else {
        (b, a)
    }
}

fn approx(a: f64, b: f64) -> bool {
    (a - b).abs() < EPS
}

/// Axis of an axis-aligned segment: `Some(true)` horizontal, `Some(false)`
/// vertical, `None` if degenerate (zero length) or diagonal.
fn axis(a: (f64, f64), b: (f64, f64)) -> Option<bool> {
    let dx = (a.0 - b.0).abs();
    let dy = (a.1 - b.1).abs();
    if dy < EPS && dx >= EPS {
        Some(true)
    } else if dx < EPS && dy >= EPS {
        Some(false)
    } else {
        None
    }
}

/// True if axis-aligned segment `a→b` has any point strictly inside `bx`.
fn segment_pierces_box(a: (f64, f64), b: (f64, f64), bx: &AbsBbox) -> bool {
    let (x_lo, x_hi) = order(a.0, b.0);
    let (y_lo, y_hi) = order(a.1, b.1);
    let x_overlap = x_lo < bx.right() - EPS && x_hi > bx.x + EPS;
    let y_overlap = y_lo < bx.bottom() - EPS && y_hi > bx.y + EPS;
    x_overlap && y_overlap
}

/// For a pair of segments from *different* wires, return a violation if they
/// run parallel within `sep` (R3) or overlap collinearly (R4). Perpendicular
/// crossings return `None` — they are legal by construction.
fn pair_violation(
    sa0: (f64, f64),
    sa1: (f64, f64),
    sb0: (f64, f64),
    sb1: (f64, f64),
    sep: f64,
) -> Option<Rule> {
    match (axis(sa0, sa1), axis(sb0, sb1)) {
        (Some(true), Some(true)) => {
            let dist = (sa0.1 - sb0.1).abs();
            let (axl, axh) = order(sa0.0, sa1.0);
            let (bxl, bxh) = order(sb0.0, sb1.0);
            let overlap = axl < bxh - EPS && bxl < axh - EPS;
            classify(overlap, dist, sep)
        }
        (Some(false), Some(false)) => {
            let dist = (sa0.0 - sb0.0).abs();
            let (ayl, ayh) = order(sa0.1, sa1.1);
            let (byl, byh) = order(sb0.1, sb1.1);
            let overlap = ayl < byh - EPS && byl < ayh - EPS;
            classify(overlap, dist, sep)
        }
        _ => None, // perpendicular (or a degenerate segment) — fine
    }
}

fn classify(overlap: bool, dist: f64, sep: f64) -> Option<Rule> {
    if !overlap {
        return None;
    }
    if dist <= EPS {
        Some(Rule::Crossing) // collinear overlap — illegal
    } else if dist < sep - EPS {
        Some(Rule::WireSpacing) // parallel, too close
    } else {
        None
    }
}

fn check_orthogonal(w: &RoutedWire, out: &mut Vec<Violation>) {
    let n = w.path.len();
    if n < 2 {
        return;
    }
    for win in w.path.windows(2) {
        if axis(win[0], win[1]).is_none() {
            out.push(Violation {
                rule: Rule::Orthogonality,
                detail: format!(
                    "{}->{}: non-orthogonal or zero-length segment {:?}->{:?}",
                    w.seg_from, w.seg_to, win[0], win[1]
                ),
            });
        }
    }
    // Interior vertices must be 90° turns (axis flips), never collinear.
    for i in 1..n - 1 {
        if let (Some(ax), Some(bx)) = (axis(w.path[i - 1], w.path[i]), axis(w.path[i], w.path[i + 1]))
        {
            if ax == bx {
                out.push(Violation {
                    rule: Rule::Orthogonality,
                    detail: format!(
                        "{}->{}: collinear/redundant vertex at {:?}",
                        w.seg_from, w.seg_to, w.path[i]
                    ),
                });
            }
        }
    }
}

fn check_attachment(w: &RoutedWire, scene: &SceneIndex, out: &mut Vec<Violation>) {
    let n = w.path.len();
    if n < 2 {
        return;
    }
    if let Some(s) = scene.lookup(&w.seg_from) {
        if !end_on_edge_perp(w.path[0], w.path[1], &s.bbox) {
            out.push(Violation {
                rule: Rule::Attachment,
                detail: format!(
                    "{}->{}: source end {:?} not on a perpendicular edge of '{}'",
                    w.seg_from, w.seg_to, w.path[0], w.seg_from
                ),
            });
        }
    }
    if let Some(t) = scene.lookup(&w.seg_to) {
        if !end_on_edge_perp(w.path[n - 1], w.path[n - 2], &t.bbox) {
            out.push(Violation {
                rule: Rule::Attachment,
                detail: format!(
                    "{}->{}: target end {:?} not on a perpendicular edge of '{}'",
                    w.seg_from, w.seg_to, w.path[n - 1], w.seg_to
                ),
            });
        }
    }
}

/// True if `p` lies on an edge of `b` and the segment `p→next` leaves that
/// edge perpendicularly (horizontal off a left/right edge, vertical off a
/// top/bottom edge).
fn end_on_edge_perp(p: (f64, f64), next: (f64, f64), b: &AbsBbox) -> bool {
    let on_v_edge =
        (approx(p.0, b.x) || approx(p.0, b.right())) && p.1 >= b.y - EPS && p.1 <= b.bottom() + EPS;
    let on_h_edge = (approx(p.1, b.y) || approx(p.1, b.bottom()))
        && p.0 >= b.x - EPS
        && p.0 <= b.right() + EPS;
    match axis(p, next) {
        Some(true) => on_v_edge,  // horizontal segment ⟂ a vertical edge
        Some(false) => on_h_edge, // vertical segment ⟂ a horizontal edge
        None => false,
    }
}

fn check_shape_clearance(w: &RoutedWire, scene: &SceneIndex, out: &mut Vec<Violation>) {
    let obstacles = scene.raw_obstacles(&w.seg_from, &w.seg_to);
    for win in w.path.windows(2) {
        for (path, bbox) in &obstacles {
            let c = oracle::shape_clearance(scene, path);
            // No-go zone = bbox grown by clearance. `-EPS` so a segment lying
            // exactly at distance `c` is accepted, not flagged.
            let zone = bbox.inflate((c - EPS).max(0.0));
            if segment_pierces_box(win[0], win[1], &zone) {
                out.push(Violation {
                    rule: Rule::ShapeClearance,
                    detail: format!(
                        "{}->{}: segment {:?}->{:?} within {} of shape '{}'",
                        w.seg_from, w.seg_to, win[0], win[1], c, path
                    ),
                });
            }
        }
    }
}

fn check_pair(a: &RoutedWire, b: &RoutedWire, vars: &VarTable, out: &mut Vec<Violation>) {
    if a.decl_span == b.decl_span {
        return; // same declaration: chain links or fan-out siblings — exempt
    }
    let sep = oracle::wire_separation(oracle::wire_gap(a, vars), oracle::wire_gap(b, vars));
    for sa in a.path.windows(2) {
        for sb in b.path.windows(2) {
            if let Some(rule) = pair_violation(sa[0], sa[1], sb[0], sb[1], sep) {
                out.push(Violation {
                    rule,
                    detail: format!(
                        "{}->{} vs {}->{}: {} (sep {})",
                        a.seg_from,
                        a.seg_to,
                        b.seg_from,
                        b.seg_to,
                        rule.id(),
                        sep
                    ),
                });
            }
        }
    }
}

/// Validate the full routing of a laid-out scene against the contract.
pub fn validate_routing(
    nodes: &[PlacedNode],
    scene_attrs: &AttrMap,
    wires: &[RoutedWire],
    vars: &VarTable,
) -> Vec<Violation> {
    let scene = SceneIndex::build(nodes, scene_attrs);
    let mut out = Vec::new();
    for w in wires {
        check_orthogonal(w, &mut out);
        check_attachment(w, &scene, &mut out);
        check_shape_clearance(w, &scene, &mut out);
    }
    for i in 0..wires.len() {
        for j in (i + 1)..wires.len() {
            check_pair(&wires[i], &wires[j], vars, &mut out);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bx(x: f64, y: f64, w: f64, h: f64) -> AbsBbox {
        AbsBbox { x, y, w, h }
    }

    #[test]
    fn axis_classifies_segments() {
        assert_eq!(axis((0.0, 0.0), (10.0, 0.0)), Some(true));
        assert_eq!(axis((0.0, 0.0), (0.0, 10.0)), Some(false));
        assert_eq!(axis((0.0, 0.0), (0.0, 0.0)), None);
        assert_eq!(axis((0.0, 0.0), (10.0, 10.0)), None);
    }

    #[test]
    fn pierces_box_detects_crossing() {
        let b = bx(-5.0, -5.0, 10.0, 10.0); // covers [-5,5]^2
        assert!(segment_pierces_box((-20.0, 0.0), (20.0, 0.0), &b));
        assert!(!segment_pierces_box((-20.0, 50.0), (20.0, 50.0), &b));
    }

    #[test]
    fn parallel_too_close_is_r3() {
        // two horizontal segments 6 apart, separation 16, x-overlap
        assert_eq!(
            pair_violation((0.0, 0.0), (50.0, 0.0), (10.0, 6.0), (60.0, 6.0), 16.0),
            Some(Rule::WireSpacing)
        );
    }

    #[test]
    fn parallel_at_separation_is_clean() {
        assert_eq!(
            pair_violation((0.0, 0.0), (50.0, 0.0), (10.0, 16.0), (60.0, 16.0), 16.0),
            None
        );
    }

    #[test]
    fn collinear_overlap_is_r4() {
        assert_eq!(
            pair_violation((0.0, 0.0), (50.0, 0.0), (25.0, 0.0), (75.0, 0.0), 16.0),
            Some(Rule::Crossing)
        );
    }

    #[test]
    fn perpendicular_crossing_is_clean() {
        assert_eq!(
            pair_violation((0.0, 0.0), (50.0, 0.0), (25.0, -20.0), (25.0, 20.0), 16.0),
            None
        );
    }

    #[test]
    fn no_overlap_no_violation() {
        // parallel and close, but x-extents don't overlap
        assert_eq!(
            pair_violation((0.0, 0.0), (10.0, 0.0), (50.0, 4.0), (60.0, 4.0), 16.0),
            None
        );
    }

    #[test]
    fn attachment_accepts_perpendicular_exit() {
        let b = bx(0.0, 0.0, 100.0, 40.0);
        // right-edge midpoint, heading further right (horizontal) — perpendicular
        assert!(end_on_edge_perp((100.0, 20.0), (130.0, 20.0), &b));
        // on the right edge but heading vertical (tangent) — not perpendicular
        assert!(!end_on_edge_perp((100.0, 20.0), (100.0, 50.0), &b));
        // not on any edge
        assert!(!end_on_edge_perp((50.0, 20.0), (80.0, 20.0), &b));
    }
}
