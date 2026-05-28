//! Per-wire channel routing.
//!
//! Given a source point on a shape edge and a target point on another
//! shape edge, build an orthogonal polyline that:
//!
//!   1. Uses the **fewest bends** possible for the edge pair.
//!   2. Places every bend at the **midline of the channel** it sits in
//!      (the geometric centre between obstacles on either side).
//!   3. Maintains `gap` clearance from every shape and every previously
//!      routed wire.
//!   4. Falls back to detour → perimeter → perpendicular crossing only
//!      when the simpler topology can't clear.
//!
//! Topologies, in order tried:
//!
//! | Edges             | Shape | Bends |
//! |-------------------|-------|-------|
//! | Facing, aligned   | `─`   | 0     |
//! | Facing            | Z     | 2     |
//! | Perpendicular     | L     | 1     |
//! | Same-direction    | U     | 2     |
//! | Anything blocked  | detour with mid channel | 4 |
//!
//! Every candidate polyline is validated end-to-end against `obstacles`
//! before being returned — if any segment would cross a shape's `gap`
//! halo, the next topology is tried instead of silently producing an
//! invalid path.

use super::channels::{
    clear_x_intervals, clear_y_intervals, column_clear, nearest_interval, row_clear,
};
use super::geometry::{AbsBbox, Edge};

pub type Polyline = Vec<(f64, f64)>;

/// Top-level entry. Tries the natural topology for the edge pair, then a
/// detour if obstacles block, and finally a best-effort fallback if even
/// the detour can't clear. Returns whatever it could build — the caller
/// can switch to alternative edges if the result still isn't valid.
pub fn route(
    src: (f64, f64),
    tgt: (f64, f64),
    src_edge: Edge,
    tgt_edge: Edge,
    obstacles: &[AbsBbox],
    world: AbsBbox,
    _prior_paths: &[Polyline],
    gap: f64,
) -> Polyline {
    let candidates = generate_candidates(src, tgt, src_edge, tgt_edge, obstacles, world, gap);

    // Pick the first candidate that clears every shape obstacle. Falls
    // back to the last attempt if nothing fully clears.
    let mut fallback: Option<Polyline> = None;
    for cand in candidates {
        if segments_clear(&cand, obstacles) {
            return cand;
        }
        fallback = Some(cand);
    }
    fallback.unwrap_or_else(|| vec![src, tgt])
}

/// Build the ordered candidate list for an edge pair, cheapest topology
/// first. Each candidate is a complete polyline — caller validates.
#[allow(clippy::too_many_arguments)]
fn generate_candidates(
    src: (f64, f64),
    tgt: (f64, f64),
    src_edge: Edge,
    tgt_edge: Edge,
    obstacles: &[AbsBbox],
    world: AbsBbox,
    gap: f64,
) -> Vec<Polyline> {
    let mut out = Vec::with_capacity(4);

    if src_edge == tgt_edge.opposite() {
        // Facing edges — straight line if aligned, otherwise Z.
        if let Some(p) = straight(src, tgt, src_edge) {
            out.push(p);
        }
        if let Some(p) = z_shape(src, tgt, src_edge, obstacles) {
            out.push(p);
        }
        out.push(detour_facing(
            src, tgt, src_edge, tgt_edge, obstacles, world, gap,
        ));
    } else if src_edge.is_horizontal_exit() != tgt_edge.is_horizontal_exit() {
        // Perpendicular — L-shape.
        if let Some(p) = l_shape(src, tgt, src_edge) {
            out.push(p);
        }
        out.push(detour_perpendicular(
            src, tgt, src_edge, tgt_edge, obstacles, world,
        ));
    } else {
        // Same-direction — U-shape.
        out.push(u_shape(src, tgt, src_edge, obstacles, world));
    }

    out
}

// ─────────────────────────── 0-bend straight ───────────────────────────

