//! Wire routing — channel-based.
//!
//! Pipeline:
//!
//! 1. **Plan** the resolved wires into one `SegmentSpec` per routed link
//!    (chains explode, fan-out specs share a `wire.span`).
//! 2. **Resolve edges** per geometric bundle: pick the `(src_edge,
//!    tgt_edge)` pair that yields the simplest topology (fewest bends).
//!    This runs *before* endpoint allocation so bins reflect the
//!    actually-used edges; bird → roof doesn't reserve four lanes on
//!    `roof.left` if water → roof is going to enter via `roof.bottom`.
//! 3. **Allocate endpoints**: for each (shape, edge) bin, distribute the
//!    wires using that endpoint into evenly-spaced lanes around the edge
//!    midpoint. Fan-out specs collapse onto a single shared lane.
//! 4. **Group** specs into bundles by (src, src_edge, tgt, tgt_edge). A
//!    bundle of N parallel wires routes once and is stamped N times by
//!    perpendicular shift, so siblings stay exactly `gap` apart.
//! 5. **Route** each bundle's canonical through the channel router. The
//!    router picks the minimum-bend topology (0 / 1 / 2 bends typically)
//!    that clears every obstacle, placing bends at channel midlines.
//! 6. **Stamp** siblings from the canonical via perpendicular shift.
//!
//! No grid, no A\*. Endpoints are pixel-perfect by construction; bends
//! are deterministic — same layout always produces the same routing.

mod channels;
mod endpoints;
mod geometry;
mod lanes;
mod planning;
mod route;
mod scene;
mod stamping;
mod text;

use crate::error::Error;
use crate::layout::ir::{PlacedNode, RoutedWire};
use crate::resolve::{MarkerKind, Markers, Program};

use endpoints::allocate_endpoints;
use geometry::{edge_midpoint, AbsBbox, Edge};
use lanes::{compute_bundle_bends, group_by_channel, redistribute_channels, BundleLane};
use planning::{plan_segments, SegmentSpec};
use scene::SceneIndex;
use stamping::{group_bundles, stamp_sibling, Bundle};
use text::place_texts;

pub fn route_wires(
    program: &Program,
    scene_nodes: &[PlacedNode],
) -> Result<Vec<RoutedWire>, Error> {
    let scene = SceneIndex::build(scene_nodes, &program.scene.attrs);
    let mut specs = plan_segments(program, &scene)?;

    // Pad the world bounds by the largest gap so perimeter detours have
    // room outside every shape.
    let max_gap = specs.iter().map(|s| s.gap).fold(0.0_f64, f64::max).max(8.0);
    let world = scene.bounds(max_gap);

    // Pick each bundle's actually-best `(src_edge, tgt_edge)` based on
    // simulated topology length, *before* allocating endpoint lanes. If
    // we wait until routing, the bin allocator wastes lanes on edges that
    // never carry a wire (visible as bird → roof landing off-centre on
    // roof.left because two lanes were reserved for water → roof, which
    // ended up exiting via roof.bottom).
    resolve_edges(&mut specs, &scene, world);

    let endpoints = allocate_endpoints(&specs);
    let bundles = group_bundles(&specs);

    // Bundle-aware lane allocation: where Z-shape bundles would otherwise
    // crowd the same channel, redistribute their canonical bends evenly so
    // every unrelated wire ends up `gap` apart. Fan-out bundles (same
    // wire span) are exempted — their shared trunks are by design.
    let bends = compute_bundle_bends(&bundles, &specs, &endpoints, &scene);
    let channel_groups = group_by_channel(
        &bends,
        specs.iter().map(|s| s.gap).fold(0.0_f64, f64::max).max(8.0),
    );
    let lane_assignments = redistribute_channels(
        &bends,
        &channel_groups,
        specs.iter().map(|s| s.gap).fold(0.0_f64, f64::max).max(8.0),
    );

    let mut routed: Vec<Option<RoutedWire>> = (0..specs.len()).map(|_| None).collect();
    let mut prior_paths: Vec<Vec<(f64, f64)>> = Vec::with_capacity(specs.len());

    for (bi, bundle) in bundles.iter().enumerate() {
        let canonical_path = route_bundle_canonical(
            bundle,
            &specs,
            &endpoints,
            &scene,
            world,
            &prior_paths,
            lane_assignments[bi],
        );

        let size = bundle.size();
        // Stamp siblings using the spec endpoints' *actual* spread —
        // this matches the compressed-lane allocation done in
        // endpoints.rs when too many wires share one (shape, edge).
        // Otherwise siblings would land at the full `gap` while their
        // endpoints sat at compressed positions, breaking parallelism.
        let stamping_gap = bundle_stamping_gap(bundle, &endpoints, &specs);
        for (k, &spec_idx) in bundle.spec_indices.iter().enumerate() {
            let path = stamp_sibling(&canonical_path, k, size, stamping_gap);
            prior_paths.push(path.clone());
            routed[spec_idx] = Some(build_routed_wire(&specs[spec_idx], path));
        }
    }

    Ok(routed.into_iter().map(Option::unwrap).collect())
}

