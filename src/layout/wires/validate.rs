//! The routing validator — checks routed wires against WIRING.md.
//!
//! It re-derives everything from the polylines and the placed scene, so it is
//! independent of the router that produced them: any router, written any way,
//! is judged against the same rules. Section A invariants are absolute; the
//! section B constraints are reported with the severity WIRING assigns to a
//! relaxation (B1 → error, B2 → warning), and B3 crossings are counted as
//! ordinary output.

use super::geometry::{
    close, collinear_overlap, perp_crossing, rect_penetrated_by, seg_rect_distance,
    seg_seg_distance, segments_intersect, Pt, Seg, EPS,
};
use super::oracle;
use super::scene::{obstacles_for, SceneIndex};
use crate::layout::ir::{PlacedNode, RoutedWire};
use crate::resolve::{AttrMap, ShapeKind, VarTable};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Rule {
    // A — hard invariants, never violated.
    Orthogonality, // A1
    Attachment,    // A2
    PerpCrossing,  // A3
    SidesOnly,     // A4
    SelfCross,     // A5
    // B — constraints (flagged when relaxed) and the crossing metric.
    NodeOverlap, // B1
    Clearance,   // B2, wire ↔ node
    Separation,  // B2, wire ↔ wire
    Crossing,    // B3 — a normal, reported crossing, not a violation
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Severity {
    Invariant, // A — absolute
    Error,     // a relaxed B1 (node overlap)
    Warning,   // a relaxed B2 (sub-clearance / sub-separation)
    Info,      // B3 crossings — output, not a problem
}

#[derive(Clone, Debug)]
pub struct Violation {
    pub rule: Rule,
    pub wires: Vec<String>,
    pub detail: String,
}

impl Rule {
    pub fn id(self) -> &'static str {
        match self {
            Rule::Orthogonality => "A1",
            Rule::Attachment => "A2",
            Rule::PerpCrossing => "A3",
            Rule::SidesOnly => "A4",
            Rule::SelfCross => "A5",
            Rule::NodeOverlap => "B1",
            Rule::Clearance | Rule::Separation => "B2",
            Rule::Crossing => "B3",
        }
    }

    pub fn severity(self) -> Severity {
        match self {
            Rule::Orthogonality
            | Rule::Attachment
            | Rule::PerpCrossing
            | Rule::SidesOnly
            | Rule::SelfCross => Severity::Invariant,
            Rule::NodeOverlap => Severity::Error,
            Rule::Clearance | Rule::Separation => Severity::Warning,
            Rule::Crossing => Severity::Info,
        }
    }
}

pub fn validate_routing(
    nodes: &[PlacedNode],
    _scene_attrs: &AttrMap,
    wires: &[RoutedWire],
    _vars: &VarTable,
) -> Vec<Violation> {
    let index = SceneIndex::build(nodes);
    let mut out = Vec::new();
    for w in wires {
        check_orthogonality(w, &mut out); // A1
        check_attachment(w, &index, &mut out); // A2
        check_sides_only(w, &index, &mut out); // A4
        check_self_cross(w, &mut out); // A5
        check_node_clearance(w, nodes, &mut out); // B1, B2 (wire ↔ node)
    }
    check_shared_runs(wires, &mut out); // A3
    check_separation(wires, &mut out); // B2 (wire ↔ wire), B3
    out
}

fn label(w: &RoutedWire) -> String {
    format!("{}->{}", w.seg_from, w.seg_to)
}

fn push(out: &mut Vec<Violation>, rule: Rule, w: &RoutedWire, detail: &str) {
    out.push(Violation {
        rule,
        wires: vec![label(w)],
        detail: detail.to_string(),
    });
}

fn pair(out: &mut Vec<Violation>, rule: Rule, a: &RoutedWire, b: &RoutedWire, detail: String) {
    out.push(Violation {
        rule,
        wires: vec![label(a), label(b)],
        detail,
    });
}

/// A wire's polyline as its list of axis-aligned segments.
fn segments(w: &RoutedWire) -> Vec<Seg> {
    w.path.windows(2).map(|s| (s[0], s[1])).collect()
}

// ───────────────────────────── A — invariants ─────────────────────────────

// A1 — every segment axis-aligned and non-zero; consecutive segments meet at 90°.
fn check_orthogonality(w: &RoutedWire, out: &mut Vec<Violation>) {
    let p = &w.path;
    if p.len() < 2 {
        return push(
            out,
            Rule::Orthogonality,
            w,
            "wire has fewer than two points",
        );
    }
    let mut prev_h: Option<bool> = None;
    for s in p.windows(2) {
        let ((ax, ay), (bx, by)) = (s[0], s[1]);
        let h = close(ay, by);
        let v = close(ax, bx);
        if h && v {
            return push(out, Rule::Orthogonality, w, "zero-length segment");
        }
        if !h && !v {
            return push(out, Rule::Orthogonality, w, "non-orthogonal segment");
        }
        if prev_h == Some(h) {
            return push(out, Rule::Orthogonality, w, "collinear / redundant vertex");
        }
        prev_h = Some(h);
    }
}

// A2 — the segment touching a node leaves perpendicular to that side, ending on it.
fn check_attachment(w: &RoutedWire, index: &SceneIndex, out: &mut Vec<Violation>) {
    let p = &w.path;
    if p.len() < 2 {
        return;
    }
    check_end(w, &w.seg_from, p[0], p[1], index, out);
    let n = p.len();
    check_end(w, &w.seg_to, p[n - 1], p[n - 2], index, out);
}

