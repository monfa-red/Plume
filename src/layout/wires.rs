//! Wire routing — obstacle-aware orthogonal A* (SPEC §9).
//!
//! For each wire segment we:
//!   1. Pick entry / exit edges by relative geometry (nearest edge).
//!   2. Apply a lane offset along the chosen edges. Parallel wires (same
//!      pair of endpoints) get distinct offsets BEFORE A* runs, so each
//!      wire computes its own path on its own track instead of stacking
//!      onto the leader and then being shifted post-hoc (which produced
//!      crossing paths when the leader and the follower took different
//!      A* routes).
//!   3. Build an obstacle map. Each scene node is a hard obstacle UNLESS it
//!      is an endpoint or a named ancestor of an endpoint — in those cases
//!      we recurse into its children so the path can enter the group that
//!      holds its endpoint while still avoiding cousin shapes.
//!   4. Run A* on a coarse grid (cell size ≈ wire-gap). The cost penalises
//!      bends so paths stay straight when they can.
//!   5. Fall back through a hierarchy: shapes-and-wires-respected → ignore
//!      other wires → ignore shapes too → straight line.

use super::ir::{PlacedNode, RoutedText, RoutedWire};
use super::values::layout_var;
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

    // Pre-pass: explode chains into per-segment specs, pre-pick the entry/
    // exit edge for each end via `nearest_edge`, and count how many segments
    // share each (shape, edge) so we can fan wires sharing an exit/entry
    // along that edge. Lanes are computed per endpoint independently — a
    // wire's source lane comes from the bin (src_id, src_edge); its target
    // lane from (tgt_id, tgt_edge).
    let specs = plan_segments(program, &scene)?;

    let mut bin_total: HashMap<(String, Edge), usize> = HashMap::new();
    for s in &specs {
        *bin_total.entry((s.src_id.clone(), s.src_edge)).or_insert(0) += 1;
        *bin_total.entry((s.tgt_id.clone(), s.tgt_edge)).or_insert(0) += 1;
    }
    let mut bin_seen: HashMap<(String, Edge), usize> = HashMap::new();
    let mut lane_offsets: Vec<(f64, f64)> = Vec::with_capacity(specs.len());
    for s in &specs {
        let src = next_lane(&mut bin_seen, &bin_total, &s.src_id, s.src_edge, s.gap);
        let tgt = next_lane(&mut bin_seen, &bin_total, &s.tgt_id, s.tgt_edge, s.gap);
        lane_offsets.push((src, tgt));
    }

    let mut routed: Vec<RoutedWire> = Vec::with_capacity(specs.len());
    let mut soft_blocked: Vec<Vec<(usize, usize)>> = Vec::new();

    for (i, spec) in specs.iter().enumerate() {
        let bounds = scene.bounds(spec.gap);
        let grid = Grid::new(bounds, (spec.gap.max(8.0) / 2.0).max(4.0));

        let (src_lane, tgt_lane) = lane_offsets[i];
        let path = route_segment_with_lanes(spec, &scene, &grid, src_lane, tgt_lane, &soft_blocked);
        soft_blocked.push(grid.cells_along(&path));
        routed.push(build_routed_wire(spec, path));
    }

    Ok(routed)
}