/// Route the canonical polyline for a bundle. The canonical's endpoints
/// are the centroid of its specs' allocated endpoints, so when stamping
/// shifts siblings by `±k·gap` perpendicular, each sibling lands on its
/// own allocated endpoint exactly.
///
/// If the natural-edge route can't clear every obstacle, try alternative
/// edge pairs (perpendicular to the default) and keep the first that
/// validates — falling back to the original if none do.
#[allow(clippy::too_many_arguments)]
fn route_bundle_canonical(
    bundle: &Bundle,
    specs: &[SegmentSpec],
    endpoints: &endpoints::Endpoints,
    scene: &SceneIndex,
    world: AbsBbox,
    prior_paths: &[Vec<(f64, f64)>],
    lane: Option<BundleLane>,
) -> Vec<(f64, f64)> {
    let canonical_spec = &specs[bundle.spec_indices[0]];
    let obstacles = scene.obstacles_for(
        &canonical_spec.src_id,
        &canonical_spec.tgt_id,
        canonical_spec.gap,
    );

    // Try the bundle's chosen edges first, then perpendicular fallbacks.
    let edge_combos = edge_fallback_order(
        bundle.src_edge,
        bundle.tgt_edge,
        &canonical_spec.src_bbox,
        &canonical_spec.tgt_bbox,
    );

    let canonical_src = centroid(bundle.spec_indices.iter().map(|&i| endpoints.src[i]));
    let canonical_tgt = centroid(bundle.spec_indices.iter().map(|&i| endpoints.tgt[i]));

    let src_halo = canonical_spec.src_bbox.inflate(canonical_spec.gap);
    let tgt_halo = canonical_spec.tgt_bbox.inflate(canonical_spec.gap);

    let mut fallback: Option<Vec<(f64, f64)>> = None;
    for (se, te) in edge_combos {
        let src_pt = if se == bundle.src_edge {
            canonical_src
        } else {
            edge_midpoint(&canonical_spec.src_bbox, se)
        };
        let tgt_pt = if te == bundle.tgt_edge {
            canonical_tgt
        } else {
            edge_midpoint(&canonical_spec.tgt_bbox, te)
        };
        // Use the channel-assigned bend only if the routing keeps the
        // bundle's natural edges — alternative edge combos have a
        // different bend axis, so the redistributed value doesn't apply.
        // The bend is dispatched to z_shape (for Z bundles) or
        // detour_facing's near-tgt approach column (for detour bundles).
        let (preferred_trunk, preferred_b2) = if se == bundle.src_edge && te == bundle.tgt_edge {
            match lane {
                Some(l) => match l.kind {
                    lanes::BendKind::ZTrunk => (Some(l.bend), None),
                    lanes::BendKind::DetourB2 => (None, Some(l.bend)),
                },
                None => (None, None),
            }
        } else {
            (None, None)
        };
        let sibling_radius =
            (bundle.size() as f64 - 1.0) / 2.0 * bundle_stamping_gap(bundle, endpoints, specs);
        let path = route::route(
            src_pt,
            tgt_pt,
            se,
            te,
            &obstacles,
            world,
            prior_paths,
            canonical_spec.gap,
            sibling_radius,
            preferred_trunk,
            preferred_b2,
            canonical_spec.src_bbox,
            canonical_spec.tgt_bbox,
        );
        // Strict clearance: middle segments must avoid running parallel
        // close to src or tgt. If this edge combo can't satisfy that,
        // try the next one before settling.
        if route::path_is_clear(&path, &obstacles, &src_halo, &tgt_halo) {
            return path;
        }
        if fallback.is_none() {
            fallback = Some(path);
        }
    }
    fallback.unwrap_or_else(|| vec![canonical_src, canonical_tgt])
}

