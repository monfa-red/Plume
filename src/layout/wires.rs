//! Wire routing — obstacle-aware orthogonal A* (SPEC §10).
//!
//! For each wire segment we:
//!   1. Pick entry / exit edges by relative geometry (nearest edge).
//!   2. Apply a lane offset along the chosen edges. Parallel wires (same
//!      pair of endpoints) get distinct offsets BEFORE A* runs, so each
//!      wire computes its own path on its own track instead of stacking
//!      onto the leader and then being shifted post-hoc.
//!   3. Build a `CellMap`: per-cell state recording shapes (hard walls),
//!      previously-routed wires (with axis), and wire halos (perpendicular
//!      gap zones).
//!   4. Run A* on a coarse grid (cell size ≈ wire-gap / 2). Cost penalises
//!      bends so paths stay straight when they can.
//!   5. Each routed wire becomes a HARD obstacle for subsequent wires,
//!      with one carve-out: a perpendicular crossing is allowed at a
//!      moderate cost. This is what enforces PCB-style spacing.
//!   6. Fall back through a hierarchy: walls+wires → walls only → no
//!      obstacles → straight line.

use super::ir::{PlacedNode, RoutedText, RoutedWire};
use super::values::layout_var;
use crate::ast::Side;
use crate::error::Error;
use crate::resolve::{
    MarkerKind, Markers, Program, ResolvedText, ResolvedValue, ResolvedWire, VarTable, WireAt,
};
use crate::span::Span;
use std::cmp::Ordering;
use std::collections::{BinaryHeap, HashMap};

pub fn route_wires(
    program: &Program,
    scene_nodes: &[PlacedNode],
) -> Result<Vec<RoutedWire>, Error> {
    let scene = SceneIndex::build(scene_nodes);

    // Explode chains into per-segment specs (one A* run = one segment).
    let specs = plan_segments(program, &scene)?;

    // Group specs into *bundles*: segments that share the same source and
    // target shapes AND the same chosen edges on both sides. A bundle of
    // size N becomes N parallel rails sharing a single A* route, offset
    // perpendicularly. Bundles of size 1 are routed normally (same as
    // Phase 1).
    let bundles = group_bundles(&specs);

    // Assign lane offsets per bundle within each (shape, edge) bin. Bundles
    // get contiguous lane ranges; the bundle's lane offset is the centre of
    // its range. This gives us "fanning per bundle" while keeping siblings
    // adjacent for the perpendicular-shift trick to produce visible rails.
    let bundle_lanes = assign_bundle_lanes(&bundles, &specs);

    // One shared grid sized to fit all wires comfortably.
    let max_gap = specs.iter().map(|s| s.gap).fold(0.0_f64, f64::max).max(8.0);
    let bounds = scene.bounds(max_gap);
    let grid = Grid::new(bounds, (max_gap / 2.0).max(4.0));

    let mut routed: Vec<Option<RoutedWire>> = (0..specs.len()).map(|_| None).collect();
    let mut routed_paths: Vec<Vec<(f64, f64)>> = Vec::with_capacity(specs.len());

    for (bi, bundle) in bundles.iter().enumerate() {
        let (src_lane, tgt_lane) = bundle_lanes[bi];
        let canonical_spec = &specs[bundle.spec_indices[0]];
        let canonical_path = route_segment_with_lanes(
            canonical_spec,
            &scene,
            &grid,
            src_lane,
            tgt_lane,
            &routed_paths,
        );

        // Stamp siblings by perpendicular shift, centred around the canonical.
        let size = bundle.spec_indices.len();
        let centre = (size as f64 - 1.0) / 2.0;
        for (k, &spec_idx) in bundle.spec_indices.iter().enumerate() {
            let shift = (k as f64 - centre) * canonical_spec.gap;
            let path = if shift.abs() < 0.5 {
                canonical_path.clone()
            } else {
                shift_polyline(&canonical_path, shift)
            };
            routed_paths.push(path.clone());
            routed[spec_idx] = Some(build_routed_wire(&specs[spec_idx], path));
        }
    }

    Ok(routed.into_iter().map(Option::unwrap).collect())
}

// ─────────────────────────── Bundles ───────────────────────────

/// A group of segments sharing the same source shape + source edge AND
/// the same target shape + target edge. They are routed as a single
/// "bus" — one canonical A* path, then siblings stamped by perpendicular
/// shift.
struct Bundle {
    src_id: String,
    src_edge: Edge,
    tgt_id: String,
    tgt_edge: Edge,
    /// Indices into the original `specs` array, in source order.
    spec_indices: Vec<usize>,
}

fn group_bundles(specs: &[SegmentSpec]) -> Vec<Bundle> {
    type Key = (String, Edge, String, Edge);
    let mut by_key: HashMap<Key, usize> = HashMap::new();
    let mut bundles: Vec<Bundle> = Vec::new();
    for (i, spec) in specs.iter().enumerate() {
        let key = (
            spec.src_id.clone(),
            spec.src_edge,
            spec.tgt_id.clone(),
            spec.tgt_edge,
        );
        let bi = *by_key.entry(key.clone()).or_insert_with(|| {
            bundles.push(Bundle {
                src_id: key.0.clone(),
                src_edge: key.1,
                tgt_id: key.2.clone(),
                tgt_edge: key.3,
                spec_indices: Vec::new(),
            });
            bundles.len() - 1
        });
        bundles[bi].spec_indices.push(i);
    }
    bundles
}

