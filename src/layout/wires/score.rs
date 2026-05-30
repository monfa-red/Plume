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

/// `(invariants, B1, B2n, B2w, crossings, turns, length_px)` — compare with `<`.
pub type Score = (usize, usize, usize, usize, usize, usize, usize);

/// Score a candidate routing: the validator's contract counts, then turns + length.
pub fn score(wires: &[RoutedWire], nodes: &[PlacedNode]) -> Score {
    let vs = validate_routing(nodes, &AttrMap::new(), wires, &VarTable::new());
    let (mut inv, mut b1, mut b2n, mut b2w, mut crossings) = (0, 0, 0, 0, 0);
    for v in &vs {
        match v.rule {
            Rule::NodeOverlap => b1 += 1,
            Rule::Clearance => b2n += 1,
            Rule::Separation => b2w += 1,
            Rule::Crossing => crossings += 1,
            r if r.severity() == Severity::Invariant => inv += 1,
            _ => {}
        }
    }
    (inv, b1, b2n, b2w, crossings, turns(wires), length_px(wires))
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