/// Per geometric bundle, pick the `(src_edge, tgt_edge)` that yields the
/// simplest topology — usually fewest bends, then geometric default as a
/// tiebreaker. This runs once before endpoint allocation so the bin sizes
/// reflect the edges that wires actually use, not what `plan_segments`
/// initially guessed from raw geometry. Any spec whose endpoint has an
/// explicit `.side` override (`spec.src_forced` / `spec.tgt_forced`) is
/// left alone on that side; the user already chose for us.
fn resolve_edges(specs: &mut [SegmentSpec], scene: &SceneIndex, world: AbsBbox) {
    use std::collections::BTreeMap;
    type Key = (String, Edge, String, Edge);
    let mut groups: BTreeMap<Key, Vec<usize>> = BTreeMap::new();
    for (i, spec) in specs.iter().enumerate() {
        let initial_src = spec.src_forced.unwrap_or(spec.src_default_edge);
        let initial_tgt = spec.tgt_forced.unwrap_or(spec.tgt_default_edge);
        let key = (
            spec.src_id.clone(),
            initial_src,
            spec.tgt_id.clone(),
            initial_tgt,
        );
        groups.entry(key).or_default().push(i);
    }
    for indices in groups.values() {
        let sample = &specs[indices[0]];
        if sample.src_forced.is_some() && sample.tgt_forced.is_some() {
            continue;
        }
        let obstacles = scene.obstacles_for(&sample.src_id, &sample.tgt_id, sample.gap);
        let (best_src, best_tgt) = pick_best_edges(sample, &obstacles, world);
        for &i in indices {
            if specs[i].src_forced.is_none() {
                specs[i].src_default_edge = best_src;
            }
            if specs[i].tgt_forced.is_none() {
                specs[i].tgt_default_edge = best_tgt;
            }
        }
    }

    // Second pass: relieve over-capacity edges by re-routing bundles
    // whose endpoint edge can't fit all the wires that landed on it.
    // Iterates until stable (or hits a safety cap) — moving one bundle
    // changes neighbouring bin loads, which can unlock further moves.
    relieve_overloaded_bins(specs, scene, world);
}

/// Loop: for each (shape, edge) bin whose load (count of bundle slots)
/// exceeds the edge's fit-without-compression capacity, try to move one
/// bundle's endpoint to a less-loaded alternative edge — provided the
/// alternative is still topology-clean and doesn't itself overload.
/// Stops as soon as no improving move is found.
fn relieve_overloaded_bins(specs: &mut [SegmentSpec], scene: &SceneIndex, world: AbsBbox) {
    const MAX_ITERS: usize = 16;
    for _ in 0..MAX_ITERS {
        if !try_one_relief(specs, scene, world) {
            return;
        }
    }
}

