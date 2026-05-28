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

/// Per-bend cost when comparing candidate routes (matches A*'s internal `BEND`,
/// so edge choice and in-search choice rank routes the same way).
const BEND_COST: f64 = 20.0;

/// Penalty for leaving from a non-geometry-preferred edge, so a wire keeps its
/// natural edge unless an alternate is clearly shorter (e.g. the near edge is
/// sealed). Less than one bend — a tiebreak, not an override.
const EDGE_BIAS: f64 = 12.0;

/// A preferred-edge route this much longer than the straight shape-to-shape
/// distance triggers the full edge search (it might reach the target from a
/// better side). Below it, the preferred route is good enough — skip the search.
const DETOUR_OK: f64 = 1.6;

/// Routed length over the straight (Manhattan) shape-centre distance — the same
/// edge-choice-independent detour measure the quality gate uses.
fn detour(path: &[(f64, f64)], spec: &SegmentSpec) -> f64 {
    let c0 = (spec.src_bbox.cx(), spec.src_bbox.cy());
    let c1 = (spec.tgt_bbox.cx(), spec.tgt_bbox.cy());
    let ideal = ((c0.0 - c1.0).abs() + (c0.1 - c1.1).abs()).max(1.0);
    let len: f64 = path
        .windows(2)
        .map(|w| (w[0].0 - w[1].0).abs() + (w[0].1 - w[1].1).abs())
        .sum();
    len / ideal
}

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

    // Base lattice: every shape's clearance-inflated edges + the world frame +
    // each wire's preferred attachment and stub. Lean on purpose — alternate
    // edges are only seeded for the few wires that actually need them (below),
    // so the common case routes on a small grid.
    let inflated: Vec<AbsBbox> = scene
        .all_boxes()
        .into_iter()
        .map(|(path, b)| b.inflate(oracle::shape_clearance(scene, &path)))
        .collect();
    let mut xs = Vec::new();
    let mut ys = Vec::new();
    for i in 0..specs.len() {
        for p in [
            endpoints.src[i],
            step_out(endpoints.src[i], endpoints.src_edge[i], MIN_STUB),
            endpoints.tgt[i],
            step_out(endpoints.tgt[i], endpoints.tgt_edge[i], MIN_STUB),
        ] {
            xs.push(p.0);
            ys.push(p.1);
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

    // Pass 1 — route each wire from its preferred edges (one A* on the lean
    // grid). A wire is "hard" if that fails or detours badly; only those pay for
    // the full edge search.
    let mut paths = Vec::with_capacity(specs.len());
    let mut src_edges: Vec<Edge> = endpoints.src_edge.clone();
    let mut tgt_edges: Vec<Edge> = endpoints.tgt_edge.clone();
    let mut hard = Vec::new();
    for (i, spec) in specs.iter().enumerate() {
        let direct = try_route(
            &grid,
            endpoints.src[i],
            src_edges[i],
            endpoints.tgt[i],
            tgt_edges[i],
            &obstacles[i],
        );
        match direct {
            Some(p) if detour(&p, spec) <= DETOUR_OK => paths.push(p),
            _ => {
                hard.push(i);
                paths.push(Vec::new()); // placeholder, filled in pass 2
            }
        }
    }

    // Pass 2 — full edge search for the hard wires, on a grid augmented with
    // their (and only their) alternate-edge attachment nodes.
    if !hard.is_empty() {
        for &i in &hard {
            for e in ALL_EDGES {
                for b in [&specs[i].src_bbox, &specs[i].tgt_bbox] {
                    let m = edge_midpoint(b, e);
                    let s = step_out(m, e, MIN_STUB);
                    xs.extend([m.0, s.0]);
                    ys.extend([m.1, s.1]);
                }
            }
        }
        let grid2 = Grid::build(&inflated, world, &xs, &ys);
        for &i in &hard {
            let (path, se, te) = route_wire(&specs[i], &endpoints, i, &grid2, &obstacles[i]);
            paths[i] = path;
            src_edges[i] = se;
            tgt_edges[i] = te;
        }
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

/// A scored candidate route during the edge search.
struct Candidate {
    cost: f64,
    path: Vec<(f64, f64)>,
    src_edge: Edge,
    tgt_edge: Edge,
}

/// Route one wire, choosing the **best** edge pair rather than the first that
/// works. The geometry-preferred edges are tried, plus every other unforced
/// edge; among all routable candidates the cheapest (fewest bends, then
/// shortest) wins, with a bias toward the preferred edges so a wire only leaves
/// from elsewhere when that is genuinely shorter (e.g. its near edge is sealed
/// by a neighbour's clearance). This is what keeps a wire from wrapping the
/// canvas to enter a shape from its far side.
fn route_wire(
    spec: &SegmentSpec,
    endpoints: &super::endpoints::Endpoints,
    i: usize,
    grid: &Grid,
    obstacles: &[AbsBbox],
) -> (Vec<(f64, f64)>, Edge, Edge) {
    let (src0, se0) = (endpoints.src[i], endpoints.src_edge[i]);
    let (tgt0, te0) = (endpoints.tgt[i], endpoints.tgt_edge[i]);
    let src_choices = edge_choices(se0, spec.src_forced.is_some());
    let tgt_choices = edge_choices(te0, spec.tgt_forced.is_some());

    let mut best: Option<Candidate> = None;
    for &se in &src_choices {
        for &te in &tgt_choices {
            // Keep the lane-allocated attachment while the edge is unchanged; an
            // alternate edge attaches at its midpoint (track assignment will
            // respread it).
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
            let Some(path) = try_route(grid, src, se, tgt, te, obstacles) else {
                continue;
            };
            let switched = (se != se0) as u32 + (te != te0) as u32;
            let cost = route_cost(&path) + f64::from(switched) * EDGE_BIAS;
            if best.as_ref().map_or(true, |b| cost < b.cost) {
                best = Some(Candidate {
                    cost,
                    path,
                    src_edge: se,
                    tgt_edge: te,
                });
            }
        }
    }
    if let Some(b) = best {
        return (b.path, b.src_edge, b.tgt_edge);
    }

    // Nothing routes (the endpoint is genuinely boxed in) — emit the orthogonal
    // fallback elbow on the preferred edges and let the validator flag it.
    let ss = step_out(src0, se0, MIN_STUB);
    let ts = step_out(tgt0, te0, MIN_STUB);
    (assemble(src0, ss, ts, tgt0, None), se0, te0)
}

/// Cost of a candidate route: length plus a per-bend penalty (spec §6 ranks
/// fewer bends above raw length, so each bend is worth a generous slice of
/// pixels). Mirrors the A* objective so edge choice and in-search choice agree.
fn route_cost(p: &[(f64, f64)]) -> f64 {
    let len: f64 = p
        .windows(2)
        .map(|w| (w[0].0 - w[1].0).abs() + (w[0].1 - w[1].1).abs())
        .sum();
    let bends = p
        .windows(3)
        .filter(|w| {
            let a = (w[0].1 - w[1].1).abs() < 0.5;
            let b = (w[1].1 - w[2].1).abs() < 0.5;
            a != b
        })
        .count();
    len + bends as f64 * BEND_COST
}

/// The edges to try for an endpoint: the preferred one and the others (unless a
/// `.side` override pins it).
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