/// For each (shape, edge), pack bundles into contiguous lane ranges. Each
/// bundle's lane offset is the centre of its range — that's the position
/// the canonical wire occupies; siblings shift symmetrically around it.
///
/// Lanes are centred around 0 across the whole bin so total bin width is
/// `total_lanes × gap`.
fn assign_bundle_lanes(bundles: &[Bundle], specs: &[SegmentSpec]) -> Vec<(f64, f64)> {
    let mut lanes = vec![(0.0_f64, 0.0_f64); bundles.len()];
    place_lanes(bundles, specs, &mut lanes, BinSide::Src);
    place_lanes(bundles, specs, &mut lanes, BinSide::Tgt);
    lanes
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum BinSide {
    Src,
    Tgt,
}

fn place_lanes(bundles: &[Bundle], specs: &[SegmentSpec], lanes: &mut [(f64, f64)], side: BinSide) {
    // Group bundles by (shape, edge) bin, preserving creation order.
    let mut bins: HashMap<(String, Edge), Vec<usize>> = HashMap::new();
    for (bi, bundle) in bundles.iter().enumerate() {
        let key = match side {
            BinSide::Src => (bundle.src_id.clone(), bundle.src_edge),
            BinSide::Tgt => (bundle.tgt_id.clone(), bundle.tgt_edge),
        };
        bins.entry(key).or_default().push(bi);
    }
    for bundle_idxs in bins.values() {
        let total: usize = bundle_idxs
            .iter()
            .map(|&bi| bundles[bi].spec_indices.len())
            .sum();
        if total <= 1 {
            continue;
        }
        // All wires in a bin share their `gap` — pick any.
        let gap = specs[bundles[bundle_idxs[0]].spec_indices[0]].gap;
        // Each bundle occupies a contiguous run of lanes; cursor walks the bin.
        let mut cursor: usize = 0;
        for &bi in bundle_idxs {
            let size = bundles[bi].spec_indices.len();
            let centre_lane = cursor as f64 + (size as f64 - 1.0) / 2.0;
            let offset = (centre_lane - (total as f64 - 1.0) / 2.0) * gap;
            match side {
                BinSide::Src => lanes[bi].0 = offset,
                BinSide::Tgt => lanes[bi].1 = offset,
            }
            cursor += size;
        }
    }
}

// ─────────────────────────── Polyline perpendicular shift ───────────────────────────

/// Shift an orthogonal polyline by `delta` perpendicular to each segment.
/// Horizontal segments move on the y axis; vertical segments move on the x
/// axis. At each bend the corner is replaced by the intersection of the
/// two shifted lines — so straight bits stay parallel to the original and
/// corners track the bend topology. Used to stamp bundle siblings.
fn shift_polyline(path: &[(f64, f64)], delta: f64) -> Vec<(f64, f64)> {
    if path.len() < 2 {
        return path.to_vec();
    }
    // Translate each segment perpendicular to its axis.
    let mut shifted: Vec<((f64, f64), (f64, f64))> = Vec::with_capacity(path.len() - 1);
    for w in path.windows(2) {
        let (a, b) = (w[0], w[1]);
        let dy = (b.1 - a.1).abs();
        let dx = (b.0 - a.0).abs();
        let segment = if dy < 0.5 {
            // Horizontal — shift y.
            ((a.0, a.1 + delta), (b.0, b.1 + delta))
        } else if dx < 0.5 {
            // Vertical — shift x.
            ((a.0 + delta, a.1), (b.0 + delta, b.1))
        } else {
            // Diagonal — defensive: shouldn't occur for orthogonal routes.
            (a, b)
        };
        shifted.push(segment);
    }

    let mut out = Vec::with_capacity(shifted.len() + 1);
    out.push(shifted[0].0);
    for pair in shifted.windows(2) {
        let (a1, b1) = pair[0];
        let (a2, _) = pair[1];
        out.push(intersect_orthogonal(a1, b1, a2));
    }
    out.push(shifted.last().unwrap().1);
    out
}

/// Intersection of two perpendicular axis-aligned lines: one passes through
/// `a1`–`b1`, the other passes through `a2` and is perpendicular to the first.
fn intersect_orthogonal(a1: (f64, f64), b1: (f64, f64), a2: (f64, f64)) -> (f64, f64) {
    let horizontal_first = (a1.1 - b1.1).abs() < 0.5;
    if horizontal_first {
        // First line: y = a1.1. Second line: x = a2.0 (vertical).
        (a2.0, a1.1)
    } else {
        // First line: x = a1.0 (vertical). Second line: y = a2.1.
        (a1.0, a2.1)
    }
}

// ─────────────────────────── Per-segment orchestration ───────────────────────────

/// One wire SEGMENT — chains explode into one spec per link. Holds the bits
/// of state we need both for the lane-counting pre-pass and for the routing
/// pass without having to re-look-up shapes or re-pick edges.
struct SegmentSpec<'a> {
    wire: &'a ResolvedWire,
    src_id: String,
    tgt_id: String,
    src_edge: Edge,
    tgt_edge: Edge,
    /// Endpoint `.side` overrides — when `Some`, A* uses this edge instead of
    /// picking the cheapest from the multi-edge candidate set.
    src_forced: Option<Edge>,
    tgt_forced: Option<Edge>,
    src_bbox: AbsBbox,
    tgt_bbox: AbsBbox,
    gap: f64,
    /// True iff this is the first segment in its chain — only the first
    /// segment carries the chain's start marker and wire-text labels.
    is_first: bool,
    /// True iff this is the last segment in its chain.
    is_last: bool,
    data_from: String,
    data_to: String,
}