fn try_one_relief(specs: &mut [SegmentSpec], scene: &SceneIndex, world: AbsBbox) -> bool {
    use std::collections::{BTreeMap, HashSet};
    type Key = (String, Edge, String, Edge);
    let mut bundle_specs: BTreeMap<Key, Vec<usize>> = BTreeMap::new();
    for (i, spec) in specs.iter().enumerate() {
        let se = spec.src_forced.unwrap_or(spec.src_default_edge);
        let te = spec.tgt_forced.unwrap_or(spec.tgt_default_edge);
        let key = (spec.src_id.clone(), se, spec.tgt_id.clone(), te);
        bundle_specs.entry(key).or_default().push(i);
    }
    let mut load: BTreeMap<(String, Edge), usize> = BTreeMap::new();
    for (key, indices) in &bundle_specs {
        let src_spans: HashSet<_> = indices.iter().map(|&i| specs[i].wire.span).collect();
        let tgt_spans: HashSet<_> = indices.iter().map(|&i| specs[i].wire.span).collect();
        *load.entry((key.0.clone(), key.1)).or_insert(0) += src_spans.len();
        *load.entry((key.2.clone(), key.3)).or_insert(0) += tgt_spans.len();
    }

    let gap = specs.first().map_or(12.0, |s| s.gap);

    // Find a (shape, edge) bin where multiple Z-bundles' combined trunk
    // span won't fit in their shared channel. Bin overload alone isn't
    // a problem — endpoint lanes compress fine. It's two bundles
    // forced to interleave their trunks in too narrow a channel that
    // breaks the visual.
    let mut bins: BTreeMap<(String, Edge), Vec<Key>> = BTreeMap::new();
    for k in bundle_specs.keys() {
        bins.entry((k.0.clone(), k.1)).or_default().push(k.clone());
        bins.entry((k.2.clone(), k.3)).or_default().push(k.clone());
    }
    let mut overload: Option<(String, Edge)> = None;
    for ((shape, edge), keys) in &bins {
        // Only Z-bundles (facing edges) share a trunk channel. Even
        // when the canonical is straight (src.y == tgt.y for a
        // facing-horizontal bundle), the siblings of a >1-spec bundle
        // bend because their lane offsets differ on each side — so we
        // still need to count them as channel occupants.
        let z_keys: Vec<&Key> = keys.iter().filter(|k| k.1.opposite() == k.3).collect();
        if z_keys.len() < 2 {
            continue;
        }
        // Fan-out bundles (e.g. `cat -> bowl & water` becomes a cat→bowl
        // bundle and a cat→water bundle, both with the same wire span)
        // are MEANT to share the trunk — they don't crowd each other.
        // Only count unique wire decls toward the buffer-between count.
        let unique_spans: HashSet<_> = z_keys
            .iter()
            .flat_map(|k| bundle_specs[*k].iter().map(|&i| specs[i].wire.span))
            .collect();
        if unique_spans.len() < 2 {
            continue;
        }
        let Some(this_bbox) = scene.lookup(shape).map(|s| s.bbox) else {
            continue;
        };
        let this_clearance = scene.clearance(shape).unwrap_or(0.0).max(gap);
        let mut min_partner_gap = f64::INFINITY;
        let mut min_partner_clearance = f64::INFINITY;
        for k in &z_keys {
            let on_src = k.0 == *shape && k.1 == *edge;
            let partner = if on_src { &k.2 } else { &k.0 };
            let Some(partner_bbox) = scene.lookup(partner).map(|s| s.bbox) else {
                continue;
            };
            let pg = partner_distance(&this_bbox, *edge, &partner_bbox);
            if pg < min_partner_gap {
                min_partner_gap = pg;
            }
            let pc = scene.clearance(partner).unwrap_or(0.0).max(gap);
            if pc < min_partner_clearance {
                min_partner_clearance = pc;
            }
        }
        // Each unique wire decl occupies one trunk slot at `gap` spacing
        // — that already covers spacing both within and between bundles
        // (since fan-out specs share a span and so collapse to one slot).
        let required = (unique_spans.len() as f64 - 1.0) * gap;
        let available = (min_partner_gap - this_clearance - min_partner_clearance).max(0.0);
        if required > available {
            overload = Some((shape.clone(), *edge));
            break;
        }
    }
    let Some((overloaded_shape, overloaded_edge)) = overload else {
        return false;
    };

    // Try every bundle on the overloaded bin. For each, search every
    // (src_edge, tgt_edge) combo that *doesn't* land on the overloaded
    // bin. Pick the cheapest valid move that fits in its new bin and
    // strictly reduces total overload.
    let edges = [Edge::Right, Edge::Bottom, Edge::Left, Edge::Top];
    let mut best_move: Option<(Key, Edge, Edge, i64)> = None;
    for (key, indices) in &bundle_specs {
        let sample = &specs[indices[0]];
        let obstacles = scene.obstacles_for(&sample.src_id, &sample.tgt_id, sample.gap);
        let src_match = key.0 == overloaded_shape && key.1 == overloaded_edge;
        let tgt_match = key.2 == overloaded_shape && key.3 == overloaded_edge;
        if !src_match && !tgt_match {
            continue;
        }
        let bundle_slots: HashSet<_> = indices.iter().map(|&i| specs[i].wire.span).collect();
        for &se in &edges {
            if let Some(forced) = sample.src_forced {
                if se != forced {
                    continue;
                }
            }
            for &te in &edges {
                if let Some(forced) = sample.tgt_forced {
                    if te != forced {
                        continue;
                    }
                }
                if (se, te) == (key.1, key.3) {
                    continue;
                }
                // Must vacate the overloaded bin.
                if src_match && se == overloaded_edge {
                    continue;
                }
                if tgt_match && te == overloaded_edge {
                    continue;
                }
                let score = simulate_path_score(sample, &obstacles, world, se, te);
                if !score.valid || score.len > 4 {
                    continue;
                }
                // New endpoint bins must have room.
                let src_new = scene.lookup(&sample.src_id).map(|s| s.bbox);
                let tgt_new = scene.lookup(&sample.tgt_id).map(|s| s.bbox);
                let (Some(src_b), Some(tgt_b)) = (src_new, tgt_new) else {
                    continue;
                };
                let src_cap = edge_capacity(&src_b, se, sample.gap);
                let tgt_cap = edge_capacity(&tgt_b, te, sample.gap);
                let src_load_after = load.get(&(sample.src_id.clone(), se)).copied().unwrap_or(0)
                    + if se != key.1 { bundle_slots.len() } else { 0 };
                let tgt_load_after = load.get(&(sample.tgt_id.clone(), te)).copied().unwrap_or(0)
                    + if te != key.3 { bundle_slots.len() } else { 0 };
                if src_load_after > src_cap || tgt_load_after > tgt_cap {
                    continue;
                }
                // Score: prefer shorter path; ties to combos closer to
                // the geometric default (already captured by edge_fallback_order
                // ordering, but here we just use vertex count and accept first).
                let score_val = score.len as i64;
                if best_move.as_ref().map_or(true, |m| score_val < m.3) {
                    best_move = Some((key.clone(), se, te, score_val));
                }
            }
        }
    }
    let Some((key, new_src, new_tgt, _)) = best_move else {
        return false;
    };
    let indices = bundle_specs.get(&key).cloned().unwrap_or_default();
    for &i in &indices {
        if specs[i].src_forced.is_none() {
            specs[i].src_default_edge = new_src;
        }
        if specs[i].tgt_forced.is_none() {
            specs[i].tgt_default_edge = new_tgt;
        }
    }
    true
}

