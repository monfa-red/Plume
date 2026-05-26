//! Wire routing — obstacle-aware orthogonal A* (SPEC §9).
//!
//! For each wire segment we:
//!   1. Pick entry / exit edges by relative geometry (nearest edge).
//!   2. Build an obstacle map. Each top-level scene node is an obstacle UNLESS
//!      it contains the source or target endpoint (in which case we recurse
//!      into its children — siblings of the endpoint inside that container
//!      become obstacles). This lets routes enter the group that holds their
//!      endpoint while still avoiding cousin shapes.
//!   3. Run A* on a coarse grid (cell size ≈ wire-gap). The cost penalises
//!      bends so paths stay straight when they can.
//!   4. Fall back through a hierarchy: shapes-and-wires-respected → ignore
//!      other wires → ignore shapes too → straight line.
//!   5. Parallel wires between the same pair are bundled apart in a final
//!      pass.

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

    let mut routed: Vec<RoutedWire> = Vec::new();
    let mut soft_blocked: Vec<Vec<(usize, usize)>> = Vec::new();

    for wire in &program.wires {
        let gap = wire_gap(wire, &program.vars);
        // The grid expands with each wire only if needed; rebuild per-wire since
        // gap can differ. Cheap relative to A* itself.
        let bounds = scene.bounds(gap);
        let grid = Grid::new(bounds, (gap.max(8.0) / 2.0).max(4.0));

        for sub in route_one_wire(wire, &scene, &grid, gap, &soft_blocked)? {
            soft_blocked.push(grid.cells_along(&sub.path));
            routed.push(sub);
        }
    }

    bundle_parallel(&mut routed);
    Ok(routed)
}

// ─────────────────────────── Per-wire orchestration ───────────────────────────

fn route_one_wire(
    wire: &ResolvedWire,
    scene: &SceneIndex,
    grid: &Grid,
    gap: f64,
    soft_blocked: &[Vec<(usize, usize)>],
) -> Result<Vec<RoutedWire>, Error> {
    let n = wire.endpoints.len();
    let from_id = wire.endpoints.first().unwrap().id.clone();
    let to_id = wire.endpoints.last().unwrap().id.clone();
    let mut subs = Vec::with_capacity(n - 1);

    for i in 0..(n - 1) {
        let src = scene
            .lookup(&wire.endpoints[i].id)
            .ok_or_else(|| undefined_wire_id(&wire.endpoints[i].id, wire.endpoints[i].span))?;
        let tgt = scene.lookup(&wire.endpoints[i + 1].id).ok_or_else(|| {
            undefined_wire_id(&wire.endpoints[i + 1].id, wire.endpoints[i + 1].span)
        })?;

        if wire.endpoints[i].id == wire.endpoints[i + 1].id {
            return Err(Error::at(
                wire.span,
                "self-loops are not yet routed (SPEC §9 self-loop is deferred)",
            ));
        }

        let path = route_segment(&src, &tgt, scene, grid, gap, soft_blocked);

        let is_first = i == 0;
        let is_last = i == n - 2;
        subs.push(RoutedWire {
            path: path.clone(),
            markers: Markers {
                start: if is_first {
                    wire.markers.start
                } else {
                    MarkerKind::None
                },
                end: if is_last {
                    wire.markers.end
                } else {
                    MarkerKind::None
                },
            },
            attrs: wire.attrs.clone(),
            texts: if is_first {
                place_texts(&wire.texts, &path)
            } else {
                Vec::new()
            },
            data_from: from_id.clone(),
            data_to: to_id.clone(),
        });
    }
    Ok(subs)
}

fn route_segment(
    src: &ShapeRef,
    tgt: &ShapeRef,
    scene: &SceneIndex,
    grid: &Grid,
    gap: f64,
    soft_blocked: &[Vec<(usize, usize)>],
) -> Vec<(f64, f64)> {
    let src_edge = nearest_edge(&src.bbox, (tgt.bbox.cx(), tgt.bbox.cy()));
    let tgt_edge = nearest_edge(&tgt.bbox, (src.bbox.cx(), src.bbox.cy()));
    let src_pt = edge_midpoint(&src.bbox, src_edge);
    let tgt_pt = edge_midpoint(&tgt.bbox, tgt_edge);

    let start_cell = grid.cell_outside(&src.bbox, src_edge, gap);
    let goal_cell = grid.cell_outside(&tgt.bbox, tgt_edge, gap);

    let shape_obstacles = scene.obstacles_for(&src.id, &tgt.id, gap);
    let blocked_by_shapes = grid.block_cells(&shape_obstacles);
    let blocked_by_wires = grid.flatten_soft(soft_blocked);

    // Tier 1: avoid shapes AND wires.
    if let Some(cells) = a_star(
        grid,
        start_cell,
        goal_cell,
        src_edge,
        tgt_edge,
        &blocked_by_shapes,
        &blocked_by_wires,
    ) {
        return assemble_path(src_pt, &cells, tgt_pt, grid);
    }
    // Tier 2: avoid shapes only.
    if let Some(cells) = a_star(
        grid,
        start_cell,
        goal_cell,
        src_edge,
        tgt_edge,
        &blocked_by_shapes,
        &[],
    ) {
        return assemble_path(src_pt, &cells, tgt_pt, grid);
    }
    // Tier 3: no obstacles (allows crossing shapes).
    if let Some(cells) = a_star(grid, start_cell, goal_cell, src_edge, tgt_edge, &[], &[]) {
        return assemble_path(src_pt, &cells, tgt_pt, grid);
    }
    // Tier 4: straight line.
    vec![src_pt, tgt_pt]
}

