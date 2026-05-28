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
use super::geometry::{AbsBbox, Edge};
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
    /// Actual perpendicular spacing between siblings — equals `wire-gap`
    /// when the endpoint bin had room, less when the bin compressed lanes
    /// to fit. The channel allocator uses this to keep each bundle's
    /// bends *and* its sibling stamps clear of neighbouring bundles.
    pub stamping_gap: f64,
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
            // If an obstacle straddles both endpoint axes and sits
            // between src and tgt, no Z-shape can clear it — the router
            // will produce a 5-segment detour instead. Such bundles
            // don't compete for the channel's Z-trunk space, so leave
            // them out of redistribution.
            if z_shape_blocked(
                canonical_src_x,
                canonical_src_y,
                canonical_tgt_x,
                canonical_tgt_y,
                bundle.src_edge.is_horizontal_exit(),
                &obstacles,
            ) {
                return None;
            }
            let size = bundle.size();
            let stamping_gap = bundle_endpoint_spacing(bundle, endpoints, canonical_spec.gap);
            // Stamping puts each sibling at `canonical ± k·stamping_gap`,
            // so the canonical's clear range must shrink by
            // `(size-1)/2 · stamping_gap` on each side — otherwise the
            // outermost siblings would overflow into a shape obstacle.
            let sibling_radius = (size as f64 - 1.0) / 2.0 * stamping_gap;
            // The trunk also has to sit clear of the src and tgt shapes'
            // own halos. Without this the router rejects the Z (its
            // strict-clearance check forbids middle segments running
            // parallel close to either endpoint shape) and falls back to
            // a 5-segment detour, which adds two unnecessary bends.
            let src_halo = canonical_spec.src_bbox.inflate(canonical_spec.gap);
            let tgt_halo = canonical_spec.tgt_bbox.inflate(canonical_spec.gap);
            let (axis, natural, clear) = if bundle.src_edge.is_horizontal_exit() {
                let y_lo = canonical_src_y.min(canonical_tgt_y);
                let y_hi = canonical_src_y.max(canonical_tgt_y);
                let x_lo = canonical_src_x.min(canonical_tgt_x);
                let x_hi = canonical_src_x.max(canonical_tgt_x);
                let xs = clear_x_intervals(y_lo, y_hi, &obstacles, x_lo, x_hi);
                let natural_x = (canonical_src_x + canonical_tgt_x) / 2.0;
                let raw = pick_widest_interval(&xs, natural_x).unwrap_or((x_lo, x_hi));
                let (halo_lo, halo_hi) =
                    trunk_halo_bounds_horizontal(bundle.src_edge, &src_halo, &tgt_halo);
                let trunk_lo = raw.0.max(halo_lo);
                let trunk_hi = raw.1.min(halo_hi);
                let raw = if trunk_lo + sibling_radius < trunk_hi - sibling_radius {
                    (trunk_lo, trunk_hi)
                } else {
                    raw
                };
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
                let (halo_lo, halo_hi) =
                    trunk_halo_bounds_vertical(bundle.src_edge, &src_halo, &tgt_halo);
                let trunk_lo = raw.0.max(halo_lo);
                let trunk_hi = raw.1.min(halo_hi);
                let raw = if trunk_lo + sibling_radius < trunk_hi - sibling_radius {
                    (trunk_lo, trunk_hi)
                } else {
                    raw
                };
                let clear = (raw.0 + sibling_radius, raw.1 - sibling_radius);
                (BendAxis::Horizontal, natural_y, clear)
            };
            Some(BundleBend {
                axis,
                natural,
                clear,
                size,
                stamping_gap,
                span: canonical_spec.wire.span,
            })
        })
        .collect()
}

/// True when an obstacle straddles both endpoints' bend axes AND sits
/// between them on the bend axis — i.e., the bundle has to wrap around
/// it, so no Z is possible and the router will fall back to a detour.
fn z_shape_blocked(
    src_x: f64,
    src_y: f64,
    tgt_x: f64,
    tgt_y: f64,
    horizontal_exit: bool,
    obstacles: &[AbsBbox],
) -> bool {
    if horizontal_exit {
        let (x_lo, x_hi) = if src_x <= tgt_x {
            (src_x, tgt_x)
        } else {
            (tgt_x, src_x)
        };
        let (y_lo, y_hi) = if src_y <= tgt_y {
            (src_y, tgt_y)
        } else {
            (tgt_y, src_y)
        };
        obstacles
            .iter()
            .any(|o| o.y <= y_lo && y_hi <= o.bottom() && x_lo < o.right() && o.x < x_hi)
    } else {
        let (x_lo, x_hi) = if src_x <= tgt_x {
            (src_x, tgt_x)
        } else {
            (tgt_x, src_x)
        };
        let (y_lo, y_hi) = if src_y <= tgt_y {
            (src_y, tgt_y)
        } else {
            (tgt_y, src_y)
        };
        obstacles
            .iter()
            .any(|o| o.x <= x_lo && x_hi <= o.right() && y_lo < o.bottom() && o.y < y_hi)
    }
}