/// Distance from `this`'s `edge` to `partner`'s opposite edge — i.e.
/// the width of the gap between the two shapes along the axis the
/// bend would travel.
fn partner_distance(this: &AbsBbox, edge: Edge, partner: &AbsBbox) -> f64 {
    match edge {
        Edge::Right => partner.x - this.right(),
        Edge::Left => this.x - partner.right(),
        Edge::Bottom => partner.y - this.bottom(),
        Edge::Top => this.y - partner.bottom(),
    }
}

/// How many wires can sit on this edge at the default `gap` spacing
/// before the bin allocator has to compress them. We use this as a
/// crude proxy for "the channel will look crowded".
fn edge_capacity(bbox: &AbsBbox, edge: Edge, gap: f64) -> usize {
    const INSET: f64 = 4.0;
    let usable = match edge {
        Edge::Left | Edge::Right => (bbox.h - 2.0 * INSET).max(0.0),
        Edge::Top | Edge::Bottom => (bbox.w - 2.0 * INSET).max(0.0),
    };
    if gap <= 0.0 {
        return 0;
    }
    ((usable / gap).floor() as usize).saturating_add(1)
}

/// If the geometric default produces a clean valid Z (≤ 4 vertices and
/// every segment clear of obstacles + halos), keep it — that's the
/// topology the diagram author meant. Only when the default would force
/// a 5-bend detour OR cross an obstacle do we go shopping for an
/// alternative `(src_edge, tgt_edge)` that yields a simpler valid shape.
///
/// "Valid" matters even for short paths: `u_shape` with `(Top, Top)` can
/// return a 4-vertex path that ploughs straight through a sibling shape
/// sitting in the same column above src. Counting vertices alone would
/// prefer that invalid 4-pt path over a valid 6-pt detour; the validity
/// gate forces detour wins.
///
/// Cleanness threshold: ≤ 4 vertices covers straight (2), L (3), and Z
/// (4); 6 vertices is the facing-detour, 5 the perpendicular detour.
/// Switching away from `(Right, Left)` to L just because L is shorter
/// would re-route `cat → bowl` to enter from the top — visually wrong.
fn pick_best_edges(spec: &SegmentSpec, obstacles: &[AbsBbox], world: AbsBbox) -> (Edge, Edge) {
    let default_src = spec.src_forced.unwrap_or(spec.src_default_edge);
    let default_tgt = spec.tgt_forced.unwrap_or(spec.tgt_default_edge);

    let default_score = simulate_path_score(spec, obstacles, world, default_src, default_tgt);
    if default_score.is_clean() {
        return (default_src, default_tgt);
    }

    // Default would detour or crash an obstacle. Try alternatives and
    // pick the simplest valid one. The default's rank-0 score acts as
    // tiebreaker — an alternative wins only if it strictly beats the
    // default on validity or vertex count.
    let combos = edge_fallback_order(default_src, default_tgt, &spec.src_bbox, &spec.tgt_bbox);
    let mut best = (default_src, default_tgt);
    let mut best_key = default_score.sort_key(0);
    for (rank, &(se, te)) in combos.iter().enumerate() {
        if let Some(forced) = spec.src_forced {
            if se != forced {
                continue;
            }
        }
        if let Some(forced) = spec.tgt_forced {
            if te != forced {
                continue;
            }
        }
        let score = simulate_path_score(spec, obstacles, world, se, te);
        let key = score.sort_key(rank as i64);
        if key < best_key {
            best_key = key;
            best = (se, te);
        }
    }
    best
}