/// Count the number of segments using each `(shape, edge)` so far, returning
/// the lane offset for THIS segment. Lanes are centred around 0: for a bin
/// with `n` segments at indices `0..n`, offsets are `(i − (n−1)/2) * step`.
fn next_lane(
    seen: &mut HashMap<(String, Edge), usize>,
    total: &HashMap<(String, Edge), usize>,
    id: &str,
    edge: Edge,
    gap: f64,
) -> f64 {
    let key = (id.to_string(), edge);
    let n = total.get(&key).copied().unwrap_or(1);
    if n <= 1 {
        return 0.0;
    }
    let i = seen.entry(key).or_insert(0);
    let idx = *i;
    *i += 1;
    let step = gap / 2.0;
    (idx as f64 - (n as f64 - 1.0) / 2.0) * step
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
        let from_id = wire.endpoints.first().unwrap().id.clone();
        let to_id = wire.endpoints.last().unwrap().id.clone();
        let gap = wire_gap(wire, &program.vars);
        for i in 0..(n - 1) {
            let src_id = wire.endpoints[i].id.clone();
            let tgt_id = wire.endpoints[i + 1].id.clone();
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
            let src_edge = nearest_edge(&src.bbox, (tgt.bbox.cx(), tgt.bbox.cy()));
            let tgt_edge = nearest_edge(&tgt.bbox, (src.bbox.cx(), src.bbox.cy()));
            out.push(SegmentSpec {
                wire,
                src_id,
                tgt_id,
                src_edge,
                tgt_edge,
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
    soft_blocked: &[Vec<(usize, usize)>],
) -> Vec<(f64, f64)> {
    let shape_obstacles = scene.obstacles_for(&spec.src_id, &spec.tgt_id, spec.gap);
    let blocked_by_shapes = grid.block_cells(&shape_obstacles);
    let blocked_by_wires = grid.flatten_soft(soft_blocked);

    // Try every edge combination (4 src × 4 tgt = 16), keeping the cheapest
    // A* result. `nearest_edge` was a fast heuristic, but on tight diagrams
    // it picks an edge that forces a long detour (e.g. exiting RIGHT into an
    // obstacle when going UP and OVER would be much shorter). Exhaustive
    // search at this scale is still microsecond-fast: the grid is coarse
    // and most candidate paths are rejected almost immediately.
    let all_edges = [Edge::Right, Edge::Bottom, Edge::Left, Edge::Top];
    type Candidate = (i64, Edge, Edge, Vec<(usize, usize)>);
    let mut best: Option<Candidate> = None;

    for tier in 0..3 {
        let (use_shapes, use_wires) = match tier {
            0 => (&blocked_by_shapes[..], &blocked_by_wires[..]),
            1 => (&blocked_by_shapes[..], &[][..]),
            _ => (&[][..], &[][..]),
        };
        for &se in &all_edges {
            for &te in &all_edges {
                let start = grid.cell_outside(&spec.src_bbox, se, spec.gap, src_lane);
                let goal = grid.cell_outside(&spec.tgt_bbox, te, spec.gap, tgt_lane);
                if let Some((cells, cost)) =
                    a_star(grid, start, goal, se, te, use_shapes, use_wires)
                {
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
    /// `name → index in `nodes`. Names are unique per SPEC §15.
    by_id: HashMap<String, usize>,
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
            by_id: HashMap::new(),
        };
        for node in scene_nodes {
            idx.walk(node, 0.0, 0.0, &[]);
        }
        idx
    }

    fn walk(&mut self, node: &PlacedNode, parent_cx: f64, parent_cy: f64, ancestors: &[usize]) {
        let abs_cx = parent_cx + node.cx;
        let abs_cy = parent_cy + node.cy;
        let bbox = AbsBbox {
            x: abs_cx + node.bbox.min_x,
            y: abs_cy + node.bbox.min_y,
            w: node.bbox.w(),
            h: node.bbox.h(),
        };
        let mut next_ancestors = ancestors.to_vec();
        if let Some(id) = &node.id {
            let i = self.nodes.len();
            self.nodes.push(IndexedNode {
                bbox,
                ancestors: ancestors.to_vec(),
                is_leaf: node.children.is_empty(),
            });
            let _ = id; // id is kept in `by_id` only
            self.by_id.insert(id.clone(), i);
            next_ancestors.push(i);
        }
        for child in &node.children {
            self.walk(child, abs_cx, abs_cy, &next_ancestors);
        }
    }

    fn lookup(&self, id: &str) -> Option<ShapeRef> {
        let i = *self.by_id.get(id)?;
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
        let src_i = self.by_id.get(src_id).copied();
        let tgt_i = self.by_id.get(tgt_id).copied();
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

    /// Mark every cell whose centre lies inside any obstacle bbox.
    fn block_cells(&self, obstacles: &[AbsBbox]) -> Vec<bool> {
        let mut blocked = vec![false; self.cols * self.rows];
        for ob in obstacles {
            let min_c = ((ob.x - self.bounds.x) / self.cell_size).floor().max(0.0) as usize;
            let min_r = ((ob.y - self.bounds.y) / self.cell_size).floor().max(0.0) as usize;
            let max_c =
                (((ob.x + ob.w - self.bounds.x) / self.cell_size).ceil() as usize).min(self.cols);
            let max_r =
                (((ob.y + ob.h - self.bounds.y) / self.cell_size).ceil() as usize).min(self.rows);
            for r in min_r..max_r {
                for c in min_c..max_c {
                    blocked[r * self.cols + c] = true;
                }
            }
        }
        blocked
    }

    fn cells_along(&self, path: &[(f64, f64)]) -> Vec<(usize, usize)> {
        let mut out = Vec::new();
        for w in path.windows(2) {
            let a = self.world_to_cell(w[0]);
            let b = self.world_to_cell(w[1]);
            line_cells(a, b, &mut out);
        }
        out.sort();
        out.dedup();
        out
    }

    fn flatten_soft(&self, soft: &[Vec<(usize, usize)>]) -> Vec<bool> {
        let mut soft_blocked = vec![false; self.cols * self.rows];
        for cells in soft {
            for &(c, r) in cells {
                if c < self.cols && r < self.rows {
                    soft_blocked[r * self.cols + c] = true;
                }
            }
        }
        soft_blocked
    }
}

/// Rasterise an orthogonal line between two grid cells (paths are already
/// axis-aligned by the time we record them).
fn line_cells(a: (usize, usize), b: (usize, usize), out: &mut Vec<(usize, usize)>) {
    if a.0 == b.0 {
        let (r0, r1) = if a.1 <= b.1 { (a.1, b.1) } else { (b.1, a.1) };
        for r in r0..=r1 {
            out.push((a.0, r));
        }
    } else if a.1 == b.1 {
        let (c0, c1) = if a.0 <= b.0 { (a.0, b.0) } else { (b.0, a.0) };
        for c in c0..=c1 {
            out.push((c, a.1));
        }
    } else {
        out.push(a);
        out.push(b);
    }
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

fn a_star(
    grid: &Grid,
    start: (usize, usize),
    goal: (usize, usize),
    src_edge: Edge,
    tgt_edge: Edge,
    blocked: &[bool],
    soft: &[bool],
) -> Option<(Vec<(usize, usize)>, i64)> {
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

    const BEND: i64 = 4; // discourage turns
    const SOFT: i64 = 4; // mild preference against crossing other wires

    while let Some(node) = open.pop() {
        if node.cell == goal {
            // Reconstruct.
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
            // Add an extra step in `goal_dir` direction beyond the goal so
            // the entry-axis snap in `simplify` lines up with the target edge.
            if let Some(extra) = step(node.cell, goal_dir, grid) {
                if extra != node.cell {
                    path.push(extra);
                }
            }
            return Some((path, node.g_cost));
        }

        for &d in &[Dir::Right, Dir::Left, Dir::Up, Dir::Down] {
            let next = match step(node.cell, d, grid) {
                Some(c) => c,
                None => continue,
            };
            let i = next.1 * grid.cols + next.0;
            if blocked[i] && next != goal && next != start {
                continue;
            }

            let mut step_cost = 1_i64;
            if node.dir != Dir::None && node.dir != d {
                step_cost += BEND;
            }
            if i < soft.len() && soft[i] {
                step_cost += SOFT;
            }

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
