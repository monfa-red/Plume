//! Graph-based routing orchestrator (Step 2). Builds one visibility grid for
//! the whole scene, then routes every wire on it with A*. Replaces the
//! topology-template pipeline (Z/L/U/detour + bundle stamping).

use super::astar;
use super::endpoints::allocate_endpoints;
use super::geometry::{AbsBbox, Edge};
use super::grid::Grid;
use super::oracle;
use super::planning::SegmentSpec;
use super::scene::SceneIndex;

/// Minimum perpendicular stub out of / into a shape edge, even when the
/// shape's clearance is tiny — guarantees a perpendicular first/last segment
/// (rule R5) and a node just outside the shape for A* to start from.
const MIN_STUB: f64 = 8.0;

/// Route every spec to an orthogonal polyline.
///
/// Each wire leaves its source edge and enters its target edge along a
/// perpendicular **stub**; A* connects the two stub ends across the shared
/// visibility grid. Step 2.4 routes against shape clearances only — wire–wire
/// separation arrives in 2.5 via the A* surcharge.
pub fn route_all(
    specs: &[SegmentSpec],
    scene: &SceneIndex,
    world: AbsBbox,
) -> Vec<Vec<(f64, f64)>> {
    let endpoints = allocate_endpoints(specs);

    // Per-wire perpendicular stub endpoints just outside each shape. A small
    // fixed step (not the full clearance) so the stub never overshoots a
    // nearby partner when clearance is large relative to shape spacing.
    let src_stub: Vec<(f64, f64)> = (0..specs.len())
        .map(|i| step_out(endpoints.src[i], endpoints.src_edge[i], MIN_STUB))
        .collect();
    let tgt_stub: Vec<(f64, f64)> = (0..specs.len())
        .map(|i| step_out(endpoints.tgt[i], endpoints.tgt_edge[i], MIN_STUB))
        .collect();

    // Global lattice: every shape's clearance-inflated edges + the world frame
    // + every attachment and stub coordinate (so each is a grid node).
    let inflated: Vec<AbsBbox> = scene
        .all_boxes()
        .into_iter()
        .map(|(path, b)| b.inflate(oracle::shape_clearance(scene, &path)))
        .collect();
    let mut xs = Vec::with_capacity(specs.len() * 4);
    let mut ys = Vec::with_capacity(specs.len() * 4);
    for i in 0..specs.len() {
        for p in [endpoints.src[i], src_stub[i], endpoints.tgt[i], tgt_stub[i]] {
            xs.push(p.0);
            ys.push(p.1);
        }
    }
    let grid = Grid::build(&inflated, world, &xs, &ys);
    let no_surcharge = |_: (f64, f64), _: (f64, f64)| 0.0;

    (0..specs.len())
        .map(|i| {
            // Per-wire obstacles: every shape except this wire's endpoints and
            // their ancestors, each grown by its own clearance (the oracle).
            let obstacles: Vec<AbsBbox> = scene
                .raw_obstacles(&specs[i].src_id, &specs[i].tgt_id)
                .into_iter()
                .map(|(path, b)| b.inflate(oracle::shape_clearance(scene, &path)))
                .collect();
            let mid = astar::route(&grid, src_stub[i], tgt_stub[i], &obstacles, &no_surcharge);
            assemble(
                endpoints.src[i],
                src_stub[i],
                tgt_stub[i],
                endpoints.tgt[i],
                mid,
            )
        })
        .collect()
}

/// Stitch the attachment points, stubs, and the A* middle into one polyline.
/// Falls back to an orthogonal elbow (never a diagonal) if A* found no path.
fn assemble(
    src: (f64, f64),
    src_stub: (f64, f64),
    tgt_stub: (f64, f64),
    tgt: (f64, f64),
    mid: Option<Vec<(f64, f64)>>,
) -> Vec<(f64, f64)> {
    let mut path = vec![src];
    match mid {
        Some(m) => path.extend(m),
        None => {
            // Last resort: keep it orthogonal and perpendicular at both ends.
            // The validator will flag any clearance breach (R2/R6).
            path.push(src_stub);
            path.push((tgt_stub.0, src_stub.1));
            path.push(tgt_stub);
        }
    }
    path.push(tgt);
    astar::collapse(path)
}

/// Move `pt` outward from a shape edge by `d`, perpendicular to that edge.
fn step_out(pt: (f64, f64), edge: Edge, d: f64) -> (f64, f64) {
    match edge {
        Edge::Right => (pt.0 + d, pt.1),
        Edge::Left => (pt.0 - d, pt.1),
        Edge::Top => (pt.0, pt.1 - d),
        Edge::Bottom => (pt.0, pt.1 + d),
    }
}
