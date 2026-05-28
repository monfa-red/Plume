//! Bundle grouping and sibling stamping.
//!
//! A *bundle* is a set of segments sharing the same source shape + source
//! edge AND the same target shape + target edge — i.e. parallel wires
//! between one pair of edges. We route the bundle's canonical path once,
//! then produce siblings by perpendicular shift, so the siblings stay
//! exactly parallel and `gap` apart.

use super::geometry::Edge;
use super::planning::SegmentSpec;
use std::collections::HashMap;

pub struct Bundle {
    #[allow(dead_code)]
    pub src_id: String,
    pub src_edge: Edge,
    #[allow(dead_code)]
    pub tgt_id: String,
    pub tgt_edge: Edge,
    /// Indices into the original `specs` array, in source order.
    pub spec_indices: Vec<usize>,
}

impl Bundle {
    pub fn size(&self) -> usize {
        self.spec_indices.len()
    }
}

/// Group specs into bundles keyed on (src, src_edge, tgt, tgt_edge). Bundles
/// preserve creation order — i.e. each bundle's position in the returned
/// `Vec` matches the source order of its first spec.
pub fn group_bundles(specs: &[SegmentSpec]) -> Vec<Bundle> {
    type Key = (String, Edge, String, Edge);
    let mut by_key: HashMap<Key, usize> = HashMap::new();
    let mut bundles: Vec<Bundle> = Vec::new();
    for (i, spec) in specs.iter().enumerate() {
        // For bundle keying, use the spec's chosen edges. Forced overrides
        // win; otherwise the geometry-default edge.
        let src_edge = spec.src_forced.unwrap_or(spec.src_default_edge);
        let tgt_edge = spec.tgt_forced.unwrap_or(spec.tgt_default_edge);
        let key = (spec.src_id.clone(), src_edge, spec.tgt_id.clone(), tgt_edge);
        let bi = *by_key.entry(key.clone()).or_insert_with(|| {
            bundles.push(Bundle {
                src_id: key.0.clone(),
                src_edge: key.1,
                tgt_id: key.2.clone(),
                tgt_edge: key.3,
                spec_indices: Vec::new(),
            });
            bundles.len() - 1
        });
        bundles[bi].spec_indices.push(i);
    }
    bundles
}

/// Stamp one sibling of a `size`-N bundle by perpendicular-shifting the
/// canonical polyline. `k` is the sibling index (0..N). The sibling at
/// `k = (N-1)/2` is the canonical itself (no shift).
pub fn stamp_sibling(canonical: &[(f64, f64)], k: usize, size: usize, gap: f64) -> Vec<(f64, f64)> {
    let centre = (size as f64 - 1.0) / 2.0;
    let shift = (k as f64 - centre) * gap;
    if shift.abs() < 0.5 {
        canonical.to_vec()
    } else {
        shift_polyline(canonical, shift)
    }
}

/// Shift an orthogonal polyline by `delta` perpendicular to each segment.
/// Horizontal segments move on the y-axis; vertical segments move on the
/// x-axis. At each bend the new corner is the intersection of the two
/// shifted lines — so straight stretches stay parallel and bend topology
/// is preserved.
pub fn shift_polyline(path: &[(f64, f64)], delta: f64) -> Vec<(f64, f64)> {
    if path.len() < 2 {
        return path.to_vec();
    }
    let mut shifted: Vec<((f64, f64), (f64, f64))> = Vec::with_capacity(path.len() - 1);
    for w in path.windows(2) {
        let (a, b) = (w[0], w[1]);
        let dy = (b.1 - a.1).abs();
        let dx = (b.0 - a.0).abs();
        let segment = if dy < 0.5 {
            ((a.0, a.1 + delta), (b.0, b.1 + delta))
        } else if dx < 0.5 {
            ((a.0 + delta, a.1), (b.0 + delta, b.1))
        } else {
            (a, b)
        };
        shifted.push(segment);
    }

    let mut out = Vec::with_capacity(shifted.len() + 1);
    out.push(shifted[0].0);
    for pair in shifted.windows(2) {
        let (a1, b1) = pair[0];
        let (a2, _) = pair[1];
        out.push(intersect_orthogonal(a1, b1, a2));
    }
    out.push(shifted.last().unwrap().1);
    out
}

/// Intersection of two perpendicular axis-aligned lines: one passes
/// through `a1`–`b1`, the other passes through `a2` and is perpendicular
/// to the first.
fn intersect_orthogonal(a1: (f64, f64), b1: (f64, f64), a2: (f64, f64)) -> (f64, f64) {
    let horizontal_first = (a1.1 - b1.1).abs() < 0.5;
    if horizontal_first {
        (a2.0, a1.1)
    } else {
        (a1.0, a2.1)
    }
}
