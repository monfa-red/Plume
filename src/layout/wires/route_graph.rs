//! Graph-based routing orchestrator (Step 2). Builds one visibility grid for
//! the whole scene, then routes every wire on it with A*. Replaces the
//! topology-template pipeline (Z/L/U/detour + bundle stamping).

use super::astar;
use super::endpoints::allocate_endpoints;
use super::geometry::{edge_midpoint, AbsBbox, Edge};
use super::grid::Grid;
use super::oracle;
use super::planning::SegmentSpec;
use super::scene::SceneIndex;
use super::tracks::{self, End};
use crate::span::Span;

/// Keep endpoints off the exact corners when they slide along an edge.
const EDGE_INSET: f64 = 4.0;

/// Minimum perpendicular stub out of / into a shape edge, even when the
/// shape's clearance is tiny — guarantees a perpendicular first/last segment
/// (rule R5) and a node just outside the shape for A* to start from.
const MIN_STUB: f64 = 8.0;

const ALL_EDGES: [Edge; 4] = [Edge::Right, Edge::Bottom, Edge::Left, Edge::Top];

/// Route every spec to an orthogonal polyline.
///
/// Each wire leaves its source edge and enters its target edge along a
/// perpendicular **stub**; A* connects the two stub ends across the shared
/// visibility grid. If the geometry-preferred edge is sealed off (the stub lands
/// inside a neighbour's clearance — e.g. two shapes a sub-clearance gap apart),
/// the wire retries from its other, unforced edges. A final track-assignment
/// pass (Step 2.5) fans parallel wires onto separated lanes.
pub fn route_all(
    specs: &[SegmentSpec],
    scene: &SceneIndex,
    world: AbsBbox,
) -> Vec<Vec<(f64, f64)>> {
    let endpoints = allocate_endpoints(specs);

    // Global lattice: every shape's clearance-inflated edges + the world frame
    // + every candidate attachment and stub coordinate (so each is a grid node).
    // Both the lane-allocated points and every edge midpoint are seeded, so an
    // alternate-edge retry always has nodes to land on.
    let inflated: Vec<AbsBbox> = scene
        .all_boxes()
        .into_iter()
        .map(|(path, b)| b.inflate(oracle::shape_clearance(scene, &path)))
        .collect();
    let mut xs = Vec::new();
    let mut ys = Vec::new();
    let seed = |p: (f64, f64), xs: &mut Vec<f64>, ys: &mut Vec<f64>| {
        xs.push(p.0);
        ys.push(p.1);
    };
    for (i, spec) in specs.iter().enumerate() {
        seed(endpoints.src[i], &mut xs, &mut ys);
        seed(
            step_out(endpoints.src[i], endpoints.src_edge[i], MIN_STUB),
            &mut xs,
            &mut ys,
        );
        seed(endpoints.tgt[i], &mut xs, &mut ys);
        seed(
            step_out(endpoints.tgt[i], endpoints.tgt_edge[i], MIN_STUB),
            &mut xs,
            &mut ys,
        );
        for e in ALL_EDGES {
            let sp = edge_midpoint(&spec.src_bbox, e);
            seed(sp, &mut xs, &mut ys);
            seed(step_out(sp, e, MIN_STUB), &mut xs, &mut ys);
            let tp = edge_midpoint(&spec.tgt_bbox, e);
            seed(tp, &mut xs, &mut ys);
            seed(step_out(tp, e, MIN_STUB), &mut xs, &mut ys);
        }
    }
    let grid = Grid::build(&inflated, world, &xs, &ys);

    // Per-wire obstacles: every shape except this wire's endpoints and their
    // ancestors, each grown by its own clearance (the oracle).
    let obstacles: Vec<Vec<AbsBbox>> = (0..specs.len())
        .map(|i| {
            scene
                .raw_obstacles(&specs[i].src_id, &specs[i].tgt_id)
                .into_iter()
                .map(|(path, b)| b.inflate(oracle::shape_clearance(scene, &path)))
                .collect()
        })
        .collect();

    let mut paths = Vec::with_capacity(specs.len());
    let mut src_edges = Vec::with_capacity(specs.len());
    let mut tgt_edges = Vec::with_capacity(specs.len());
    for (i, spec) in specs.iter().enumerate() {
        let (path, se, te) = route_wire(spec, &endpoints, i, &grid, &obstacles[i]);
        paths.push(path);
        src_edges.push(se);
        tgt_edges.push(te);
    }

    // Step 2.5: fan parallel wires onto separated, shape-clear tracks.
    let decls: Vec<Span> = specs.iter().map(|s| s.wire.span).collect();
    let gaps: Vec<f64> = specs.iter().map(|s| s.gap).collect();
    let ends: Vec<(End, End)> = (0..specs.len())
        .map(|i| {
            (
                edge_clamp(&specs[i].src_bbox, src_edges[i]),
                edge_clamp(&specs[i].tgt_bbox, tgt_edges[i]),
            )
        })
        .collect();
    tracks::assign(&mut paths, &decls, &gaps, &obstacles, &ends);
    paths
}