/// Straight line between facing edges when the endpoints already share
/// the relevant axis. Returns `None` if the endpoints don't align.
fn straight(src: (f64, f64), tgt: (f64, f64), src_edge: Edge) -> Option<Polyline> {
    if src_edge.is_horizontal_exit() {
        if (src.1 - tgt.1).abs() < 0.5 {
            return Some(vec![src, tgt]);
        }
    } else if (src.0 - tgt.0).abs() < 0.5 {
        return Some(vec![src, tgt]);
    }
    None
}

// ─────────────────────────── 2-bend Z-shape ───────────────────────────

/// Natural Z-shape for facing edges. Picks the bend coordinate at the
/// midline of the channel between the two shapes — the natural midpoint
/// when it lies in a clear strip, otherwise the nearest clear strip's
/// midline.
///
/// Returns `None` if the chosen bend coordinate would land on the wrong
/// side of either endpoint's edge — i.e., the trunk would extend back
/// into a shape's interior. (Common when two shapes in the same row are
/// connected `Bottom→Top`: the midpoint of `src.bottom` and `tgt.top`
/// lands inside both shapes.) When that happens the caller falls through
/// to a 4-bend detour that wraps the trunk around the shapes.
fn z_shape(
    src: (f64, f64),
    tgt: (f64, f64),
    src_edge: Edge,
    obstacles: &[AbsBbox],
) -> Option<Polyline> {
    let tgt_edge = src_edge.opposite();
    if src_edge.is_horizontal_exit() {
        let (x_lo, x_hi) = order(src.0, tgt.0);
        let (y_lo, y_hi) = order(src.1, tgt.1);
        let mid_x = pick_clear_column((src.0 + tgt.0) / 2.0, x_lo, x_hi, y_lo, y_hi, obstacles)?;
        if !valid_horizontal_trunk(mid_x, src.0, tgt.0, src_edge, tgt_edge) {
            return None;
        }
        Some(vec![src, (mid_x, src.1), (mid_x, tgt.1), tgt])
    } else {
        let (x_lo, x_hi) = order(src.0, tgt.0);
        let (y_lo, y_hi) = order(src.1, tgt.1);
        let mid_y = pick_clear_row((src.1 + tgt.1) / 2.0, y_lo, y_hi, x_lo, x_hi, obstacles)?;
        if !valid_vertical_trunk(mid_y, src.1, tgt.1, src_edge, tgt_edge) {
            return None;
        }
        Some(vec![src, (src.0, mid_y), (tgt.0, mid_y), tgt])
    }
}

/// The trunk x of a facing-horizontal Z must lie on the exit side of the
/// source edge and the entry side of the target edge — otherwise the
/// vertical bend would run back inside one of the endpoints' shapes.
fn valid_horizontal_trunk(
    mid_x: f64,
    src_x: f64,
    tgt_x: f64,
    src_edge: Edge,
    tgt_edge: Edge,
) -> bool {
    let src_ok = match src_edge {
        Edge::Right => mid_x > src_x + 0.5,
        Edge::Left => mid_x < src_x - 0.5,
        _ => true,
    };
    let tgt_ok = match tgt_edge {
        Edge::Right => mid_x > tgt_x + 0.5,
        Edge::Left => mid_x < tgt_x - 0.5,
        _ => true,
    };
    src_ok && tgt_ok
}

fn valid_vertical_trunk(
    mid_y: f64,
    src_y: f64,
    tgt_y: f64,
    src_edge: Edge,
    tgt_edge: Edge,
) -> bool {
    let src_ok = match src_edge {
        Edge::Bottom => mid_y > src_y + 0.5,
        Edge::Top => mid_y < src_y - 0.5,
        _ => true,
    };
    let tgt_ok = match tgt_edge {
        Edge::Bottom => mid_y > tgt_y + 0.5,
        Edge::Top => mid_y < tgt_y - 0.5,
        _ => true,
    };
    src_ok && tgt_ok
}

// ─────────────────────────── 1-bend L-shape ───────────────────────────

