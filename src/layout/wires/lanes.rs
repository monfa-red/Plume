//! Bundle-aware mid-channel lane allocation.
//!
//! After endpoint allocation but before routing, this pass looks at every
//! Z-shape bundle's *natural* bend coordinate (the midpoint of the channel
//! between its two endpoints) and groups bundles whose natural bends fall
//! close enough to each other that they'd otherwise crowd the same
//! channel. Within each group, lanes are redistributed evenly with `gap`
//! spacing so every wire's bend ends up `gap` clear of every other.
//!
//! Two design rules keep this from collapsing fan-out trunks:
//!
//! 1. **Span-aware skip.** If every bundle in a group shares the same
//!    `wire.span`, they're siblings of one fan-out decl (`a -> b & c`).
//!    Their trunks are *meant* to overlap; leave them at their natural
//!    positions.
//! 2. **Bundle-level allocation.** A bundle of N siblings claims N
//!    contiguous lane slots, and its canonical bend sits at the slot
//!    midline. Sibling stamping (`±k·gap`) then lands each sibling
//!    exactly on its own slot, preserving intra-bundle spacing.

use super::endpoints::Endpoints;
use super::geometry::Edge;
use super::planning::SegmentSpec;
use super::stamping::Bundle;
use crate::span::Span;
use std::collections::HashSet;

/// Which axis the bundle's middle bend runs along.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum BendAxis {
    /// Bend is a vertical segment — facing-horizontal edges (Right↔Left).
    Vertical,
    /// Bend is a horizontal segment — facing-vertical edges (Top↔Bottom).
    Horizontal,
}

/// Natural bend information for one Z-shape bundle. Non-Z bundles
/// (perpendicular or same-direction edges) produce `None`.
pub struct BundleBend {
    pub axis: BendAxis,
    /// Natural bend coordinate: x for `Vertical` bends, y for `Horizontal`.
    pub natural: f64,
    /// Number of siblings the bundle stamps.
    pub size: usize,
    /// Wire-decl span — bundles sharing the same span are fan-out
    /// siblings and must keep their trunks merged.
    pub span: Span,
}

/// Result of one bundle's lane-allocation decision.
#[derive(Clone, Copy)]
pub struct BundleLane {
    /// Final bend coordinate after redistribution. The router uses this
    /// in place of the natural midpoint when picking the canonical's bend.
    pub bend: f64,
}

/// Compute the natural bend for each bundle. Returns `None` for bundles
/// whose topology isn't a Z (perpendicular L, same-direction U) — those
/// don't have a single shared channel to redistribute along.
pub fn compute_bundle_bends(
    bundles: &[Bundle],
    specs: &[SegmentSpec],
    endpoints: &Endpoints,
) -> Vec<Option<BundleBend>> {
    bundles
        .iter()
        .map(|bundle| {
            // Only facing-edge bundles have a single middle bend.
            if bundle.src_edge != bundle.tgt_edge.opposite() {
                return None;
            }
            let canonical_spec = &specs[bundle.spec_indices[0]];
            let (canonical_src_x, canonical_src_y) =
                centroid(bundle.spec_indices.iter().map(|&i| endpoints.src[i]));
            let (canonical_tgt_x, canonical_tgt_y) =
                centroid(bundle.spec_indices.iter().map(|&i| endpoints.tgt[i]));
            // Straight wires (endpoints aligned on the relevant axis)
            // render as a single segment with no bend — they don't occupy
            // a slot in the channel and shouldn't push other wires aside.
            let is_straight = if bundle.src_edge.is_horizontal_exit() {
                (canonical_src_y - canonical_tgt_y).abs() < 0.5
            } else {
                (canonical_src_x - canonical_tgt_x).abs() < 0.5
            };
            if is_straight {
                return None;
            }
            let (axis, natural) = if bundle.src_edge.is_horizontal_exit() {
                (
                    BendAxis::Vertical,
                    (canonical_src_x + canonical_tgt_x) / 2.0,
                )
            } else {
                (
                    BendAxis::Horizontal,
                    (canonical_src_y + canonical_tgt_y) / 2.0,
                )
            };
            Some(BundleBend {
                axis,
                natural,
                size: bundle.size(),
                span: canonical_spec.wire.span,
            })
        })
        .collect()
}