/// Topology-quality summary for one candidate `(src_edge, tgt_edge)`.
#[derive(Clone, Copy)]
struct PathScore {
    /// Path vertex count — proxy for bends.
    len: usize,
    /// True if every segment cleared every obstacle and stayed out of
    /// the src/tgt halos along middle segments. Strict-check failure
    /// here means the actual router will also reject it and fall back.
    valid: bool,
}

impl PathScore {
    fn is_clean(self) -> bool {
        self.valid && self.len <= 4
    }
    /// Lower is better. Invalid paths get a giant penalty so they only
    /// win if *every* candidate is invalid (rare, signifies a layout
    /// the router can't help). Among valid paths, vertex count wins;
    /// `rank` (from `edge_fallback_order`) breaks ties so the geometric
    /// default beats an equally-good alternative.
    fn sort_key(self, rank: i64) -> i64 {
        let invalid_pen = if self.valid { 0 } else { 1_000_000 };
        invalid_pen + (self.len as i64) * 100 + rank
    }
}

fn simulate_path_score(
    spec: &SegmentSpec,
    obstacles: &[AbsBbox],
    world: AbsBbox,
    se: Edge,
    te: Edge,
) -> PathScore {
    let src_pt = edge_midpoint(&spec.src_bbox, se);
    let tgt_pt = edge_midpoint(&spec.tgt_bbox, te);
    let path = route::route(
        src_pt,
        tgt_pt,
        se,
        te,
        obstacles,
        world,
        &[],
        spec.gap,
        0.0,
        None,
        None,
        spec.src_bbox,
        spec.tgt_bbox,
    );
    let src_halo = spec.src_bbox.inflate(spec.gap);
    let tgt_halo = spec.tgt_bbox.inflate(spec.gap);
    // `path_is_clear` only checks the middle segments for halo overlap,
    // so it can't see L-shapes whose corner sits inside the tgt bbox
    // or "u-collapsed" straights whose single segment ploughs across
    // the tgt. The extra checks below cover those cases.
    let bends_clear = bends_outside_endpoint_bboxes(&path, &spec.src_bbox, &spec.tgt_bbox);
    let entry_ok = path_entry_matches_edges(&path, se, te);
    let valid =
        route::path_is_clear(&path, obstacles, &src_halo, &tgt_halo) && bends_clear && entry_ok;
    PathScore {
        len: path.len(),
        valid,
    }
}

