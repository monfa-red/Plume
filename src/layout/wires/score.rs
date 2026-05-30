//! The routing scorecard — the objective the side-search minimises.
//!
//! A lexicographic tuple, where strictly smaller is strictly better. It follows
//! WIRING's own priority: the constraints first (invariants A1–A5, B1 node overlap,
//! B2 wire-node and wire-wire clearance), then the objective (B3 crossings, B4
//! turns, B5 length). The first five come from the independent validator; turns and
//! length are summed from the polylines. Promoting **turns** to sit right after
//! crossings is what makes the search minimise bends among equal-crossing layouts.

use super::geometry::length;
use super::validate::{validate_routing, Rule, Severity};
use crate::layout::ir::{PlacedNode, RoutedWire};
use crate::resolve::{AttrMap, VarTable};

/// A comparison tuple, smaller strictly better (see [`score`] for the field order).
pub type Score = (usize, usize, usize, usize, usize, usize, usize);

/// The validator's raw contract counts for a routing, before they're arranged into a
/// comparison tuple. `a3` (shared parallel runs) is kept apart from the per-wire
/// invariants because the nudge pass resolves it, while A1/A2/A4/A5 it cannot.
struct Counts {
    inv: usize, // A1/A2/A4/A5 — per-wire invariants the nudge can't repair
    a3: usize,  // A3 shared parallel runs — the nudge separates these
    b1: usize,
    b2n: usize,
    b2w: usize, // wire-wire separation — the nudge separates these too
    crossings: usize,
}

fn counts(wires: &[RoutedWire], nodes: &[PlacedNode]) -> Counts {
    let vs = validate_routing(nodes, &AttrMap::new(), wires, &VarTable::new());
    let mut c = Counts {
        inv: 0,
        a3: 0,
        b1: 0,
        b2n: 0,
        b2w: 0,
        crossings: 0,
    };
    for v in &vs {
        match v.rule {
            Rule::NodeOverlap => c.b1 += 1,
            Rule::Clearance => c.b2n += 1,
            Rule::Separation => c.b2w += 1,
            Rule::Crossing => c.crossings += 1,
            Rule::PerpCrossing => c.a3 += 1,
            r if r.severity() == Severity::Invariant => c.inv += 1,
            _ => {}
        }
    }
    c
}

/// The routing objective the side-search and the keep-better both minimise, ordered
/// most-important first:
///
/// 1. **hard per-wire invariants** (A1/A2/A4/A5) and **B1** node overlap — absolute;
/// 2. **B2n** endpoint skim — side-selection's to avoid, the nudge can't;
/// 3. **B3 crossings** then **B4 turns** — the visible quality the user cares about;
/// 4. **A3 shared runs + B2w sub-separation** — these the *nudge* resolves (and are
///    flagged, not hard), so they sit **below** crossings: the search must not split
///    a clean bundle (raising crossings) just to spare a sub-separation the nudge
///    would clear or, at worst, flag;
/// 5. **B5 length** — the final tie-break.
///
/// Ranking crossings above A3/B2w is the key: a route-only score over-weights the
/// shared runs the nudge fixes, so the proxy [`super::select`] nudges before scoring,
/// making this the real shipped geometry.
pub fn score(wires: &[RoutedWire], nodes: &[PlacedNode]) -> Score {
    let c = counts(wires, nodes);
    (
        c.inv,
        c.b1,
        c.b2n,
        c.crossings,
        turns(wires),
        c.a3 + c.b2w,
        length_px(wires),
    )
}

/// Total 90° bends across all wires (B4): a polyline of `n` points has `n - 2`.
pub fn turns(wires: &[RoutedWire]) -> usize {
    wires.iter().map(|w| w.path.len().saturating_sub(2)).sum()
}

/// Total polyline length in whole px (B5 / a determinism-safe B6 tidiness proxy).
pub fn length_px(wires: &[RoutedWire]) -> usize {
    wires.iter().map(|w| length(&w.path).round() as usize).sum()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::resolve::Markers;
    use crate::span::Span;

    fn wire(path: Vec<(f64, f64)>) -> RoutedWire {
        RoutedWire {
            path,
            markers: Markers::default(),
            attrs: AttrMap::new(),
            texts: Vec::new(),
            data_from: "a".into(),
            data_to: "b".into(),
            seg_from: "a".into(),
            seg_to: "b".into(),
            decl_span: Span::empty(),
            fan_from: None,
            fan_to: None,
        }
    }

    #[test]
    fn turns_and_length_are_summed_across_wires() {
        let straight = wire(vec![(0.0, 0.0), (10.0, 0.0)]); // 0 turns, len 10
        let ell = wire(vec![(0.0, 0.0), (10.0, 0.0), (10.0, 10.0)]); // 1 turn, len 20
        let wires = vec![straight, ell];
        assert_eq!(turns(&wires), 1, "0 + 1 bends");
        assert_eq!(length_px(&wires), 30, "10 + 20 px");
    }
}