fn check_end(
    w: &RoutedWire,
    path: &str,
    at: Pt,
    next: Pt,
    index: &SceneIndex,
    out: &mut Vec<Violation>,
) {
    let Some(r) = index.rect(path) else {
        return push(
            out,
            Rule::Attachment,
            w,
            &format!("endpoint node '{path}' not found in scene"),
        );
    };
    let (x, y) = at;
    let on_lr =
        (close(x, r.min_x) || close(x, r.max_x)) && y >= r.min_y - EPS && y <= r.max_y + EPS;
    let on_tb =
        (close(y, r.min_y) || close(y, r.max_y)) && x >= r.min_x - EPS && x <= r.max_x + EPS;
    if !on_lr && !on_tb {
        return push(
            out,
            Rule::Attachment,
            w,
            &format!("does not land on '{path}'s edge"),
        );
    }
    let touching_h = close(y, next.1); // horizontal touching segment
    let ok = if on_lr && !on_tb {
        touching_h // a left/right (vertical) edge needs a horizontal approach
    } else if on_tb && !on_lr {
        !touching_h
    } else {
        true // exactly on a corner — accept
    };
    if !ok {
        push(
            out,
            Rule::Attachment,
            w,
            &format!("non-perpendicular attachment at '{path}'"),
        );
    }
}

// A4 — wires attach to shape sides only, never to a text node.
fn check_sides_only(w: &RoutedWire, index: &SceneIndex, out: &mut Vec<Violation>) {
    for path in [&w.seg_from, &w.seg_to] {
        if index.shape(path) == Some(ShapeKind::Text) {
            push(
                out,
                Rule::SidesOnly,
                w,
                &format!("endpoint '{path}' is a text node"),
            );
        }
    }
}

// A5 — a wire never crosses or overlaps itself (non-adjacent segments).
fn check_self_cross(w: &RoutedWire, out: &mut Vec<Violation>) {
    let segs = segments(w);
    for i in 0..segs.len() {
        for j in (i + 2)..segs.len() {
            if segments_intersect(segs[i], segs[j]) {
                return push(out, Rule::SelfCross, w, "wire crosses itself");
            }
        }
    }
}

// A3 — two different wires may only cross perpendicularly; never share a run.
// (The fan-sibling trunk exemption arrives with the multi-wire phases.)
fn check_shared_runs(wires: &[RoutedWire], out: &mut Vec<Violation>) {
    let segs: Vec<Vec<Seg>> = wires.iter().map(segments).collect();
    for i in 0..wires.len() {
        for j in (i + 1)..wires.len() {
            let shares = segs[i]
                .iter()
                .any(|a| segs[j].iter().any(|b| collinear_overlap(*a, *b)));
            if shares {
                pair(
                    out,
                    Rule::PerpCrossing,
                    &wires[i],
                    &wires[j],
                    "wires share a parallel run".into(),
                );
            }
        }
    }
}

// ───────────────────────────── B — constraints ─────────────────────────────

// B1 / B2 — a wire never enters a node's interior, and stays `clearance` away.
fn check_node_clearance(w: &RoutedWire, nodes: &[PlacedNode], out: &mut Vec<Violation>) {
    let obstacles = obstacles_for(nodes, [&w.seg_from, &w.seg_to]);
    let clearance = oracle::clearance(&w.attrs);
    let segs = segments(w);
    for obs in &obstacles {
        if segs.iter().any(|s| rect_penetrated_by(*obs, *s)) {
            push(out, Rule::NodeOverlap, w, "wire crosses a node's interior");
        } else {
            let gap = segs
                .iter()
                .map(|s| seg_rect_distance(*obs, *s))
                .fold(f64::INFINITY, f64::min);
            if gap + EPS < clearance {
                push(
                    out,
                    Rule::Clearance,
                    w,
                    &format!("{gap:.1} from a node (< clearance {clearance:.0})"),
                );
            }
        }
    }
}

// B2 (wire ↔ wire) and B3 — wires keep `separation` apart, except where they
// cross perpendicularly. A crossing is exempt from B2 and merely counted (B3);
// shared parallel runs are A3's business, not double-counted here.
fn check_separation(wires: &[RoutedWire], out: &mut Vec<Violation>) {
    let segs: Vec<Vec<Seg>> = wires.iter().map(segments).collect();
    for i in 0..wires.len() {
        for j in (i + 1)..wires.len() {
            let separation = oracle::separation(&wires[i].attrs, &wires[j].attrs);
            let mut gap = f64::INFINITY;
            for a in &segs[i] {
                for b in &segs[j] {
                    if perp_crossing(*a, *b) {
                        pair(
                            out,
                            Rule::Crossing,
                            &wires[i],
                            &wires[j],
                            "perpendicular crossing".into(),
                        );
                    } else if !collinear_overlap(*a, *b) {
                        gap = gap.min(seg_seg_distance(*a, *b));
                    }
                }
            }
            if gap + EPS < separation {
                pair(
                    out,
                    Rule::Separation,
                    &wires[i],
                    &wires[j],
                    format!("{gap:.1} between wires (< separation {separation:.0})"),
                );
            }
        }
    }
}