/// For a facing-horizontal Z (src on Right or Left, tgt on the opposite),
/// the trunk's x must lie east of one halo and west of the other.
/// Returns `(allowed_lo, allowed_hi)` — the open interval the trunk x
/// can land in without crossing either endpoint's halo.
fn trunk_halo_bounds_horizontal(
    src_edge: Edge,
    src_halo: &AbsBbox,
    tgt_halo: &AbsBbox,
) -> (f64, f64) {
    match src_edge {
        // src exits east → src west of tgt → trunk between src.right.halo and tgt.left.halo
        Edge::Right => (src_halo.right(), tgt_halo.x),
        Edge::Left => (tgt_halo.right(), src_halo.x),
        _ => (f64::NEG_INFINITY, f64::INFINITY),
    }
}

/// Mirror of `trunk_halo_bounds_horizontal` for facing-vertical bundles.
fn trunk_halo_bounds_vertical(
    src_edge: Edge,
    src_halo: &AbsBbox,
    tgt_halo: &AbsBbox,
) -> (f64, f64) {
    match src_edge {
        Edge::Bottom => (src_halo.bottom(), tgt_halo.y),
        Edge::Top => (tgt_halo.bottom(), src_halo.y),
        _ => (f64::NEG_INFINITY, f64::INFINITY),
    }
}

/// Same calculation mod.rs's `bundle_stamping_gap` uses: the actual
/// perpendicular spread between consecutive sibling endpoints. When the
/// source bin compressed lanes (too many wires for the edge length), this
/// is smaller than `wire-gap`, and the channel allocator needs to know
/// because the bundle's bend stamps share that same spacing.
fn bundle_endpoint_spacing(bundle: &Bundle, endpoints: &Endpoints, fallback_gap: f64) -> f64 {
    let size = bundle.size();
    if size <= 1 {
        return fallback_gap;
    }
    let horizontal_exit = bundle.src_edge.is_horizontal_exit();
    let mut min = f64::INFINITY;
    let mut max = f64::NEG_INFINITY;
    for &i in &bundle.spec_indices {
        let v = if horizontal_exit {
            endpoints.src[i].1
        } else {
            endpoints.src[i].0
        };
        min = min.min(v);
        max = max.max(v);
    }
    let spread = (max - min).max(0.0);
    if spread > 0.5 {
        spread / (size as f64 - 1.0)
    } else {
        fallback_gap
    }
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

/// Redistribute the bundles in each channel group so every bundle's
/// stamped sibling bends sit clear of every other bundle's stamped
/// siblings. A bundle's actual stamped span is
/// `(size − 1) · stamping_gap` — when its endpoint bin compressed lanes
/// (too many wires on one edge), `stamping_gap < wire-gap`, and the
/// channel allocator has to respect that smaller spacing or the
/// neighbour's bends end up interleaved between this bundle's stamps.
///
/// Sibling spans are never compressed (the stamps come from the endpoint
/// allocation and shifting them in the channel would desync the bend
/// from the start/end points). What we *can* compress is the buffer
/// between bundles — full `wire-gap` ideally, less if the channel is
/// tight, down to zero only when no other choice exists.
///
/// Returns `None` for bundles that don't need redistribution (fan-out
/// siblings or no channel conflict).
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

        let n = sorted.len();
        let half_spans: Vec<f64> = sorted
            .iter()
            .map(|&i| {
                let b = bends[i].as_ref().unwrap();
                (b.size as f64 - 1.0) / 2.0 * b.stamping_gap
            })
            .collect();
        let total_internal: f64 = half_spans.iter().map(|h| 2.0 * h).sum();

        // Available room: leftmost bundle's centre can sit as far left
        // as its `clear.0`, with `half_spans[0]` of its stamps reaching
        // further left; symmetric on the right.
        let leftmost = bends[sorted[0]].as_ref().unwrap().clear.0;
        let rightmost = bends[sorted[n - 1]].as_ref().unwrap().clear.1;
        let available = (rightmost + half_spans[n - 1]) - (leftmost - half_spans[0]);

        let needed = total_internal + (n as f64 - 1.0) * gap;
        let buffer = if n > 1 && needed > available {
            let leftover = available - total_internal;
            (leftover / (n as f64 - 1.0)).max(0.0)
        } else {
            gap
        };
        let used = total_internal + (n as f64 - 1.0) * buffer;

        // Centre the laid-out stack inside the available range.
        let stack_start = (leftmost - half_spans[0]) + (available - used) / 2.0;
        let mut cursor = stack_start;
        for (idx, &bi) in sorted.iter().enumerate() {
            let centre_raw = cursor + half_spans[idx];
            // Clamp into this bundle's own reachable range so its
            // outermost sibling never overflows into a shape obstacle.
            let b = bends[bi].as_ref().unwrap();
            let centre = centre_raw.clamp(b.clear.0, b.clear.1);
            out[bi] = Some(BundleLane { bend: centre });
            cursor += 2.0 * half_spans[idx];
            if idx + 1 < n {
                cursor += buffer;
            }
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
