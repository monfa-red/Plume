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
//! After the per-bin centring, a second pass aligns "free" bundles — a
//! bundle whose endpoint bin holds only that one bundle is uniformly
//! shifted along its edge so its endpoints sit on the same world axis as
//! the partner side. When both bins are equally constrained the wire keeps
//! its centred placement and renders as a Z.
//!
//! Output: per-spec exact `(x, y)` for both ends. The router takes these
//! as ground truth — it doesn't compute endpoints from the bbox itself.

use super::geometry::{edge_midpoint, shift_along_edge, AbsBbox, Edge};
use super::planning::SegmentSpec;
use crate::span::Span;
use std::collections::{BTreeMap, BTreeSet};

pub struct Endpoints {
    /// Per spec: exact world coords where the wire lands on its source.
    pub src: Vec<(f64, f64)>,
    /// Per spec: exact world coords where the wire lands on its target.
    pub tgt: Vec<(f64, f64)>,
    /// Per spec: which edge of the source shape is used.
    pub src_edge: Vec<Edge>,
    /// Per spec: which edge of the target shape is used.
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

    let mut src_lanes = allocate_lanes(specs, &src_edge, Side::Src);
    let mut tgt_lanes = allocate_lanes(specs, &tgt_edge, Side::Tgt);

    align_free_bundles(specs, &src_edge, &tgt_edge, &mut src_lanes, &mut tgt_lanes);

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
    let mut bins: BTreeMap<(String, Edge), Vec<usize>> = BTreeMap::new();
    for (i, spec) in specs.iter().enumerate() {
        let key = match side {
            Side::Src => (spec.src_id.clone(), edges[i]),
            Side::Tgt => (spec.tgt_id.clone(), edges[i]),
        };
        bins.entry(key).or_default().push(i);
    }

    const INSET: f64 = 4.0;

    for ((_, edge), indices) in &bins {
        // Assign slots, collapsing fan-out specs (same wire span) onto one.
        let mut span_to_slot: BTreeMap<Span, usize> = BTreeMap::new();
        let mut spec_slot: BTreeMap<usize, usize> = BTreeMap::new();
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

        // Compress the lane spacing if the natural span would overflow
        // the edge length. Bin overflow happens when too many wires share
        // one (shape, edge) — e.g. N parallel `cat -> dog` specs all
        // entering dog's left edge. Without compression, the outer lanes
        // clamp onto each other; with compression, each spec gets its
        // own slot evenly distributed across the usable edge length.
        let first_spec_bbox = match side {
            Side::Src => &specs[indices[0]].src_bbox,
            Side::Tgt => &specs[indices[0]].tgt_bbox,
        };
        let usable = match edge {
            Edge::Left | Edge::Right => (first_spec_bbox.h - 2.0 * INSET).max(0.0),
            Edge::Top | Edge::Bottom => (first_spec_bbox.w - 2.0 * INSET).max(0.0),
        };
        let natural_span = (slot_count as f64 - 1.0) * gap;
        let effective_gap = if slot_count > 1 && natural_span > usable {
            (usable / (slot_count as f64 - 1.0)).max(1.0)
        } else {
            gap
        };

        let centre = (slot_count as f64 - 1.0) / 2.0;
        for &i in indices {
            let slot = spec_slot[&i] as f64;
            lanes[i] = (slot - centre) * effective_gap;
        }
    }

    lanes
}

/// A bundle is identified by `(src_id, src_edge, tgt_id, tgt_edge)`. Two specs
/// share a bundle iff their endpoints land on the same pair of edges.
type BundleKey = (String, Edge, String, Edge);

