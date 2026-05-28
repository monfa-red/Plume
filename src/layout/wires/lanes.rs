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

use super::channels::{clear_x_intervals, clear_y_intervals};
use super::endpoints::Endpoints;
use super::planning::SegmentSpec;
use super::scene::SceneIndex;
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
    /// The clear interval along the bend axis — the x range a `Vertical`
    /// bend can occupy, or the y range a `Horizontal` bend can occupy,
    /// while keeping every sibling segment gap-clear from shape obstacles.
    pub clear: (f64, f64),
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
    scene: &SceneIndex,
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
            let obstacles = scene.obstacles_for(
                &canonical_spec.src_id,
                &canonical_spec.tgt_id,
                canonical_spec.gap,
            );
            let size = bundle.size();
            // Stamping puts each sibling at `canonical ± k·gap`, so the
            // canonical's clear range must shrink by `(size-1)/2 · gap`
            // on each side — otherwise the outermost siblings would
            // overflow into a shape obstacle.
            let sibling_radius = (size as f64 - 1.0) / 2.0 * canonical_spec.gap;
            let (axis, natural, clear) = if bundle.src_edge.is_horizontal_exit() {
                let y_lo = canonical_src_y.min(canonical_tgt_y);
                let y_hi = canonical_src_y.max(canonical_tgt_y);
                let x_lo = canonical_src_x.min(canonical_tgt_x);
                let x_hi = canonical_src_x.max(canonical_tgt_x);
                let xs = clear_x_intervals(y_lo, y_hi, &obstacles, x_lo, x_hi);
                let natural_x = (canonical_src_x + canonical_tgt_x) / 2.0;
                let raw = pick_widest_interval(&xs, natural_x).unwrap_or((x_lo, x_hi));
                let clear = (raw.0 + sibling_radius, raw.1 - sibling_radius);
                (BendAxis::Vertical, natural_x, clear)
            } else {
                let x_lo = canonical_src_x.min(canonical_tgt_x);
                let x_hi = canonical_src_x.max(canonical_tgt_x);
                let y_lo = canonical_src_y.min(canonical_tgt_y);
                let y_hi = canonical_src_y.max(canonical_tgt_y);
                let ys = clear_y_intervals(x_lo, x_hi, &obstacles, y_lo, y_hi);
                let natural_y = (canonical_src_y + canonical_tgt_y) / 2.0;
                let raw = pick_widest_interval(&ys, natural_y).unwrap_or((y_lo, y_hi));
                let clear = (raw.0 + sibling_radius, raw.1 - sibling_radius);
                (BendAxis::Horizontal, natural_y, clear)
            };
            Some(BundleBend {
                axis,
                natural,
                clear,
                size,
                span: canonical_spec.wire.span,
            })
        })
        .collect()
}

/// Pick the clear interval most likely to contain the natural bend —
/// prefers the one containing `natural`, falls back to the closest. We
/// return both bounds so the channel allocator can clamp against this
/// bundle's actual reachable range.
fn pick_widest_interval(
    intervals: &[super::channels::Interval],
    natural: f64,
) -> Option<(f64, f64)> {
    intervals
        .iter()
        .min_by(|a, b| {
            let da = if a.contains(natural) {
                0.0
            } else {
                (a.mid() - natural).abs()
            };
            let db = if b.contains(natural) {
                0.0
            } else {
                (b.mid() - natural).abs()
            };
            da.partial_cmp(&db).unwrap_or(std::cmp::Ordering::Equal)
        })
        .map(|iv| (iv.min, iv.max))
}

