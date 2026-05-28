//! Wire routing — channel-based.
//!
//! Pipeline:
//!
//! 1. **Plan** the resolved wires into one `SegmentSpec` per routed link
//!    (chains explode, fan-out specs share a `wire.span`).
//! 2. **Allocate endpoints**: for each (shape, edge) bin, distribute the
//!    wires using that endpoint into evenly-spaced lanes around the edge
//!    midpoint. Fan-out specs collapse onto a single shared lane.
//! 3. **Group** specs into bundles by (src, src_edge, tgt, tgt_edge). A
//!    bundle of N parallel wires routes once and is stamped N times by
//!    perpendicular shift, so siblings stay exactly `gap` apart.
//! 4. **Route** each bundle's canonical through the channel router. The
//!    router picks the minimum-bend topology (0 / 1 / 2 bends typically)
//!    that clears every obstacle, placing bends at channel midlines.
//! 5. **Stamp** siblings from the canonical via perpendicular shift.
//!
//! No grid, no A\*. Endpoints are pixel-perfect by construction; bends
//! are deterministic — same layout always produces the same routing.

mod channels;
mod endpoints;
mod geometry;
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
use planning::{plan_segments, SegmentSpec};
use scene::SceneIndex;
use stamping::{group_bundles, stamp_sibling, Bundle};
use text::place_texts;

pub fn route_wires(
    program: &Program,
    scene_nodes: &[PlacedNode],
) -> Result<Vec<RoutedWire>, Error> {
    let scene = SceneIndex::build(scene_nodes);
    let specs = plan_segments(program, &scene)?;
    let endpoints = allocate_endpoints(&specs);
    let bundles = group_bundles(&specs);

    // Pad the world bounds by the largest gap so perimeter detours have
    // room outside every shape.
    let max_gap = specs.iter().map(|s| s.gap).fold(0.0_f64, f64::max).max(8.0);
    let world = scene.bounds(max_gap);

    let mut routed: Vec<Option<RoutedWire>> = (0..specs.len()).map(|_| None).collect();
    let mut prior_paths: Vec<Vec<(f64, f64)>> = Vec::with_capacity(specs.len());

    for bundle in &bundles {
        let canonical_path =
            route_bundle_canonical(bundle, &specs, &endpoints, &scene, world, &prior_paths);

        let size = bundle.size();
        let gap = specs[bundle.spec_indices[0]].gap;
        for (k, &spec_idx) in bundle.spec_indices.iter().enumerate() {
            let path = stamp_sibling(&canonical_path, k, size, gap);
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
fn route_bundle_canonical(
    bundle: &Bundle,
    specs: &[SegmentSpec],
    endpoints: &endpoints::Endpoints,
    scene: &SceneIndex,
    world: AbsBbox,
    prior_paths: &[Vec<(f64, f64)>],
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
        let path = route::route(src_pt, tgt_pt, se, te, &obstacles, world, prior_paths);
        if route::path_is_clear(&path, &obstacles) {
            return path;
        }
        if fallback.is_none() {
            fallback = Some(path);
        }
    }
    fallback.unwrap_or_else(|| vec![canonical_src, canonical_tgt])
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

/// Edges of `my` worth trying when routing toward `other` — every edge
/// except the one strictly facing AWAY from `other`. The default edge is
/// listed first.
fn candidate_edges(my: &AbsBbox, other: &AbsBbox, default: Edge) -> Vec<Edge> {
    let dx = other.cx() - my.cx();
    let dy = other.cy() - my.cy();
    let mut out = vec![default];
    for e in [Edge::Right, Edge::Bottom, Edge::Left, Edge::Top] {
        if e == default {
            continue;
        }
        let strictly_away = match e {
            Edge::Right => dx < -0.5,
            Edge::Left => dx > 0.5,
            Edge::Bottom => dy < -0.5,
            Edge::Top => dy > 0.5,
        };
        if !strictly_away {
            out.push(e);
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