/// Pass 2: for each bundle, if one endpoint bin is "free" (contains only
/// that single bundle) and the other isn't, shift the free side uniformly
/// so the wire endpoints align on the same world axis. Skip the adjustment
/// when the shift would push any endpoint outside the free side's bin.
///
/// "Free" means the bin has exactly one bundle keyed in it — shifting that
/// bundle's lanes can't collide with anything else. Bins that pivot a
/// fan-out (multiple specs sharing one slot but each in a different
/// bundle) count as multi-bundle, so the trunk stays where centring put it.
fn align_free_bundles(
    specs: &[SegmentSpec],
    src_edge: &[Edge],
    tgt_edge: &[Edge],
    src_lanes: &mut [f64],
    tgt_lanes: &mut [f64],
) {
    let mut bundle_keys = Vec::with_capacity(specs.len());
    for (i, spec) in specs.iter().enumerate() {
        bundle_keys.push((
            spec.src_id.clone(),
            src_edge[i],
            spec.tgt_id.clone(),
            tgt_edge[i],
        ));
    }

    let mut src_bin_bundles: BTreeMap<(String, Edge), BTreeSet<BundleKey>> = BTreeMap::new();
    let mut tgt_bin_bundles: BTreeMap<(String, Edge), BTreeSet<BundleKey>> = BTreeMap::new();
    for (i, spec) in specs.iter().enumerate() {
        src_bin_bundles
            .entry((spec.src_id.clone(), src_edge[i]))
            .or_default()
            .insert(bundle_keys[i].clone());
        tgt_bin_bundles
            .entry((spec.tgt_id.clone(), tgt_edge[i]))
            .or_default()
            .insert(bundle_keys[i].clone());
    }

    let mut by_bundle: BTreeMap<BundleKey, Vec<usize>> = BTreeMap::new();
    for (i, key) in bundle_keys.iter().enumerate() {
        by_bundle.entry(key.clone()).or_default().push(i);
    }

    for indices in by_bundle.values() {
        let first = indices[0];
        let se = src_edge[first];
        let te = tgt_edge[first];
        // Only facing edges can produce a straight wire. Mixed
        // (perpendicular or same-direction) bundles share no axis to align
        // on, so leave them centred.
        if se.opposite() != te {
            continue;
        }

        let src_bundles = src_bin_bundles[&(specs[first].src_id.clone(), se)].len();
        let tgt_bundles = tgt_bin_bundles[&(specs[first].tgt_id.clone(), te)].len();

        if src_bundles > 1 && tgt_bundles == 1 {
            try_uniform_shift(specs, indices, src_lanes, tgt_lanes, te, Side::Tgt);
        } else if tgt_bundles > 1 && src_bundles == 1 {
            try_uniform_shift(specs, indices, src_lanes, tgt_lanes, se, Side::Src);
        }
    }
}

/// Shift `side`'s lanes for every spec in `indices` by a uniform delta so
/// that each spec's endpoint lands on the same world axis as its partner.
/// Skips if the shift would push any spec's lane outside its bin's usable
/// half-extent.
fn try_uniform_shift(
    specs: &[SegmentSpec],
    indices: &[usize],
    src_lanes: &mut [f64],
    tgt_lanes: &mut [f64],
    edge: Edge,
    side: Side,
) {
    let horizontal = edge.is_horizontal_exit();

    let desired: Vec<f64> = indices
        .iter()
        .map(|&i| {
            let (partner_centre, this_centre) = match side {
                Side::Src => (
                    partner_axis(&specs[i].tgt_bbox, horizontal),
                    this_axis(&specs[i].src_bbox, horizontal),
                ),
                Side::Tgt => (
                    partner_axis(&specs[i].src_bbox, horizontal),
                    this_axis(&specs[i].tgt_bbox, horizontal),
                ),
            };
            let partner_lane = match side {
                Side::Src => tgt_lanes[i],
                Side::Tgt => src_lanes[i],
            };
            (partner_centre + partner_lane) - this_centre
        })
        .collect();

    let current: Vec<f64> = indices
        .iter()
        .map(|&i| match side {
            Side::Src => src_lanes[i],
            Side::Tgt => tgt_lanes[i],
        })
        .collect();

    // Shift must be uniform across the bundle so siblings keep their relative
    // spacing. Pass 1 assigns each bundle consecutive slots in the same source
    // order on both sides, so deltas should already match.
    let delta = desired[0] - current[0];
    for k in 1..indices.len() {
        if ((desired[k] - current[k]) - delta).abs() > 0.5 {
            return;
        }
    }
    if delta.abs() < 0.01 {
        return;
    }

    for &i in indices {
        let new_lane = match side {
            Side::Src => src_lanes[i] + delta,
            Side::Tgt => tgt_lanes[i] + delta,
        };
        let bbox = match side {
            Side::Src => &specs[i].src_bbox,
            Side::Tgt => &specs[i].tgt_bbox,
        };
        if !lane_within_bin(bbox, edge, new_lane) {
            return;
        }
    }

    for &i in indices {
        match side {
            Side::Src => src_lanes[i] += delta,
            Side::Tgt => tgt_lanes[i] += delta,
        }
    }
}

fn partner_axis(bbox: &AbsBbox, horizontal_exit: bool) -> f64 {
    if horizontal_exit {
        bbox.cy()
    } else {
        bbox.cx()
    }
}

fn this_axis(bbox: &AbsBbox, horizontal_exit: bool) -> f64 {
    if horizontal_exit {
        bbox.cy()
    } else {
        bbox.cx()
    }
}

fn lane_within_bin(bbox: &AbsBbox, edge: Edge, lane: f64) -> bool {
    const INSET: f64 = 4.0;
    let half = match edge {
        Edge::Left | Edge::Right => (bbox.h - 2.0 * INSET).max(0.0) / 2.0,
        Edge::Top | Edge::Bottom => (bbox.w - 2.0 * INSET).max(0.0) / 2.0,
    };
    // Strict inequality (with a 0.5 px margin) keeps alignment from pinning
    // the endpoint to a corner — e.g. a bundle whose partner side is at
    // its bin's compressed extreme shouldn't drag a single-spec free bin
    // out to the same extreme. A wire we can't align without entering at
    // the edge is better off centred.
    lane.abs() < half - 0.5
}