/// Group bundles whose middle bends would crowd the same channel. Two
/// bundles share a group iff they share an axis AND their clear
/// intervals overlap by more than `gap` (so there's room to place
/// distinct lanes within the overlap).
///
/// Uses transitive union-find: if A overlaps B and B overlaps C, all
/// three end up in one group, even when A and C don't directly touch.
pub fn group_by_channel(bends: &[Option<BundleBend>], gap: f64) -> Vec<Vec<usize>> {
    let active: Vec<usize> = bends
        .iter()
        .enumerate()
        .filter_map(|(i, b)| b.as_ref().map(|_| i))
        .collect();

    let mut parent: Vec<usize> = (0..bends.len()).collect();
    fn find(parent: &mut [usize], i: usize) -> usize {
        let mut root = i;
        while parent[root] != root {
            root = parent[root];
        }
        let mut cur = i;
        while parent[cur] != root {
            let next = parent[cur];
            parent[cur] = root;
            cur = next;
        }
        root
    }
    fn union(parent: &mut [usize], a: usize, b: usize) {
        let ra = find(parent, a);
        let rb = find(parent, b);
        if ra != rb {
            parent[ra] = rb;
        }
    }

    for (i_idx, &i) in active.iter().enumerate() {
        let bi = bends[i].as_ref().unwrap();
        for &j in &active[i_idx + 1..] {
            let bj = bends[j].as_ref().unwrap();
            if bi.axis != bj.axis {
                continue;
            }
            // Naturals must lie within `(size_a + size_b) * gap` of each
            // other — that's how much room the combined slots would need.
            // Bundles further apart aren't competing for the same lanes.
            let natural_threshold = (bi.size + bj.size) as f64 * gap;
            if (bi.natural - bj.natural).abs() >= natural_threshold {
                continue;
            }
            // Their clear intervals must overlap by at least `gap` — if
            // there's no common reachable space, redistributing them
            // together just pushes one outside its own interval.
            let lo = bi.clear.0.max(bj.clear.0);
            let hi = bi.clear.1.min(bj.clear.1);
            if hi - lo < gap {
                continue;
            }
            union(&mut parent, i, j);
        }
    }

    let mut buckets: std::collections::HashMap<usize, Vec<usize>> =
        std::collections::HashMap::new();
    for &i in &active {
        let root = find(&mut parent, i);
        buckets.entry(root).or_default().push(i);
    }
    buckets.into_values().filter(|g| g.len() > 1).collect()
}

/// Redistribute the bundles in each channel group into evenly-spaced
/// lanes WITHIN the channel's shared clear interval. If the desired
/// `gap` spacing doesn't fit, the spacing is compressed proportionally
/// so every bundle still lands inside its reachable interval — closer
/// than ideal but never overlapping a shape obstacle.
///
/// Returns `None` for bundles that don't need redistribution (fan-out
/// siblings, no channel conflict).
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

        let mut sorted = group.clone();
        sorted.sort_by(|&a, &b| {
            bends[a]
                .as_ref()
                .unwrap()
                .natural
                .partial_cmp(&bends[b].as_ref().unwrap().natural)
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        // Lanes have to fit inside the intersection of every bundle's
        // clear interval — that's the set of positions every bundle can
        // actually reach. Use that as the usable channel.
        let channel_lo = sorted
            .iter()
            .map(|&i| bends[i].as_ref().unwrap().clear.0)
            .fold(f64::NEG_INFINITY, f64::max);
        let channel_hi = sorted
            .iter()
            .map(|&i| bends[i].as_ref().unwrap().clear.1)
            .fold(f64::INFINITY, f64::min);
        let channel_width = (channel_hi - channel_lo).max(0.0);

        let total_slots: usize = sorted
            .iter()
            .map(|&i| bends[i].as_ref().unwrap().size)
            .sum();
        if total_slots <= 1 {
            continue;
        }
        // Spacing is `gap` when the channel has room; otherwise compress
        // proportionally so the full slot span still fits inside the
        // channel.
        let needed_span = (total_slots as f64 - 1.0) * gap;
        let spacing = if needed_span <= channel_width {
            gap
        } else {
            channel_width / (total_slots as f64 - 1.0)
        };

        // Centre the lane span inside the channel.
        let total_span = (total_slots as f64 - 1.0) * spacing;
        let first_lane = channel_lo + (channel_width - total_span) / 2.0;

        let mut slot = 0;
        for &bi in &sorted {
            let size = bends[bi].as_ref().unwrap().size;
            let canonical_slot = slot as f64 + (size as f64 - 1.0) / 2.0;
            let canonical_position = first_lane + canonical_slot * spacing;
            out[bi] = Some(BundleLane {
                bend: canonical_position,
            });
            slot += size;
        }
    }
    out
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