fn plan_segments<'a>(
    program: &'a Program,
    scene: &SceneIndex,
) -> Result<Vec<SegmentSpec<'a>>, Error> {
    let mut out = Vec::new();
    for wire in &program.wires {
        let n = wire.endpoints.len();
        let from_id = wire.endpoints.first().unwrap().path.clone();
        let to_id = wire.endpoints.last().unwrap().path.clone();
        let gap = wire_gap(wire, &program.vars);
        for i in 0..(n - 1) {
            let src_id = wire.endpoints[i].path.clone();
            let tgt_id = wire.endpoints[i + 1].path.clone();
            if src_id == tgt_id {
                return Err(Error::at(
                    wire.span,
                    "self-loops are not yet routed (SPEC §9 self-loop is deferred)",
                ));
            }
            let src = scene
                .lookup(&src_id)
                .ok_or_else(|| undefined_wire_id(&src_id, wire.endpoints[i].span))?;
            let tgt = scene
                .lookup(&tgt_id)
                .ok_or_else(|| undefined_wire_id(&tgt_id, wire.endpoints[i + 1].span))?;
            let src_forced = wire.endpoints[i].side.map(side_to_edge);
            let tgt_forced = wire.endpoints[i + 1].side.map(side_to_edge);
            let src_edge = src_forced
                .unwrap_or_else(|| nearest_edge(&src.bbox, (tgt.bbox.cx(), tgt.bbox.cy())));
            let tgt_edge = tgt_forced
                .unwrap_or_else(|| nearest_edge(&tgt.bbox, (src.bbox.cx(), src.bbox.cy())));
            out.push(SegmentSpec {
                wire,
                src_id,
                tgt_id,
                src_edge,
                tgt_edge,
                src_forced,
                tgt_forced,
                src_bbox: src.bbox,
                tgt_bbox: tgt.bbox,
                gap,
                is_first: i == 0,
                is_last: i == n - 2,
                data_from: from_id.clone(),
                data_to: to_id.clone(),
            });
        }
    }
    Ok(out)
}

fn route_segment_with_lanes(
    spec: &SegmentSpec,
    scene: &SceneIndex,
    grid: &Grid,
    src_lane: f64,
    tgt_lane: f64,
    prior_paths: &[Vec<(f64, f64)>],
) -> Vec<(f64, f64)> {
    let shape_obstacles = scene.obstacles_for(&spec.src_id, &spec.tgt_id, spec.gap);

    // Build three cell-map tiers up front. Each one represents a relaxation
    // of constraints — A* tries them in order until one finds a path.
    let walls_and_wires = {
        let mut m = CellMap::new(grid);
        m.mark_walls(grid, &shape_obstacles);
        for p in prior_paths {
            m.mark_wire_path(grid, p);
        }
        m
    };
    let walls_only = {
        let mut m = CellMap::new(grid);
        m.mark_walls(grid, &shape_obstacles);
        m
    };
    let empty = CellMap::new(grid);

    // Edge candidates: 3 per side normally. When the endpoint carries an
    // explicit `.side` override, we pin to that single edge.
    let src_edges = match spec.src_forced {
        Some(e) => vec![e],
        None => candidate_edges(&spec.src_bbox, &spec.tgt_bbox),
    };
    let tgt_edges = match spec.tgt_forced {
        Some(e) => vec![e],
        None => candidate_edges(&spec.tgt_bbox, &spec.src_bbox),
    };
    type Candidate = (i64, Edge, Edge, Vec<(usize, usize)>);
    let mut best: Option<Candidate> = None;

    for cells in [&walls_and_wires, &walls_only, &empty] {
        for &se in &src_edges {
            for &te in &tgt_edges {
                let start = grid.cell_outside(&spec.src_bbox, se, spec.gap, src_lane);
                let goal = grid.cell_outside(&spec.tgt_bbox, te, spec.gap, tgt_lane);
                if let Some((cells, cost)) = a_star(grid, start, goal, se, te, cells) {
                    if best.as_ref().map_or(true, |b| cost < b.0) {
                        best = Some((cost, se, te, cells));
                    }
                }
            }
        }
        if best.is_some() {
            break;
        }
    }

    let Some((_, src_edge, tgt_edge, cells)) = best else {
        // Final fallback: straight line. We still need anchor points.
        let pt = edge_midpoint(&spec.src_bbox, spec.src_edge);
        return vec![pt, edge_midpoint(&spec.tgt_bbox, spec.tgt_edge)];
    };

    // Snap the lane-shifted endpoints to the grid row/column that A*
    // actually picked. Without this, the first segment from `src_pt` to
    // `grid.cell_center(start)` ends up with a 1–3 px perpendicular kink
    // (the grid is discrete, the lane shift is continuous). Snapping
    // collapses that kink to zero.
    let src_pt = snap_to_cell(
        lane_shift(
            edge_midpoint(&spec.src_bbox, src_edge),
            src_edge,
            src_lane,
            &spec.src_bbox,
        ),
        src_edge,
        cells.first().copied(),
        grid,
    );
    let tgt_pt = snap_to_cell(
        lane_shift(
            edge_midpoint(&spec.tgt_bbox, tgt_edge),
            tgt_edge,
            tgt_lane,
            &spec.tgt_bbox,
        ),
        tgt_edge,
        cells.last().copied(),
        grid,
    );

    assemble_path(src_pt, src_edge, &cells, tgt_pt, tgt_edge, grid)
}