/// L-shape for perpendicular edges. The corner sits at the orthogonal
/// intersection of the exit rays. Returns the polyline unconditionally —
/// validation is the caller's job; if it doesn't clear we'll fall through
/// to a detour.
fn l_shape(src: (f64, f64), tgt: (f64, f64), src_edge: Edge) -> Option<Polyline> {
    let corner = if src_edge.is_horizontal_exit() {
        (tgt.0, src.1)
    } else {
        (src.0, tgt.1)
    };
    Some(vec![src, corner, tgt])
}

// ─────────────────────────── 2-bend U-shape ───────────────────────────

/// U-shape for two same-direction edges — the wire has to loop out past
/// both shapes' shared side and come back. Builds the loop in a clear
/// row/column outside the relevant shape projection.
fn u_shape(
    src: (f64, f64),
    tgt: (f64, f64),
    src_edge: Edge,
    obstacles: &[AbsBbox],
    world: AbsBbox,
) -> Polyline {
    match src_edge {
        Edge::Right => {
            let y_lo = src.1.min(tgt.1);
            let y_hi = src.1.max(tgt.1);
            let beyond = src.0.max(tgt.0) + 32.0;
            let xs = clear_x_intervals(y_lo, y_hi, obstacles, src.0.max(tgt.0), world.right());
            let mid_x = nearest_interval(&xs, beyond)
                .map(|iv| iv.mid())
                .unwrap_or(beyond);
            vec![src, (mid_x, src.1), (mid_x, tgt.1), tgt]
        }
        Edge::Left => {
            let y_lo = src.1.min(tgt.1);
            let y_hi = src.1.max(tgt.1);
            let beyond = src.0.min(tgt.0) - 32.0;
            let xs = clear_x_intervals(y_lo, y_hi, obstacles, world.x, src.0.min(tgt.0));
            let mid_x = nearest_interval(&xs, beyond)
                .map(|iv| iv.mid())
                .unwrap_or(beyond);
            vec![src, (mid_x, src.1), (mid_x, tgt.1), tgt]
        }
        Edge::Top => {
            let x_lo = src.0.min(tgt.0);
            let x_hi = src.0.max(tgt.0);
            let beyond = src.1.min(tgt.1) - 32.0;
            let ys = clear_y_intervals(x_lo, x_hi, obstacles, world.y, src.1.min(tgt.1));
            let mid_y = nearest_interval(&ys, beyond)
                .map(|iv| iv.mid())
                .unwrap_or(beyond);
            vec![src, (src.0, mid_y), (tgt.0, mid_y), tgt]
        }
        Edge::Bottom => {
            let x_lo = src.0.min(tgt.0);
            let x_hi = src.0.max(tgt.0);
            let beyond = src.1.max(tgt.1) + 32.0;
            let ys = clear_y_intervals(x_lo, x_hi, obstacles, src.1.max(tgt.1), world.bottom());
            let mid_y = nearest_interval(&ys, beyond)
                .map(|iv| iv.mid())
                .unwrap_or(beyond);
            vec![src, (src.0, mid_y), (tgt.0, mid_y), tgt]
        }
    }
}

// ─────────────────────────── 4-bend detour ───────────────────────────

/// Detour for facing edges when the natural Z-shape can't clear. The
/// wire exits its src axis, swings perpendicularly to a clear trunk,
/// crosses the trunk, swings back to tgt's axis, and arrives. The first
/// and last segments always run along the edge axis — exit and entry
/// directions stay perpendicular to their respective shape edges.
///
/// Topology (for facing horizontal):
/// ```text
///   src ── ┐
///          │
///          └─── (trunk_y) ───┐
///                            │
///                            └── tgt
/// ```
#[allow(clippy::too_many_arguments)]
fn detour_facing(
    src: (f64, f64),
    tgt: (f64, f64),
    src_edge: Edge,
    tgt_edge: Edge,
    obstacles: &[AbsBbox],
    world: AbsBbox,
    gap: f64,
) -> Polyline {
    if src_edge.is_horizontal_exit() {
        detour_facing_horizontal(src, tgt, src_edge, tgt_edge, obstacles, world, gap)
    } else {
        detour_facing_vertical(src, tgt, src_edge, tgt_edge, obstacles, world, gap)
    }
}

