//! Endpoint allocation — for each (shape, edge) bin, distribute the wires
//! using that endpoint into evenly-spaced lanes around the edge midpoint.
//!
//! Two rules:
//!
//! 1. **One slot per wire decl in the bin.** Fan-out specs from a single
//!    decl (`a -> b & c` produces two specs sharing `wire.span`) collapse
//!    onto the same slot — so they exit from one shared point.
//! 2. **Median-centred.** N slots produce lane offsets
//!    `(i − (N−1)/2) × gap` for `i = 0..N` — symmetric about the edge
//!    midpoint, regardless of declaration order.
//!
//! Output: per-spec exact `(x, y)` for both ends. The router takes these
//! as ground truth — it doesn't compute endpoints from the bbox itself.

use super::geometry::{edge_midpoint, shift_along_edge, AbsBbox, Edge};
use super::planning::SegmentSpec;
use crate::span::Span;
use std::collections::HashMap;

pub struct Endpoints {
    /// Per spec: exact world coords where the wire lands on its source.
    pub src: Vec<(f64, f64)>,
    /// Per spec: exact world coords where the wire lands on its target.
    pub tgt: Vec<(f64, f64)>,
    /// Per spec: which edge of the source shape is used.
    #[allow(dead_code)]
    pub src_edge: Vec<Edge>,
    /// Per spec: which edge of the target shape is used.
    #[allow(dead_code)]
    pub tgt_edge: Vec<Edge>,
}

pub fn allocate_endpoints(specs: &[SegmentSpec]) -> Endpoints {
    let n = specs.len();
    let mut src_edge = vec![Edge::Right; n];
    let mut tgt_edge = vec![Edge::Right; n];
    for (i, spec) in specs.iter().enumerate() {
        src_edge[i] = spec.src_forced.unwrap_or(spec.src_default_edge);
        tgt_edge[i] = spec.tgt_forced.unwrap_or(spec.tgt_default_edge);
    }

    let src_lanes = allocate_lanes(specs, &src_edge, Side::Src);
    let tgt_lanes = allocate_lanes(specs, &tgt_edge, Side::Tgt);

    let mut src = vec![(0.0, 0.0); n];
    let mut tgt = vec![(0.0, 0.0); n];
    for (i, spec) in specs.iter().enumerate() {
        src[i] = endpoint_coord(&spec.src_bbox, src_edge[i], src_lanes[i]);
        tgt[i] = endpoint_coord(&spec.tgt_bbox, tgt_edge[i], tgt_lanes[i]);
    }

    Endpoints {
        src,
        tgt,
        src_edge,
        tgt_edge,
    }
}

/// Exact world coord where a wire on `edge` of `bbox` with lane offset
/// `lane` lands. Lane is along the edge, clamped so it stays on the edge.
fn endpoint_coord(bbox: &AbsBbox, edge: Edge, lane: f64) -> (f64, f64) {
    shift_along_edge(edge_midpoint(bbox, edge), edge, lane, bbox)
}

#[derive(Copy, Clone, PartialEq, Eq)]
enum Side {
    Src,
    Tgt,
}

fn allocate_lanes(specs: &[SegmentSpec], edges: &[Edge], side: Side) -> Vec<f64> {
    let mut lanes = vec![0.0_f64; specs.len()];

    // Group spec indices by (shape_id, edge) bin.
    let mut bins: HashMap<(String, Edge), Vec<usize>> = HashMap::new();
    for (i, spec) in specs.iter().enumerate() {
        let key = match side {
            Side::Src => (spec.src_id.clone(), edges[i]),
            Side::Tgt => (spec.tgt_id.clone(), edges[i]),
        };
        bins.entry(key).or_default().push(i);
    }

    for indices in bins.values() {
        // Assign slots, collapsing fan-out specs (same wire span) onto one.
        let mut span_to_slot: HashMap<Span, usize> = HashMap::new();
        let mut spec_slot: HashMap<usize, usize> = HashMap::new();
        let mut slot_count = 0;
        for &i in indices {
            let span = specs[i].wire.span;
            let slot = *span_to_slot.entry(span).or_insert_with(|| {
                let s = slot_count;
                slot_count += 1;
                s
            });
            spec_slot.insert(i, slot);
        }
        if slot_count == 0 {
            continue;
        }
        let gap = specs[indices[0]].gap;
        let centre = (slot_count as f64 - 1.0) / 2.0;
        for &i in indices {
            let slot = spec_slot[&i] as f64;
            lanes[i] = (slot - centre) * gap;
        }
    }

    lanes
}