/// For `self_bbox` connecting to `other_bbox`, return the edges of
/// `self_bbox` worth considering as the entry/exit point — that is, every
/// edge except the one strictly facing AWAY from `other_bbox`. With
/// `other` to the right, the Left edge is excluded; with `other` below,
/// the Top edge is excluded. Perpendicular edges are kept because they
/// can produce a shorter route when the direct edge is blocked.
fn candidate_edges(self_bbox: &AbsBbox, other_bbox: &AbsBbox) -> Vec<Edge> {
    let dx = other_bbox.cx() - self_bbox.cx();
    let dy = other_bbox.cy() - self_bbox.cy();
    let mut out = Vec::with_capacity(4);
    for e in [Edge::Right, Edge::Bottom, Edge::Left, Edge::Top] {
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
    if out.is_empty() {
        // Degenerate case (perfectly overlapping centres) — fall back to all.
        out.extend_from_slice(&[Edge::Right, Edge::Bottom, Edge::Left, Edge::Top]);
    }
    out
}

/// Align `pt` with the adjacent A* cell on the edge's perpendicular axis,
/// so the first/last segment is purely horizontal or vertical. For a
/// Right/Left edge the cell dictates `y`; for Top/Bottom it dictates `x`.
fn snap_to_cell(
    pt: (f64, f64),
    edge: Edge,
    cell: Option<(usize, usize)>,
    grid: &Grid,
) -> (f64, f64) {
    let Some(c) = cell else {
        return pt;
    };
    let cc = grid.cell_center(c);
    match edge {
        Edge::Right | Edge::Left => (pt.0, cc.1),
        Edge::Top | Edge::Bottom => (cc.0, pt.1),
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

fn assemble_path(
    src_pt: (f64, f64),
    src_edge: Edge,
    cells: &[(usize, usize)],
    tgt_pt: (f64, f64),
    tgt_edge: Edge,
    grid: &Grid,
) -> Vec<(f64, f64)> {
    let mut pts: Vec<(f64, f64)> = Vec::with_capacity(cells.len() + 4);
    pts.push(src_pt);
    // Insert a corner so src_pt → first_cell is strictly axis-aligned. The
    // source's chosen edge dictates which axis the first segment runs on.
    if let Some(&first) = cells.first() {
        let first_world = grid.cell_center(first);
        if let Some(corner) = bridge_corner(src_pt, first_world, src_edge) {
            pts.push(corner);
        }
    }
    for &c in cells {
        pts.push(grid.cell_center(c));
    }
    // Same trick for the tail: insert a corner so last_cell → tgt_pt is axis-aligned.
    if let Some(&last) = cells.last() {
        let last_world = grid.cell_center(last);
        if let Some(corner) = bridge_corner(tgt_pt, last_world, tgt_edge) {
            pts.push(corner);
        }
    }
    pts.push(tgt_pt);
    simplify(&pts)
}

/// Return the corner needed to make `inner → endpoint` strictly orthogonal,
/// where `endpoint` lies on a shape edge of orientation `edge`. The endpoint's
/// edge fixes which axis the connecting segment runs along: a Right/Left edge
/// means a horizontal segment, Top/Bottom means vertical. The corner sits at
/// the intersection.
fn bridge_corner(endpoint: (f64, f64), inner: (f64, f64), edge: Edge) -> Option<(f64, f64)> {
    if approx_eq(endpoint.0, inner.0) && approx_eq(endpoint.1, inner.1) {
        return None;
    }
    let corner = match edge {
        Edge::Right | Edge::Left => (inner.0, endpoint.1),
        Edge::Top | Edge::Bottom => (endpoint.0, inner.1),
    };
    // Skip the corner if it collapses onto either endpoint of the segment.
    if (approx_eq(corner.0, endpoint.0) && approx_eq(corner.1, endpoint.1))
        || (approx_eq(corner.0, inner.0) && approx_eq(corner.1, inner.1))
    {
        return None;
    }
    Some(corner)
}

fn simplify(pts: &[(f64, f64)]) -> Vec<(f64, f64)> {
    if pts.len() < 3 {
        return pts.to_vec();
    }
    let mut out: Vec<(f64, f64)> = vec![pts[0]];
    for i in 1..pts.len() - 1 {
        let a = out.last().copied().unwrap();
        let b = pts[i];
        let c = pts[i + 1];
        // Drop b if a → b → c is collinear (same x or same y on both legs).
        let collinear = (approx_eq(a.0, b.0) && approx_eq(b.0, c.0))
            || (approx_eq(a.1, b.1) && approx_eq(b.1, c.1));
        if !collinear {
            out.push(b);
        }
    }
    out.push(*pts.last().unwrap());
    out
}

fn approx_eq(a: f64, b: f64) -> bool {
    (a - b).abs() < 0.5
}

// ─────────────────────────── Wire gap ───────────────────────────

fn wire_gap(wire: &ResolvedWire, vars: &VarTable) -> f64 {
    // Wires-block-level `gap=N` (merged into each wire by resolve) wins,
    // else fall back to the layout var.
    if let Some(ResolvedValue::Number(n)) = wire.attrs.get("gap") {
        return *n;
    }
    layout_var(vars, "wire-gap").unwrap_or(16.0)
}

// ─────────────────────────── Scene index + obstacles ───────────────────────────

#[derive(Clone, Copy, Debug)]
pub struct AbsBbox {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

impl AbsBbox {
    fn cx(&self) -> f64 {
        self.x + self.w / 2.0
    }
    fn cy(&self) -> f64 {
        self.y + self.h / 2.0
    }
    fn inflate(&self, by: f64) -> AbsBbox {
        AbsBbox {
            x: self.x - by,
            y: self.y - by,
            w: self.w + 2.0 * by,
            h: self.h + 2.0 * by,
        }
    }
}

#[derive(Clone)]
struct ShapeRef {
    bbox: AbsBbox,
}

/// Flat-but-tree-aware index of every named scene node. Each entry remembers
/// its own bbox plus the chain of named ancestors so we can decide which
/// shapes count as obstacles for a given wire.
struct SceneIndex {
    /// One entry per named (`id`-having) node, in source order.
    nodes: Vec<IndexedNode>,
    /// Fully-qualified dot-path → index in `nodes`. Resolver canonicalises
    /// endpoint paths, so this is the lookup the routing pass needs.
    by_path: HashMap<String, usize>,
}

struct IndexedNode {
    bbox: AbsBbox,
    /// Indices into `nodes` for every ancestor that has an id, root-first.
    ancestors: Vec<usize>,
    is_leaf: bool,
}

impl SceneIndex {
    fn build(scene_nodes: &[PlacedNode]) -> Self {
        let mut idx = SceneIndex {
            nodes: Vec::new(),
            by_path: HashMap::new(),
        };
        for node in scene_nodes {
            idx.walk(node, 0.0, 0.0, &[], &mut Vec::new());
        }
        idx
    }

    fn walk(
        &mut self,
        node: &PlacedNode,
        parent_cx: f64,
        parent_cy: f64,
        ancestors: &[usize],
        path_stack: &mut Vec<String>,
    ) {
        let abs_cx = parent_cx + node.cx;
        let abs_cy = parent_cy + node.cy;
        let bbox = AbsBbox {
            x: abs_cx + node.bbox.min_x,
            y: abs_cy + node.bbox.min_y,
            w: node.bbox.w(),
            h: node.bbox.h(),
        };
        let mut next_ancestors = ancestors.to_vec();
        let mut pushed_path = false;
        if let Some(id) = &node.id {
            let i = self.nodes.len();
            path_stack.push(id.clone());
            let full_path = path_stack.join(".");
            pushed_path = true;
            self.nodes.push(IndexedNode {
                bbox,
                ancestors: ancestors.to_vec(),
                is_leaf: node.children.is_empty(),
            });
            self.by_path.insert(full_path, i);
            next_ancestors.push(i);
        }
        for child in &node.children {
            self.walk(child, abs_cx, abs_cy, &next_ancestors, path_stack);
        }
        if pushed_path {
            path_stack.pop();
        }
    }

    fn lookup(&self, path: &str) -> Option<ShapeRef> {
        let i = *self.by_path.get(path)?;
        Some(ShapeRef {
            bbox: self.nodes[i].bbox,
        })
    }

    /// World bounds spanning every node, inflated by `pad` on every side so
    /// the grid has room for paths that leave the immediate area.
    fn bounds(&self, pad: f64) -> AbsBbox {
        if self.nodes.is_empty() {
            return AbsBbox {
                x: -50.0,
                y: -50.0,
                w: 100.0,
                h: 100.0,
            };
        }
        let mut min_x = f64::INFINITY;
        let mut min_y = f64::INFINITY;
        let mut max_x = f64::NEG_INFINITY;
        let mut max_y = f64::NEG_INFINITY;
        for n in &self.nodes {
            min_x = min_x.min(n.bbox.x);
            min_y = min_y.min(n.bbox.y);
            max_x = max_x.max(n.bbox.x + n.bbox.w);
            max_y = max_y.max(n.bbox.y + n.bbox.h);
        }
        AbsBbox {
            x: min_x - pad * 2.0,
            y: min_y - pad * 2.0,
            w: max_x - min_x + pad * 4.0,
            h: max_y - min_y + pad * 4.0,
        }
    }

    /// Obstacles for a wire between `src_id` and `tgt_id`. Each shape is an
    /// obstacle UNLESS it is an endpoint or an ancestor of an endpoint, in
    /// which case the path is allowed to cross its boundary.
    fn obstacles_for(&self, src_id: &str, tgt_id: &str, gap: f64) -> Vec<AbsBbox> {
        let src_i = self.by_path.get(src_id).copied();
        let tgt_i = self.by_path.get(tgt_id).copied();
        let mut passable: Vec<usize> = Vec::new();
        if let Some(i) = src_i {
            passable.push(i);
            passable.extend(self.nodes[i].ancestors.iter().copied());
        }
        if let Some(i) = tgt_i {
            passable.push(i);
            passable.extend(self.nodes[i].ancestors.iter().copied());
        }

        let mut out = Vec::new();
        for (i, n) in self.nodes.iter().enumerate() {
            if passable.contains(&i) {
                continue;
            }
            // A container only contributes its own frame if all its named
            // ancestors are passable (otherwise its outer container would
            // already cover it).
            if !n.ancestors.iter().all(|a| passable.contains(a)) {
                continue;
            }
            // Skip the text label children that sit at top of a group — they
            // overlap their parent and don't add information.
            if !n.is_leaf && n.bbox.w == 0.0 && n.bbox.h == 0.0 {
                continue;
            }
            out.push(n.bbox.inflate(gap));
        }
        out
    }
}

fn undefined_wire_id(id: &str, span: Span) -> Error {
    Error::at(span, format!("wire references undefined id '{}'", id))
}

// ─────────────────────────── Grid ───────────────────────────

struct Grid {
    bounds: AbsBbox,
    cell_size: f64,
    cols: usize,
    rows: usize,
}

impl Grid {
    fn new(bounds: AbsBbox, cell_size: f64) -> Self {
        let cols = ((bounds.w / cell_size).ceil() as usize).max(1);
        let rows = ((bounds.h / cell_size).ceil() as usize).max(1);
        Self {
            bounds,
            cell_size,
            cols,
            rows,
        }
    }

    fn world_to_cell(&self, p: (f64, f64)) -> (usize, usize) {
        let c = ((p.0 - self.bounds.x) / self.cell_size).floor() as isize;
        let r = ((p.1 - self.bounds.y) / self.cell_size).floor() as isize;
        (
            c.clamp(0, self.cols as isize - 1) as usize,
            r.clamp(0, self.rows as isize - 1) as usize,
        )
    }

    fn cell_center(&self, cell: (usize, usize)) -> (f64, f64) {
        (
            self.bounds.x + (cell.0 as f64 + 0.5) * self.cell_size,
            self.bounds.y + (cell.1 as f64 + 0.5) * self.cell_size,
        )
    }

    /// Pick the cell one step outside `bbox` in the direction of `edge`,
    /// padded by `gap` so we sit clear of any obstacle inflation. The
    /// `lane_offset` shifts along the edge — used so parallel wires start
    /// from different tracks.
    fn cell_outside(
        &self,
        bbox: &AbsBbox,
        edge: Edge,
        gap: f64,
        lane_offset: f64,
    ) -> (usize, usize) {
        let pad = gap + self.cell_size * 0.5;
        let p = match edge {
            Edge::Right => (bbox.x + bbox.w + pad, bbox.cy() + lane_offset),
            Edge::Left => (bbox.x - pad, bbox.cy() + lane_offset),
            Edge::Top => (bbox.cx() + lane_offset, bbox.y - pad),
            Edge::Bottom => (bbox.cx() + lane_offset, bbox.y + bbox.h + pad),
        };
        self.world_to_cell(p)
    }
}

// ─────────────────────────── CellMap ───────────────────────────
//
// Per-cell routing state, recorded as a packed bitfield. Each cell can carry
// any combination of:
//
//   WALL    — hard obstacle (shape bbox or shape halo). Nothing routes here.
//   WIRE_H  — a previously-routed wire passes through this cell horizontally.
//   WIRE_V  — same, vertically. WIRE_H+WIRE_V together = a wire bend cell.
//   HALO_H  — perpendicular gap of a horizontal wire (the row above/below
//             the wire's track). Blocks parallel horizontal approach;
//             vertical traversal passes through freely.
//   HALO_V  — same idea, for vertical wires.
//
// The combination gives us PCB-style spacing: a wire's track is impassable
// to anyone going the same direction (overlap), passable perpendicularly
// at cost (crossing), and surrounded by halo zones that block parallel-too-
// close approaches but allow perpendicular pass-through.

type Cell = u8;
const WALL: Cell = 1 << 0;
const WIRE_H: Cell = 1 << 1;
const WIRE_V: Cell = 1 << 2;
const HALO_H: Cell = 1 << 3;
const HALO_V: Cell = 1 << 4;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Axis {
    H,
    V,
}

struct CellMap {
    cols: usize,
    rows: usize,
    cells: Vec<Cell>,
}

impl CellMap {
    fn new(grid: &Grid) -> Self {
        Self {
            cols: grid.cols,
            rows: grid.rows,
            cells: vec![0; grid.cols * grid.rows],
        }
    }

    fn at(&self, cell: (usize, usize)) -> Cell {
        self.cells[cell.1 * self.cols + cell.0]
    }

    /// Mark every cell whose centre lies inside any obstacle bbox as WALL.
    /// Shapes are passed already inflated by `wire-gap`, so this also
    /// produces the shape's halo for free.
    fn mark_walls(&mut self, grid: &Grid, obstacles: &[AbsBbox]) {
        for ob in obstacles {
            let min_c = ((ob.x - grid.bounds.x) / grid.cell_size).floor().max(0.0) as usize;
            let min_r = ((ob.y - grid.bounds.y) / grid.cell_size).floor().max(0.0) as usize;
            let max_c =
                (((ob.x + ob.w - grid.bounds.x) / grid.cell_size).ceil() as usize).min(self.cols);
            let max_r =
                (((ob.y + ob.h - grid.bounds.y) / grid.cell_size).ceil() as usize).min(self.rows);
            for r in min_r..max_r {
                for c in min_c..max_c {
                    self.cells[r * self.cols + c] |= WALL;
                }
            }
        }
    }

    /// Walk a routed wire's polyline and mark its track (`WIRE_H` / `WIRE_V`)
    /// plus the parallel halo cells (`HALO_H` / `HALO_V`).
    fn mark_wire_path(&mut self, grid: &Grid, path: &[(f64, f64)]) {
        for window in path.windows(2) {
            let a = grid.world_to_cell(window[0]);
            let b = grid.world_to_cell(window[1]);
            if a == b {
                continue;
            }
            if a.1 == b.1 {
                self.mark_horizontal_segment(a, b);
            } else if a.0 == b.0 {
                self.mark_vertical_segment(a, b);
            }
            // Diagonal/empty segments shouldn't occur for orthogonal routes;
            // ignore them defensively.
        }
    }

    fn mark_horizontal_segment(&mut self, a: (usize, usize), b: (usize, usize)) {
        let r = a.1;
        let (c0, c1) = if a.0 <= b.0 { (a.0, b.0) } else { (b.0, a.0) };
        for c in c0..=c1 {
            self.cells[r * self.cols + c] |= WIRE_H;
        }
        if r > 0 {
            for c in c0..=c1 {
                self.cells[(r - 1) * self.cols + c] |= HALO_H;
            }
        }
        if r + 1 < self.rows {
            for c in c0..=c1 {
                self.cells[(r + 1) * self.cols + c] |= HALO_H;
            }
        }
    }

    fn mark_vertical_segment(&mut self, a: (usize, usize), b: (usize, usize)) {
        let c = a.0;
        let (r0, r1) = if a.1 <= b.1 { (a.1, b.1) } else { (b.1, a.1) };
        for r in r0..=r1 {
            self.cells[r * self.cols + c] |= WIRE_V;
        }
        if c > 0 {
            for r in r0..=r1 {
                self.cells[r * self.cols + c - 1] |= HALO_V;
            }
        }
        if c + 1 < self.cols {
            for r in r0..=r1 {
                self.cells[r * self.cols + c + 1] |= HALO_V;
            }
        }
    }

    /// Decide whether a wire moving along `axis` can step into this cell.
    /// Returns the entry cost adjustment, or `None` if blocked.
    ///
    /// The rule, in plain English:
    ///   - WALL: never enter.
    ///   - Cell has a wire on MY axis: would overlap → never enter.
    ///   - Cell has a wire on the OTHER axis: crossing — allowed at cost.
    ///   - Cell is a halo of a wire on MY axis: parallel-too-close → never.
    ///   - Cell is a halo of a wire on the OTHER axis: perpendicular pass
    ///     through the gap zone is fine — no cost penalty.
    ///   - Otherwise: free.
    fn entry_for(&self, cell: (usize, usize), axis: Axis) -> EntryRule {
        let s = self.cells[cell.1 * self.cols + cell.0];
        if s & WALL != 0 {
            return EntryRule::Blocked;
        }
        let (my_wire, my_halo, cross_wire) = match axis {
            Axis::H => (WIRE_H, HALO_H, WIRE_V),
            Axis::V => (WIRE_V, HALO_V, WIRE_H),
        };
        if s & my_wire != 0 || s & my_halo != 0 {
            return EntryRule::Blocked;
        }
        if s & cross_wire != 0 {
            return EntryRule::Cross;
        }
        EntryRule::Free
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum EntryRule {
    Free,
    /// Crossing a perpendicular wire — allowed at moderate cost.
    Cross,
    Blocked,
}

// ─────────────────────────── A* ───────────────────────────

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
enum Dir {
    None,
    Right,
    Left,
    Up,
    Down,
}

#[derive(Clone, Copy)]
struct Node {
    f_cost: i64,
    g_cost: i64,
    cell: (usize, usize),
    dir: Dir,
}

impl PartialEq for Node {
    fn eq(&self, other: &Self) -> bool {
        self.f_cost == other.f_cost
    }
}
impl Eq for Node {}
impl Ord for Node {
    fn cmp(&self, other: &Self) -> Ordering {
        other.f_cost.cmp(&self.f_cost) // min-heap via reversed compare
    }
}
impl PartialOrd for Node {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

type AStarKey = ((usize, usize), Dir);

/// A* on the wire-routing grid. Cost shape:
///
///   step:    1 per cell
///   bend:    +4 when the direction changes
///   cross:   +8 when stepping onto a cell occupied by a perpendicular wire
///   blocked: cell is a WALL or would overlap a same-axis wire
///
/// Crossings carry one extra rule: when we step onto a perpendicular-wire
/// cell we MUST exit in the same direction we entered (no bend at the
/// cross). Otherwise we'd be tracing along an existing wire instead of
/// truly crossing it. This is enforced by inspecting the current cell's
/// `WIRE_H` / `WIRE_V` flags relative to the incoming direction.
fn a_star(
    grid: &Grid,
    start: (usize, usize),
    goal: (usize, usize),
    src_edge: Edge,
    tgt_edge: Edge,
    cells: &CellMap,
) -> Option<(Vec<(usize, usize)>, i64)> {
    const BEND: i64 = 4;
    const CROSS: i64 = 8;

    let start_dir = preferred_dir(src_edge);
    let goal_dir = opposite(preferred_dir(tgt_edge));

    let mut open = BinaryHeap::new();
    let mut came_from: HashMap<AStarKey, AStarKey> = HashMap::new();
    let mut best_g: HashMap<AStarKey, i64> = HashMap::new();

    open.push(Node {
        f_cost: 0,
        g_cost: 0,
        cell: start,
        dir: start_dir,
    });
    best_g.insert((start, start_dir), 0);

    while let Some(node) = open.pop() {
        if node.cell == goal {
            let mut path = vec![node.cell];
            let mut key = (node.cell, node.dir);
            while let Some(prev) = came_from.get(&key) {
                path.push(prev.0);
                key = *prev;
                if prev.0 == start {
                    break;
                }
            }
            path.reverse();
            // One extra step in `goal_dir` so the entry-axis snap in
            // `simplify` lines up with the target edge.
            if let Some(extra) = step(node.cell, goal_dir, grid) {
                if extra != node.cell {
                    path.push(extra);
                }
            }
            return Some((path, node.g_cost));
        }

        // If we just entered this cell by crossing a perpendicular wire, the
        // only legal next step is to continue straight. Otherwise we'd bend
        // ONTO the wire we just crossed, which is the same as overlap.
        let must_continue = perpendicular_cross_here(cells, node.cell, node.dir);

        for &d in &[Dir::Right, Dir::Left, Dir::Up, Dir::Down] {
            if must_continue && d != node.dir {
                continue;
            }
            let next = match step(node.cell, d, grid) {
                Some(c) => c,
                None => continue,
            };

            // Source/target cells are always reachable — they're our anchors,
            // not obstacles to ourselves.
            let is_endpoint = next == goal || next == start;
            let entry = if is_endpoint {
                EntryRule::Free
            } else {
                cells.entry_for(next, axis_of(d))
            };
            let cross_cost = match entry {
                EntryRule::Blocked => continue,
                EntryRule::Free => 0,
                EntryRule::Cross => CROSS,
            };

            let mut step_cost = 1_i64;
            if node.dir != Dir::None && node.dir != d {
                step_cost += BEND;
            }
            step_cost += cross_cost;

            let g = node.g_cost + step_cost;
            let key = (next, d);
            if let Some(&prev_g) = best_g.get(&key) {
                if g >= prev_g {
                    continue;
                }
            }
            best_g.insert(key, g);
            came_from.insert(key, (node.cell, node.dir));
            let h = manhattan(next, goal) as i64;
            open.push(Node {
                f_cost: g + h,
                g_cost: g,
                cell: next,
                dir: d,
            });
        }
    }
    None
}

fn axis_of(d: Dir) -> Axis {
    match d {
        Dir::Right | Dir::Left => Axis::H,
        Dir::Up | Dir::Down | Dir::None => Axis::V,
    }
}

/// True iff arriving at `cell` going `dir` placed us *on top of* a
/// perpendicular wire. In that case we must continue straight to leave.
fn perpendicular_cross_here(cells: &CellMap, cell: (usize, usize), dir: Dir) -> bool {
    if matches!(dir, Dir::None) {
        return false;
    }
    let s = cells.at(cell);
    match axis_of(dir) {
        Axis::H => s & WIRE_V != 0,
        Axis::V => s & WIRE_H != 0,
    }
}

fn step(c: (usize, usize), d: Dir, grid: &Grid) -> Option<(usize, usize)> {
    // `.then_some(...)` evaluates eagerly — guard with `if` to avoid usize underflow.
    match d {
        Dir::Right if c.0 + 1 < grid.cols => Some((c.0 + 1, c.1)),
        Dir::Left if c.0 > 0 => Some((c.0 - 1, c.1)),
        Dir::Down if c.1 + 1 < grid.rows => Some((c.0, c.1 + 1)),
        Dir::Up if c.1 > 0 => Some((c.0, c.1 - 1)),
        _ => None,
    }
}

fn manhattan(a: (usize, usize), b: (usize, usize)) -> usize {
    a.0.abs_diff(b.0) + a.1.abs_diff(b.1)
}

fn preferred_dir(edge: Edge) -> Dir {
    match edge {
        Edge::Right => Dir::Right,
        Edge::Left => Dir::Left,
        Edge::Top => Dir::Up,
        Edge::Bottom => Dir::Down,
    }
}

fn opposite(d: Dir) -> Dir {
    match d {
        Dir::Right => Dir::Left,
        Dir::Left => Dir::Right,
        Dir::Up => Dir::Down,
        Dir::Down => Dir::Up,
        Dir::None => Dir::None,
    }
}

// ─────────────────────────── Edge selection ───────────────────────────

#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
enum Edge {
    Right,
    Bottom,
    Left,
    Top,
}

fn side_to_edge(s: Side) -> Edge {
    match s {
        Side::Top => Edge::Top,
        Side::Bottom => Edge::Bottom,
        Side::Left => Edge::Left,
        Side::Right => Edge::Right,
    }
}

fn nearest_edge(my: &AbsBbox, other: (f64, f64)) -> Edge {
    let dx = other.0 - my.cx();
    let dy = other.1 - my.cy();
    let adx = dx.abs();
    let ady = dy.abs();
    if adx > ady {
        if dx >= 0.0 {
            Edge::Right
        } else {
            Edge::Left
        }
    } else if ady > adx {
        if dy >= 0.0 {
            Edge::Bottom
        } else {
            Edge::Top
        }
    } else if dx >= 0.0 {
        Edge::Right
    } else if dy >= 0.0 {
        Edge::Bottom
    } else {
        Edge::Left
    }
}

fn edge_midpoint(bbox: &AbsBbox, e: Edge) -> (f64, f64) {
    match e {
        Edge::Right => (bbox.x + bbox.w, bbox.cy()),
        Edge::Left => (bbox.x, bbox.cy()),
        Edge::Top => (bbox.cx(), bbox.y),
        Edge::Bottom => (bbox.cx(), bbox.y + bbox.h),
    }
}

/// Shift an edge connection point along the edge by `lane_offset`, clamped
/// so the point stays on the shape's edge.
fn lane_shift(pt: (f64, f64), edge: Edge, lane_offset: f64, bbox: &AbsBbox) -> (f64, f64) {
    if lane_offset.abs() < 0.01 {
        return pt;
    }
    let inset = 4.0; // keep at least this far from the edge corner
    match edge {
        Edge::Top | Edge::Bottom => {
            let min_x = bbox.x + inset;
            let max_x = bbox.x + bbox.w - inset;
            ((pt.0 + lane_offset).clamp(min_x, max_x), pt.1)
        }
        Edge::Left | Edge::Right => {
            let min_y = bbox.y + inset;
            let max_y = bbox.y + bbox.h - inset;
            (pt.0, (pt.1 + lane_offset).clamp(min_y, max_y))
        }
    }
}

// ─────────────────────────── Text placement ───────────────────────────

fn place_texts(texts: &[ResolvedText], path: &[(f64, f64)]) -> Vec<RoutedText> {
    let mut out = Vec::with_capacity(texts.len());
    for t in texts {
        let fraction = match &t.at {
            WireAt::Start => 0.0,
            WireAt::Mid => 0.5,
            WireAt::End => 1.0,
            WireAt::Fraction(f) => *f,
        };
        let (pos, tangent) = point_at_fraction(path, fraction);
        out.push(RoutedText {
            content: t.text.clone(),
            position: pos,
            tangent,
            attrs: t.attrs.clone(),
        });
    }
    out
}

fn point_at_fraction(path: &[(f64, f64)], f: f64) -> ((f64, f64), (f64, f64)) {
    if path.is_empty() {
        return ((0.0, 0.0), (1.0, 0.0));
    }
    if path.len() == 1 {
        return (path[0], (1.0, 0.0));
    }
    let total: f64 = path.windows(2).map(|w| dist(w[0], w[1])).sum();
    let target = total * f.clamp(0.0, 1.0);
    let mut acc = 0.0;
    for w in path.windows(2) {
        let seg = dist(w[0], w[1]);
        if acc + seg >= target {
            let local_f = if seg > 0.0 { (target - acc) / seg } else { 0.0 };
            let x = w[0].0 + (w[1].0 - w[0].0) * local_f;
            let y = w[0].1 + (w[1].1 - w[0].1) * local_f;
            let dx = (w[1].0 - w[0].0) / seg.max(1e-9);
            let dy = (w[1].1 - w[0].1) / seg.max(1e-9);
            return ((x, y), (dx, dy));
        }
        acc += seg;
    }
    let last = *path.last().unwrap();
    let prev = path[path.len() - 2];
    let dx = last.0 - prev.0;
    let dy = last.1 - prev.1;
    let len = (dx * dx + dy * dy).sqrt().max(1e-9);
    (last, (dx / len, dy / len))
}

fn dist(a: (f64, f64), b: (f64, f64)) -> f64 {
    ((b.0 - a.0).powi(2) + (b.1 - a.1).powi(2)).sqrt()
}