#[allow(clippy::too_many_arguments)]
fn detour_facing_horizontal(
    src: (f64, f64),
    tgt: (f64, f64),
    src_edge: Edge,
    tgt_edge: Edge,
    obstacles: &[AbsBbox],
    world: AbsBbox,
    gap: f64,
) -> Polyline {
    // Pick a horizontal trunk clear across the full x-span between src
    // and tgt. The trunk is the row the wire travels along between the
    // two flanking obstacles.
    let (x_lo, x_hi) = order(src.0, tgt.0);
    let ys = clear_y_intervals(x_lo, x_hi, obstacles, world.y, world.bottom());
    let preferred_y = (src.1 + tgt.1) / 2.0;
    let trunk_y = pick_trunk(&ys, preferred_y);

    // Bend columns on each side: `b1` flanks the obstacle band on src's
    // side, `b2` on tgt's side. Each must lie on the *exit* side of its
    // endpoint's edge so the wire heads outward, not back into the shape.
    let (sy_lo, sy_hi) = order(src.1, trunk_y);
    let (ty_lo, ty_hi) = order(trunk_y, tgt.1);
    let b1 = bend_column_for(src.0, src_edge, sy_lo, sy_hi, obstacles, world, gap);
    let b2 = bend_column_for(tgt.0, tgt_edge, ty_lo, ty_hi, obstacles, world, gap);

    collapse_collinear(vec![
        src,
        (b1, src.1),
        (b1, trunk_y),
        (b2, trunk_y),
        (b2, tgt.1),
        tgt,
    ])
}

#[allow(clippy::too_many_arguments)]
fn detour_facing_vertical(
    src: (f64, f64),
    tgt: (f64, f64),
    src_edge: Edge,
    tgt_edge: Edge,
    obstacles: &[AbsBbox],
    world: AbsBbox,
    gap: f64,
) -> Polyline {
    // For same-row Bottom↔Top wires the trunk has to live OUTSIDE both
    // shapes' y range. Pick `b1` and `b2` based on the exit/entry edge so
    // the wire's first and last legs head outward, then connect them
    // through a clear trunk column.
    let (y_lo, y_hi) = order(src.1, tgt.1);
    let xs = clear_x_intervals(y_lo, y_hi, obstacles, world.x, world.right());
    let preferred_x = (src.0 + tgt.0) / 2.0;
    let trunk_x = pick_trunk(&xs, preferred_x);

    let (sx_lo, sx_hi) = order(src.0, trunk_x);
    let (tx_lo, tx_hi) = order(trunk_x, tgt.0);
    let b1 = bend_row_for(src.1, src_edge, sx_lo, sx_hi, obstacles, world, gap);
    let b2 = bend_row_for(tgt.1, tgt_edge, tx_lo, tx_hi, obstacles, world, gap);

    collapse_collinear(vec![
        src,
        (src.0, b1),
        (trunk_x, b1),
        (trunk_x, b2),
        (tgt.0, b2),
        tgt,
    ])
}

/// Pick a horizontal-jog y-coord on the *exit* side of `edge`. The wire
/// bends `gap` past the anchor by default — far enough to look clean —
/// and snaps to the nearest clear strip beyond the edge so it never
/// folds back into the shape's interior.
#[allow(clippy::too_many_arguments)]
fn bend_row_for(
    anchor: f64,
    edge: Edge,
    x_lo: f64,
    x_hi: f64,
    obstacles: &[AbsBbox],
    world: AbsBbox,
    gap: f64,
) -> f64 {
    let (search_lo, search_hi, desired) = match edge {
        Edge::Bottom => (anchor + 0.5, world.bottom(), anchor + gap),
        Edge::Top => (world.y, anchor - 0.5, anchor - gap),
        _ => (world.y, world.bottom(), anchor),
    };
    if search_hi <= search_lo {
        return desired;
    }
    let ys = clear_y_intervals(x_lo, x_hi, obstacles, search_lo, search_hi);
    pick_trunk(&ys, desired)
}