/// Route one wire, retrying alternate (unforced) edges when the preferred edge
/// is unroutable. Returns the assembled polyline and the edges actually used.
fn route_wire(
    spec: &SegmentSpec,
    endpoints: &super::endpoints::Endpoints,
    i: usize,
    grid: &Grid,
    obstacles: &[AbsBbox],
) -> (Vec<(f64, f64)>, Edge, Edge) {
    let (src0, se0) = (endpoints.src[i], endpoints.src_edge[i]);
    let (tgt0, te0) = (endpoints.tgt[i], endpoints.tgt_edge[i]);

    // Candidate edges per endpoint: the chosen one first, then the rest unless
    // a `.side` override forces it.
    let src_choices = edge_choices(se0, spec.src_forced.is_some());
    let tgt_choices = edge_choices(te0, spec.tgt_forced.is_some());

    // Sweep target edges first (the common sealed case), then source, then both.
    // The lane-allocated points are kept while their edge is unchanged; an
    // alternate edge attaches at its midpoint and track assignment respreads.
    for &te in &tgt_choices {
        let tgt = if te == te0 {
            tgt0
        } else {
            edge_midpoint(&spec.tgt_bbox, te)
        };
        if let Some(p) = try_route(grid, src0, se0, tgt, te, obstacles) {
            return (p, se0, te);
        }
    }
    for &se in &src_choices {
        let src = if se == se0 {
            src0
        } else {
            edge_midpoint(&spec.src_bbox, se)
        };
        if let Some(p) = try_route(grid, src, se, tgt0, te0, obstacles) {
            return (p, se, te0);
        }
    }
    for &se in &src_choices {
        for &te in &tgt_choices {
            let src = if se == se0 {
                src0
            } else {
                edge_midpoint(&spec.src_bbox, se)
            };
            let tgt = if te == te0 {
                tgt0
            } else {
                edge_midpoint(&spec.tgt_bbox, te)
            };
            if let Some(p) = try_route(grid, src, se, tgt, te, obstacles) {
                return (p, se, te);
            }
        }
    }

    // Nothing routes (the endpoint is genuinely boxed in) — emit the orthogonal
    // fallback elbow on the preferred edges and let the validator flag it.
    let ss = step_out(src0, se0, MIN_STUB);
    let ts = step_out(tgt0, te0, MIN_STUB);
    (assemble(src0, ss, ts, tgt0, None), se0, te0)
}

/// The edges to try for an endpoint: the preferred one, then the others (unless
/// a `.side` override pins it).
fn edge_choices(preferred: Edge, forced: bool) -> Vec<Edge> {
    if forced {
        return vec![preferred];
    }
    let mut out = vec![preferred];
    out.extend(ALL_EDGES.into_iter().filter(|&e| e != preferred));
    out
}

/// Attempt a single A* route between two attachment points; `None` if A* finds
/// no clear path (so the caller can try another edge).
fn try_route(
    grid: &Grid,
    src: (f64, f64),
    src_edge: Edge,
    tgt: (f64, f64),
    tgt_edge: Edge,
    obstacles: &[AbsBbox],
) -> Option<Vec<(f64, f64)>> {
    let ss = step_out(src, src_edge, MIN_STUB);
    let ts = step_out(tgt, tgt_edge, MIN_STUB);
    let mid = astar::route(grid, ss, ts, obstacles)?;
    Some(assemble(src, ss, ts, tgt, Some(mid)))
}

/// The range an endpoint may slide while staying on its edge (and so staying
/// perpendicular). A left/right edge lets the wire's `y` roam; a top/bottom edge
/// lets its `x` roam.
fn edge_clamp(b: &AbsBbox, edge: Edge) -> End {
    if edge.is_horizontal_exit() {
        End {
            horizontal: true,
            lo: b.y + EDGE_INSET,
            hi: b.bottom() - EDGE_INSET,
        }
    } else {
        End {
            horizontal: false,
            lo: b.x + EDGE_INSET,
            hi: b.right() - EDGE_INSET,
        }
    }
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