/// Every segment except the *first* must stay clear of src's bbox, and
/// every segment except the *last* must stay clear of tgt's bbox. This
/// catches paths where the router took a shortcut straight through the
/// endpoint shape — visually obvious as a wire that pierces dog from
/// one side and comes out the other.
fn bends_outside_endpoint_bboxes(
    path: &[(f64, f64)],
    src_bbox: &AbsBbox,
    tgt_bbox: &AbsBbox,
) -> bool {
    let n = path.len();
    if n < 2 {
        return true;
    }
    for i in 0..n - 1 {
        let a = path[i];
        let b = path[i + 1];
        if i != 0 && segment_intersects_box(a, b, src_bbox) {
            return false;
        }
        if i != n - 2 && segment_intersects_box(a, b, tgt_bbox) {
            return false;
        }
    }
    true
}

/// True if the axis-aligned segment `a → b` has any point strictly
/// inside `bbox`. Endpoints on the edge of `bbox` don't count.
fn segment_intersects_box(a: (f64, f64), b: (f64, f64), bbox: &AbsBbox) -> bool {
    let (x_lo, x_hi) = if a.0 <= b.0 { (a.0, b.0) } else { (b.0, a.0) };
    let (y_lo, y_hi) = if a.1 <= b.1 { (a.1, b.1) } else { (b.1, a.1) };
    let x_overlap = x_lo < bbox.right() - 0.5 && x_hi > bbox.x + 0.5;
    let y_overlap = y_lo < bbox.bottom() - 0.5 && y_hi > bbox.y + 0.5;
    x_overlap && y_overlap
}

/// True iff the path's first segment heads in the outward direction of
/// `se` (out of src) and its last segment heads inward at `te`. A
/// "u-collapsed" straight that ends at tgt going parallel to tgt's edge
/// fails this: the wire crosses tgt instead of entering it.
fn path_entry_matches_edges(path: &[(f64, f64)], se: Edge, te: Edge) -> bool {
    if path.len() < 2 {
        return true;
    }
    let (a0, a1) = (path[0], path[1]);
    let dx0 = a1.0 - a0.0;
    let dy0 = a1.1 - a0.1;
    let src_ok = match se {
        Edge::Right => dx0 > 0.5,
        Edge::Left => dx0 < -0.5,
        Edge::Bottom => dy0 > 0.5,
        Edge::Top => dy0 < -0.5,
    };
    if !src_ok {
        return false;
    }
    let n = path.len();
    let (b0, b1) = (path[n - 2], path[n - 1]);
    let dx1 = b1.0 - b0.0;
    let dy1 = b1.1 - b0.1;
    match te {
        Edge::Right => dx1 < -0.5,
        Edge::Left => dx1 > 0.5,
        Edge::Bottom => dy1 < -0.5,
        Edge::Top => dy1 > 0.5,
    }
}