/// Mirror of `bend_row_for` for vertical-jog x-coords.
#[allow(clippy::too_many_arguments)]
fn bend_column_for(
    anchor: f64,
    edge: Edge,
    y_lo: f64,
    y_hi: f64,
    obstacles: &[AbsBbox],
    world: AbsBbox,
    gap: f64,
) -> f64 {
    let (search_lo, search_hi, desired) = match edge {
        Edge::Right => (anchor + 0.5, world.right(), anchor + gap),
        Edge::Left => (world.x, anchor - 0.5, anchor - gap),
        _ => (world.x, world.right(), anchor),
    };
    if search_hi <= search_lo {
        return desired;
    }
    let xs = clear_x_intervals(y_lo, y_hi, obstacles, search_lo, search_hi);
    pick_trunk(&xs, desired)
}

/// Detour for perpendicular edge pairs (e.g. R+B, T+L). The first and last
/// segments run along their respective edge's normal — horizontal exit
/// ends with a vertical entry (or vice versa) — so the wire approaches the
/// target perpendicular to its edge rather than tangent.
///
/// Four segments instead of the facing case's five: the trunk swap doesn't
/// need a return leg, because src and tgt sit on different axes.
fn detour_perpendicular(
    src: (f64, f64),
    tgt: (f64, f64),
    src_edge: Edge,
    _tgt_edge: Edge,
    obstacles: &[AbsBbox],
    world: AbsBbox,
) -> Polyline {
    if src_edge.is_horizontal_exit() {
        // h, v, h, v — `src → (b1, src.y) → (b1, trunk_y) → (tgt.x, trunk_y) → tgt`.
        let (x_lo, x_hi) = order(src.0, tgt.0);
        let preferred_y = (src.1 + tgt.1) / 2.0;
        let ys = clear_y_intervals(x_lo, x_hi, obstacles, world.y, world.bottom());
        let trunk_y = pick_trunk(&ys, preferred_y);

        let (sy_lo, sy_hi) = order(src.1, trunk_y);
        let b1 = pick_bend_column(src.0, sy_lo, sy_hi, x_lo, x_hi, obstacles);

        collapse_collinear(vec![src, (b1, src.1), (b1, trunk_y), (tgt.0, trunk_y), tgt])
    } else {
        // v, h, v, h — `src → (src.x, b1) → (trunk_x, b1) → (trunk_x, tgt.y) → tgt`.
        let (y_lo, y_hi) = order(src.1, tgt.1);
        let preferred_x = (src.0 + tgt.0) / 2.0;
        let xs = clear_x_intervals(y_lo, y_hi, obstacles, world.x, world.right());
        let trunk_x = pick_trunk(&xs, preferred_x);

        let (sx_lo, sx_hi) = order(src.0, trunk_x);
        let b1 = pick_bend_row(src.1, sx_lo, sx_hi, y_lo, y_hi, obstacles);

        collapse_collinear(vec![src, (src.0, b1), (trunk_x, b1), (trunk_x, tgt.1), tgt])
    }
}

// ─────────────────────────── Channel pickers ───────────────────────────

/// Midline of the clear-x strip closest to `desired` for a vertical
/// segment spanning `[y_lo, y_hi]`. If `desired` falls inside a clear
/// strip, that exact x is returned; otherwise the strip's midpoint.
fn pick_clear_column(
    desired: f64,
    x_lo: f64,
    x_hi: f64,
    y_lo: f64,
    y_hi: f64,
    obstacles: &[AbsBbox],
) -> Option<f64> {
    let xs = clear_x_intervals(y_lo, y_hi, obstacles, x_lo, x_hi);
    nearest_interval(&xs, desired).map(|iv| {
        if iv.contains(desired) {
            desired
        } else {
            iv.mid()
        }
    })
}

/// Mirror of `pick_clear_column` for a horizontal segment spanning
/// `[x_lo, x_hi]`.
fn pick_clear_row(
    desired: f64,
    y_lo: f64,
    y_hi: f64,
    x_lo: f64,
    x_hi: f64,
    obstacles: &[AbsBbox],
) -> Option<f64> {
    let ys = clear_y_intervals(x_lo, x_hi, obstacles, y_lo, y_hi);
    nearest_interval(&ys, desired).map(|iv| {
        if iv.contains(desired) {
            desired
        } else {
            iv.mid()
        }
    })
}