fn assemble_path(
    src_pt: (f64, f64),
    cells: &[(usize, usize)],
    tgt_pt: (f64, f64),
    grid: &Grid,
) -> Vec<(f64, f64)> {
    // Convert grid cell centres to world coords; collapse collinear runs;
    // anchor first and last points to the actual shape edges.
    let mut pts: Vec<(f64, f64)> = Vec::with_capacity(cells.len() + 2);
    pts.push(src_pt);
    for &c in cells {
        pts.push(grid.cell_center(c));
    }
    pts.push(tgt_pt);
    simplify(&pts)
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
    // Force the first segment to leave the source orthogonally — snap the
    // second point's coordinates to align with the source's exit axis.
    if out.len() >= 2 {
        let (sx, sy) = out[0];
        let (mut nx, mut ny) = out[1];
        if (nx - sx).abs() > (ny - sy).abs() {
            ny = sy;
        } else {
            nx = sx;
        }
        out[1] = (nx, ny);
    }
    let last_i = out.len() - 1;
    if out.len() >= 2 {
        let (tx, ty) = out[last_i];
        let (mut px, mut py) = out[last_i - 1];
        if (tx - px).abs() > (ty - py).abs() {
            py = ty;
        } else {
            px = tx;
        }
        out[last_i - 1] = (px, py);
    }
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
    id: String,
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
            id: id.to_string(),
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
    /// padded by `gap` so we sit clear of any obstacle inflation.
    fn cell_outside(&self, bbox: &AbsBbox, edge: Edge, gap: f64) -> (usize, usize) {
        let pad = gap + self.cell_size * 0.5;
        let p = match edge {
            Edge::Right => (bbox.x + bbox.w + pad, bbox.cy()),
            Edge::Left => (bbox.x - pad, bbox.cy()),
            Edge::Top => (bbox.cx(), bbox.y - pad),
            Edge::Bottom => (bbox.cx(), bbox.y + bbox.h + pad),
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
) -> Option<Vec<(usize, usize)>> {
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
            return Some(path);
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

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
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

// ─────────────────────────── Edge bundling ───────────────────────────

/// When two or more routed wires connect the same pair of nodes in the same
/// direction, fan their endpoints apart along the shared edge so the SVG
/// paths don't sit on top of each other.
fn bundle_parallel(routed: &mut [RoutedWire]) {
    if routed.len() < 2 {
        return;
    }
    let mut groups: HashMap<(String, String), Vec<usize>> = HashMap::new();
    for (i, w) in routed.iter().enumerate() {
        let key = canonical_pair(&w.data_from, &w.data_to);
        groups.entry(key).or_default().push(i);
    }
    for (_, members) in groups.iter() {
        if members.len() < 2 {
            continue;
        }
        // Spacing tuned for the default --wire-gap of 16. Bundled offsets
        // step by gap/2 so the visible separation matches the routing grid.
        let spacing = 8.0;
        let n = members.len() as f64;
        for (idx, &m) in members.iter().enumerate() {
            let offset = (idx as f64 - (n - 1.0) / 2.0) * spacing;
            shift_endpoints(&mut routed[m], offset);
        }
    }
}

fn canonical_pair(a: &str, b: &str) -> (String, String) {
    if a <= b {
        (a.to_string(), b.to_string())
    } else {
        (b.to_string(), a.to_string())
    }
}

fn shift_endpoints(w: &mut RoutedWire, offset: f64) {
    if w.path.len() < 2 || offset.abs() < 0.01 {
        return;
    }
    let n = w.path.len();
    let first = w.path[0];
    let second = w.path[1];
    let last = w.path[n - 1];
    let prev = w.path[n - 2];

    // Shift the endpoint perpendicular to the first/last segment's direction.
    let (dx1, dy1) = (second.0 - first.0, second.1 - first.1);
    let (dx2, dy2) = (last.0 - prev.0, last.1 - prev.1);
    if dx1.abs() > dy1.abs() {
        w.path[0].1 += offset;
        w.path[1].1 += offset;
    } else {
        w.path[0].0 += offset;
        w.path[1].0 += offset;
    }
    if dx2.abs() > dy2.abs() {
        w.path[n - 1].1 += offset;
        w.path[n - 2].1 += offset;
    } else {
        w.path[n - 1].0 += offset;
        w.path[n - 2].0 += offset;
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