/// Build the priority-ordered list of (src_edge, tgt_edge) pairs to try.
/// The default pair (already in `bundle`) comes first; then perpendicular
/// alternatives on each side, skipping any edge that points strictly
/// away from the partner shape.
fn edge_fallback_order(
    default_src: Edge,
    default_tgt: Edge,
    src_bbox: &AbsBbox,
    tgt_bbox: &AbsBbox,
) -> Vec<(Edge, Edge)> {
    let src_candidates = candidate_edges(src_bbox, tgt_bbox, default_src);
    let tgt_candidates = candidate_edges(tgt_bbox, src_bbox, default_tgt);

    let mut out = Vec::with_capacity(src_candidates.len() * tgt_candidates.len());
    // Default pair first.
    out.push((default_src, default_tgt));
    // Then permutations, default-edge biased.
    for &te in &tgt_candidates {
        if te == default_tgt {
            continue;
        }
        out.push((default_src, te));
    }
    for &se in &src_candidates {
        if se == default_src {
            continue;
        }
        out.push((se, default_tgt));
    }
    for &se in &src_candidates {
        if se == default_src {
            continue;
        }
        for &te in &tgt_candidates {
            if te == default_tgt {
                continue;
            }
            out.push((se, te));
        }
    }
    out
}

/// Edges of `my` worth trying when routing toward `other`. We list every
/// edge — even ones that face away from the partner — because tight
/// layouts sometimes need the "wrong" direction: water → roof can route
/// cleanly via `water.Bottom` even though Bottom points south while roof
/// is north, since the wraparound is the only obstacle-free path. The
/// default edge is listed first so it wins ties in `pick_best_edges`.
fn candidate_edges(_my: &AbsBbox, _other: &AbsBbox, default: Edge) -> Vec<Edge> {
    let mut out = vec![default];
    for e in [Edge::Right, Edge::Bottom, Edge::Left, Edge::Top] {
        if e != default {
            out.push(e);
        }
    }
    out
}

/// The actual perpendicular distance between consecutive siblings of
/// `bundle`. For a bundle whose endpoint lanes weren't compressed,
/// equals `spec.gap`. For an overflowing bin (more wires than fit at
/// `gap`), equals the compressed step that allocate_lanes used.
fn bundle_stamping_gap(
    bundle: &Bundle,
    endpoints: &endpoints::Endpoints,
    specs: &[SegmentSpec],
) -> f64 {
    let size = bundle.size();
    if size <= 1 {
        return specs[bundle.spec_indices[0]].gap;
    }
    // For facing-horizontal bundles siblings differ in y; for vertical
    // siblings differ in x. Use whichever axis the edge spans.
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
        specs[bundle.spec_indices[0]].gap
    }
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

fn build_routed_wire(spec: &SegmentSpec, path: Vec<(f64, f64)>) -> RoutedWire {
    RoutedWire {
        markers: Markers {
            start: if spec.is_first {
                spec.wire.markers.start
            } else {
                MarkerKind::None
            },
            end: if spec.is_last {
                spec.wire.markers.end
            } else {
                MarkerKind::None
            },
        },
        attrs: spec.wire.attrs.clone(),
        texts: if spec.is_first {
            place_texts(&spec.wire.texts, &path)
        } else {
            Vec::new()
        },
        data_from: spec.data_from.clone(),
        data_to: spec.data_to.clone(),
        path,
    }
}