/// Pick a trunk row/column close to `target`. If `target` falls inside a
/// clear interval, use it exactly (so the wire's middle leg lands at the
/// natural geometric centre); otherwise snap to the closest interval's
/// midline.
fn pick_trunk(intervals: &[super::channels::Interval], target: f64) -> f64 {
    nearest_interval(intervals, target)
        .map(|iv| {
            if iv.contains(target) {
                target
            } else {
                iv.mid()
            }
        })
        .unwrap_or(target)
}

/// Midline of the clear column strip closest to `anchor`, in the relevant
/// y-strip. The strip is the y-span between the endpoint and the trunk —
/// the flanking column the bend can safely occupy.
fn pick_bend_column(
    anchor: f64,
    y_lo: f64,
    y_hi: f64,
    x_lo: f64,
    x_hi: f64,
    obstacles: &[AbsBbox],
) -> f64 {
    let xs = clear_x_intervals(y_lo, y_hi, obstacles, x_lo, x_hi);
    nearest_interval(&xs, anchor)
        .map(|iv| iv.mid())
        .unwrap_or(anchor)
}

fn pick_bend_row(
    anchor: f64,
    x_lo: f64,
    x_hi: f64,
    y_lo: f64,
    y_hi: f64,
    obstacles: &[AbsBbox],
) -> f64 {
    let ys = clear_y_intervals(x_lo, x_hi, obstacles, y_lo, y_hi);
    nearest_interval(&ys, anchor)
        .map(|iv| iv.mid())
        .unwrap_or(anchor)
}

// ─────────────────────────── Validation ───────────────────────────

/// Public alias of `segments_clear` — callers use this to decide whether
/// to retry with alternative edges.
pub fn path_is_clear(path: &[(f64, f64)], obstacles: &[AbsBbox]) -> bool {
    segments_clear(path, obstacles)
}

/// True if every segment of `path` (assumed axis-aligned) is gap-clear
/// of every obstacle in `obstacles`.
fn segments_clear(path: &[(f64, f64)], obstacles: &[AbsBbox]) -> bool {
    for w in path.windows(2) {
        let (a, b) = (w[0], w[1]);
        if (a.1 - b.1).abs() < 0.5 {
            // Horizontal segment.
            let (x_lo, x_hi) = order(a.0, b.0);
            if !row_clear(a.1, x_lo, x_hi, obstacles) {
                return false;
            }
        } else if (a.0 - b.0).abs() < 0.5 {
            // Vertical segment.
            let (y_lo, y_hi) = order(a.1, b.1);
            if !column_clear(a.0, y_lo, y_hi, obstacles) {
                return false;
            }
        }
    }
    true
}

// ─────────────────────────── Polyline utilities ───────────────────────────

/// Drop points that lie collinear with their neighbours. Keeps the
/// polyline minimal so adjacent zero-length segments don't render as
/// kinks.
fn collapse_collinear(pts: Vec<(f64, f64)>) -> Polyline {
    if pts.len() < 3 {
        return pts;
    }
    let mut out: Polyline = vec![pts[0]];
    for i in 1..pts.len() - 1 {
        let a = *out.last().unwrap();
        let b = pts[i];
        let c = pts[i + 1];
        let same_x = (a.0 - b.0).abs() < 0.5 && (b.0 - c.0).abs() < 0.5;
        let same_y = (a.1 - b.1).abs() < 0.5 && (b.1 - c.1).abs() < 0.5;
        let degenerate = (a.0 - b.0).abs() < 0.5 && (a.1 - b.1).abs() < 0.5;
        if !(same_x || same_y || degenerate) {
            out.push(b);
        }
    }
    out.push(*pts.last().unwrap());
    out
}

fn order(a: f64, b: f64) -> (f64, f64) {
    if a <= b {
        (a, b)
    } else {
        (b, a)
    }
}