/// Group bundles whose natural bends are close enough to collide in the
/// same channel. Two bundles share a group iff they share an axis and
/// their natural bend coordinates lie within
/// `((size_a + size_b) · gap)` of each other — large enough to catch the
/// case where each bundle's own siblings would overlap the other's.
pub fn group_by_channel(bends: &[Option<BundleBend>], gap: f64) -> Vec<Vec<usize>> {
    let mut indexed: Vec<(usize, BendAxis, f64, usize)> = bends
        .iter()
        .enumerate()
        .filter_map(|(i, b)| b.as_ref().map(|b| (i, b.axis, b.natural, b.size)))
        .collect();
    indexed.sort_by(|a, b| {
        a.1.cmp_axis(&b.1)
            .then_with(|| a.2.partial_cmp(&b.2).unwrap_or(std::cmp::Ordering::Equal))
    });

    let mut groups: Vec<Vec<usize>> = Vec::new();
    let mut current: Vec<usize> = Vec::new();
    let mut current_axis: Option<BendAxis> = None;
    let mut last_pos: Option<f64> = None;
    let mut last_size: Option<usize> = None;

    for (i, axis, pos, size) in indexed {
        let threshold = (size + last_size.unwrap_or(size)) as f64 * gap;
        let in_same_group = current_axis == Some(axis)
            && last_pos.is_some()
            && (pos - last_pos.unwrap()).abs() < threshold;
        if in_same_group {
            current.push(i);
        } else {
            if current.len() > 1 {
                groups.push(std::mem::take(&mut current));
            } else {
                current.clear();
            }
            current.push(i);
            current_axis = Some(axis);
        }
        last_pos = Some(pos);
        last_size = Some(size);
    }
    if current.len() > 1 {
        groups.push(current);
    }
    groups
}

/// Redistribute the bundles in each channel group into evenly-spaced
/// lanes. Returns per-bundle lane assignments — `None` means "use natural,
/// no redistribution needed".
pub fn redistribute_channels(
    bends: &[Option<BundleBend>],
    groups: &[Vec<usize>],
    gap: f64,
) -> Vec<Option<BundleLane>> {
    let mut out: Vec<Option<BundleLane>> = vec![None; bends.len()];

    for group in groups {
        // Fan-out siblings share a `wire.span`; they're meant to merge
        // their trunks, so leave them at their natural positions.
        let spans: HashSet<Span> = group
            .iter()
            .map(|&i| bends[i].as_ref().unwrap().span)
            .collect();
        if spans.len() <= 1 {
            continue;
        }

        // Sort the group by natural position so neighbours in the channel
        // get neighbouring lane slots. Within the channel, lane order
        // mirrors natural-bend order.
        let mut sorted = group.clone();
        sorted.sort_by(|&a, &b| {
            bends[a]
                .as_ref()
                .unwrap()
                .natural
                .partial_cmp(&bends[b].as_ref().unwrap().natural)
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        // Total slots = sum of bundle sizes. Centre the lane span on the
        // size-weighted mean of natural positions so the redistributed
        // bends stay near where the wires want to be.
        let total_slots: usize = sorted
            .iter()
            .map(|&i| bends[i].as_ref().unwrap().size)
            .sum();
        let total_span = (total_slots as f64 - 1.0) * gap;
        let centre = {
            let mut sum = 0.0;
            let mut count = 0.0;
            for &i in &sorted {
                let b = bends[i].as_ref().unwrap();
                sum += b.natural * (b.size as f64);
                count += b.size as f64;
            }
            sum / count.max(1.0)
        };
        let first_lane = centre - total_span / 2.0;

        // Hand each bundle a contiguous run of slots; its canonical
        // bend sits at the run's midline so stamping `±k·gap` lands every
        // sibling on its own slot.
        let mut slot = 0;
        for &bi in &sorted {
            let size = bends[bi].as_ref().unwrap().size;
            let canonical_slot = slot as f64 + (size as f64 - 1.0) / 2.0;
            let canonical_position = first_lane + canonical_slot * gap;
            out[bi] = Some(BundleLane {
                bend: canonical_position,
            });
            slot += size;
        }
    }
    out
}

trait AxisCmp {
    fn cmp_axis(&self, other: &Self) -> std::cmp::Ordering;
}
impl AxisCmp for BendAxis {
    fn cmp_axis(&self, other: &Self) -> std::cmp::Ordering {
        use BendAxis::*;
        match (self, other) {
            (Vertical, Vertical) | (Horizontal, Horizontal) => std::cmp::Ordering::Equal,
            (Vertical, Horizontal) => std::cmp::Ordering::Less,
            (Horizontal, Vertical) => std::cmp::Ordering::Greater,
        }
    }
}

#[allow(dead_code)]
fn _edge_check(e: Edge) -> bool {
    matches!(e, Edge::Right | Edge::Left | Edge::Top | Edge::Bottom)
}

fn centroid(mut pts: impl Iterator<Item = (f64, f64)>) -> (f64, f64) {
    let mut n = 0.0;
    let mut sx = 0.0;
    let mut sy = 0.0;
    for (x, y) in pts.by_ref() {
        sx += x;
        sy += y;
        n += 1.0;
    }
    if n == 0.0 {
        (0.0, 0.0)
    } else {
        (sx / n, sy / n)
    }
}
