//! The routing validator — checks routed wires against WIRING.md's invariants
//! (section A). It re-derives everything from the polylines and the scene, so it
//! is independent of the router that produced them. Later phases extend it to
//! the constraints (section B); the reserved Severity levels are for those.
#![allow(dead_code)]

use super::geometry::{close, EPS};
use super::scene::SceneIndex;
use crate::layout::ir::{PlacedNode, RoutedWire};
use crate::resolve::{AttrMap, VarTable};

type Pt = (f64, f64);
type Seg = (Pt, Pt);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Rule {
    Orthogonality, // A1
    Attachment,    // A2
    PerpCrossing,  // A3
    SelfCross,     // A5
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Severity {
    Invariant,
    Error,
    Warning,
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
            Rule::SelfCross => "A5",
        }
    }

    pub fn severity(self) -> Severity {
        Severity::Invariant
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
        check_orthogonality(w, &mut out);
        check_attachment(w, &index, &mut out);
        check_self_cross(w, &mut out);
    }
    check_crossings(wires, &mut out);
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

// A5 — a wire never crosses or overlaps itself (non-adjacent segments).
fn check_self_cross(w: &RoutedWire, out: &mut Vec<Violation>) {
    let segs: Vec<Seg> = w.path.windows(2).map(|s| (s[0], s[1])).collect();
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
fn check_crossings(wires: &[RoutedWire], out: &mut Vec<Violation>) {
    for i in 0..wires.len() {
        for j in (i + 1)..wires.len() {
            if shares_run(&wires[i], &wires[j]) {
                out.push(Violation {
                    rule: Rule::PerpCrossing,
                    wires: vec![label(&wires[i]), label(&wires[j])],
                    detail: "wires share a parallel run".into(),
                });
            }
        }
    }
}

fn shares_run(a: &RoutedWire, b: &RoutedWire) -> bool {
    for sa in a.path.windows(2) {
        for sb in b.path.windows(2) {
            if collinear_overlap((sa[0], sa[1]), (sb[0], sb[1])) {
                return true;
            }
        }
    }
    false
}

// ── axis-aligned segment helpers ──

/// `Some(true)` = horizontal, `Some(false)` = vertical, `None` = zero-length or diagonal.
fn orient(s: Seg) -> Option<bool> {
    let ((ax, ay), (bx, by)) = s;
    match (close(ay, by), close(ax, bx)) {
        (true, false) => Some(true),
        (false, true) => Some(false),
        _ => None,
    }
}

fn range_overlap(a0: f64, a1: f64, b0: f64, b1: f64) -> bool {
    let lo = a0.min(a1).max(b0.min(b1));
    let hi = a0.max(a1).min(b0.max(b1));
    hi - lo > EPS
}

fn within(t: f64, a: f64, b: f64) -> bool {
    t >= a.min(b) - EPS && t <= a.max(b) + EPS
}

/// Two same-orientation segments lying on the same line with overlapping extent.
fn collinear_overlap(a: Seg, b: Seg) -> bool {
    let (((ax0, ay0), (ax1, ay1)), ((bx0, by0), (bx1, by1))) = (a, b);
    match (orient(a), orient(b)) {
        (Some(true), Some(true)) => close(ay0, by0) && range_overlap(ax0, ax1, bx0, bx1),
        (Some(false), Some(false)) => close(ax0, bx0) && range_overlap(ay0, ay1, by0, by1),
        _ => false,
    }
}

/// True if two axis-aligned segments meet — perpendicular crossing or overlap.
fn segments_intersect(a: Seg, b: Seg) -> bool {
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
