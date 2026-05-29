//! Global, obstacle-aware track assignment (Step 2.5).
//!
//! A\* (Step 2.4) routes each wire against shapes only, so parallel wires
//! between the same pair collapse onto one shared track — they stack
//! (R4 collinear) or sit closer than `separation` (R3). This pass fans them
//! apart: every segment is reassigned to a clear track at least `separation`
//! from any overlapping segment of another declaration.
//!
//! The trick is that *attachment* segments are not pinned — a wire's endpoint
//! slides freely *along its edge* and stays perpendicular (R5), so the first and
//! last segments are movable too, merely **clamped** to the edge's span. That
//! folds endpoint lane-spreading and trunk separation into one decision.
//!
//! Three properties keep it honest:
//!
//! - **Attachment-safe (R5).** Endpoints only ever move along their own edge.
//! - **Obstacle-aware (R2).** Each candidate track is tested clear of shapes
//!   (and so are the neighbour segments it drags) — never a blind offset.
//! - **Global, not greedy.** Within a channel the segments are interval-coloured
//!   (sorted by entry, nearest free track), so a whole bundle fans into parallel
//!   rails rather than each wire detouring around the last.
//!
//! Sliding a segment only changes its constant coordinate; the endpoints it
//! shares with its neighbours move with it, so the polyline stays orthogonal and
//! joins exactly — no bend drift.

use super::astar::collapse;
use super::geometry::AbsBbox;
use super::grid::edge_clear;
use crate::span::Span;

const EPS: f64 = 0.5;
/// V/H refinement passes. Each pass fixes one axis; a later pass picks up the
/// span changes an earlier one introduced. Three converges on these scenes.
const PASSES: usize = 3;
/// Granularity of candidate track offsets when sliding a segment.
const STEP: f64 = 2.0;
/// Farthest an interior segment may slide from where A\* put it.
const REACH: f64 = 400.0;

#[derive(Clone, Copy, PartialEq)]
pub enum Axis {
    H,
    V,
}

/// The edge constraint on one of a wire's endpoints: a horizontal-exit edge
/// (left/right) pins the first/last segment to be horizontal and lets its `y`
/// roam in `[lo, hi]`; a vertical-exit edge does the same for `x`.
#[derive(Clone, Copy)]
pub struct End {
    pub horizontal: bool,
    pub lo: f64,
    pub hi: f64,
}

/// One axis-aligned segment of a wire's polyline, with the range its track may
/// occupy (`[clamp_lo, clamp_hi]`; unbounded for interior segments).
#[derive(Clone, Copy)]
struct Seg {
    wire: usize,
    /// Segment runs `path[k]..path[k+1]`.
    k: usize,
    /// Constant coordinate (y for horizontal, x for vertical).
    pos: f64,
    /// Span minimum along the varying axis.
    lo: f64,
    /// Span maximum along the varying axis.
    hi: f64,
    decl: Span,
    gap: f64,
    clamp_lo: f64,
    clamp_hi: f64,
    /// A straight, single-segment wire is already placed optimally by endpoint
    /// allocation (evenly spread along its edge, even when overflowing). Such
    /// segments are fixed constraints here — re-tracking them would only undo
    /// that even spread and cram one wire. Multi-segment wires stay movable.
    movable: bool,
}

/// Fan every wire's segments onto separated, shape-clear tracks. `decl`/`gaps`/
/// `obstacles`/`ends` are per-wire: declaration span (same span = one
/// declaration, exempt from separation), the wire's gap, the clearance-inflated
/// shapes it must avoid, and its two endpoint edge constraints.
pub fn assign(
    paths: &mut [Vec<(f64, f64)>],
    decl: &[Span],
    gaps: &[f64],
    obstacles: &[Vec<AbsBbox>],
    ends: &[(End, End)],
) {
    for _ in 0..PASSES {
        assign_axis(paths, decl, gaps, obstacles, ends, Axis::V);
        assign_axis(paths, decl, gaps, obstacles, ends, Axis::H);
    }
    for p in paths.iter_mut() {
        *p = collapse(std::mem::take(p));
    }
}

fn assign_axis(
    paths: &mut [Vec<(f64, f64)>],
    decl: &[Span],
    gaps: &[f64],
    obstacles: &[Vec<AbsBbox>],
    ends: &[(End, End)],
    axis: Axis,
) {
    let segs = collect(paths, decl, gaps, ends, axis);

    // Fixed segments (straight single-segment wires) are constraints from the
    // start — movable segments route around them but they never move.
    let mut placed: Vec<Seg> = segs.iter().copied().filter(|s| !s.movable).collect();

    // Assign movable segments in sweep order (by interval start, then current
    // track, then wire) so the colouring is deterministic.
    let mut order: Vec<usize> = (0..segs.len()).filter(|&i| segs[i].movable).collect();
    order.sort_by(|&a, &b| {
        let (x, y) = (&segs[a], &segs[b]);
        x.lo.total_cmp(&y.lo)
            .then(x.pos.total_cmp(&y.pos))
            .then(x.wire.cmp(&y.wire))
    });

    for i in order {
        let s = segs[i];
        let chosen = choose_track(&s, &placed, paths, obstacles, axis);
        slide(&mut paths[s.wire], s.k, axis, chosen);
        placed.push(Seg { pos: chosen, ..s });
    }
}

/// Read every segment of `axis` out of the current polylines, tagging each with
/// the track range it may occupy.
fn collect(
    paths: &[Vec<(f64, f64)>],
    decl: &[Span],
    gaps: &[f64],
    ends: &[(End, End)],
    axis: Axis,
) -> Vec<Seg> {
    let mut segs = Vec::new();
    for (w, path) in paths.iter().enumerate() {
        if path.len() < 2 {
            continue;
        }
        let nseg = path.len() - 1;
        for k in 0..nseg {
            let (a, b) = (path[k], path[k + 1]);
            let seg_axis = if (a.1 - b.1).abs() < EPS {
                Axis::H
            } else if (a.0 - b.0).abs() < EPS {
                Axis::V
            } else {
                continue; // degenerate / diagonal — leave for the validator
            };
            if seg_axis != axis {
                continue;
            }
            let (pos, lo, hi) = match axis {
                Axis::H => (a.1, a.0.min(b.0), a.0.max(b.0)),
                Axis::V => (a.0, a.1.min(b.1), a.1.max(b.1)),
            };
            let (clamp_lo, clamp_hi) = track_range(k, nseg, axis, &ends[w]);
            segs.push(Seg {
                wire: w,
                k,
                pos,
                lo,
                hi,
                decl: decl[w],
                gap: gaps[w],
                clamp_lo,
                clamp_hi,
                movable: nseg > 1,
            });
        }
    }
    segs
}

/// The allowed track range for segment `k`: interior segments roam within
/// `REACH` of where they sit; endpoint segments are clamped to their edge so
/// the attachment stays on the shape (and stays perpendicular). A single-segment
/// wire is clamped to the overlap of both edges.
fn track_range(k: usize, nseg: usize, axis: Axis, ends: &(End, End)) -> (f64, f64) {
    let is_first = k == 0;
    let is_last = k == nseg - 1;
    let edge_range = |e: &End| {
        if (axis == Axis::H) == e.horizontal {
            Some((e.lo, e.hi))
        } else {
            None
        }
    };
    let src = if is_first { edge_range(&ends.0) } else { None };
    let tgt = if is_last { edge_range(&ends.1) } else { None };
    match (src, tgt) {
        (Some(a), Some(b)) => (a.0.max(b.0), a.1.min(b.1)),
        (Some(a), None) | (None, Some(a)) => a,
        (None, None) => (f64::NEG_INFINITY, f64::INFINITY),
    }
}

/// Pick a shape-clear, in-range track for `s`. Prefer the nearest track that
/// keeps full `separation` from every overlapping segment of another
/// declaration; when the channel can't fit one (inherent overflow), fall back
/// to the track with the most slack — spreading the bundle so it degrades to a
/// too-close R3 rather than a collinear R4.
fn choose_track(
    s: &Seg,
    placed: &[Seg],
    paths: &[Vec<(f64, f64)>],
    obstacles: &[Vec<AbsBbox>],
    axis: Axis,
) -> f64 {
    let lo = s.clamp_lo.max(s.pos - REACH);
    let hi = s.clamp_hi.min(s.pos + REACH);
    let steps = (REACH / STEP) as i64;
    let mut best: Option<(f64, f64)> = None; // (margin, pos), most slack wins
    for d in 0..=steps {
        for cand in offsets(s.pos, d) {
            if cand < lo - EPS || cand > hi + EPS {
                continue;
            }
            if !clear(s, cand, &paths[s.wire], &obstacles[s.wire], axis) {
                continue;
            }
            let m = margin(s, cand, placed);
            if m >= -EPS {
                return cand; // fully separated; nearest-first, so we're done
            }
            if best.map_or(true, |(bm, _)| m > bm + EPS) {
                best = Some((m, cand));
            }
        }
    }
    best.map(|(_, p)| p).unwrap_or(s.pos.clamp(lo, hi))
}

/// Candidate positions at ring `d`: the original first, then symmetric pairs.
fn offsets(pos: f64, d: i64) -> [f64; 2] {
    if d == 0 {
        [pos, pos]
    } else {
        let o = d as f64 * STEP;
        [pos - o, pos + o]
    }
}

/// Worst slack over all overlapping placed segments of a *different*
/// declaration: `min(distance − separation)`. `≥ 0` means fully separated;
/// `+∞` when nothing overlaps.
fn margin(s: &Seg, cand: f64, placed: &[Seg]) -> f64 {
    let mut worst = f64::INFINITY;
    for t in placed {
        if t.decl == s.decl {
            continue;
        }
        if s.lo < t.hi - EPS && t.lo < s.hi - EPS {
            let sep = s.gap.max(t.gap);
            worst = worst.min((cand - t.pos).abs() - sep);
        }
    }
    worst
}

/// True if sliding segment `k` to `cand` leaves it and any neighbour segments
/// clear of `obstacles` and non-degenerate.
fn clear(s: &Seg, cand: f64, path: &[(f64, f64)], obstacles: &[AbsBbox], axis: Axis) -> bool {
    let nseg = path.len() - 1;
    let (pk, pk1) = match axis {
        Axis::H => ((path[s.k].0, cand), (path[s.k + 1].0, cand)),
        Axis::V => ((cand, path[s.k].1), (cand, path[s.k + 1].1)),
    };
    let mut affected = vec![(pk, pk1)];
    if s.k > 0 {
        affected.push((path[s.k - 1], pk));
    }
    if s.k + 1 < nseg {
        affected.push((pk1, path[s.k + 2]));
    }
    affected.iter().all(|&(a, b)| {
        let len = (a.0 - b.0).abs() + (a.1 - b.1).abs();
        len > EPS && edge_clear(a, b, obstacles)
    })
}

/// Move segment `k` onto track `pos`, dragging the two shared endpoints.
fn slide(path: &mut [(f64, f64)], k: usize, axis: Axis, pos: f64) {
    match axis {
        Axis::H => {
            path[k].1 = pos;
            path[k + 1].1 = pos;
        }
        Axis::V => {
            path[k].0 = pos;
            path[k + 1].0 = pos;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn span(i: usize) -> Span {
        Span::new(i, i + 1)
    }

    fn free_end() -> End {
        End {
            horizontal: true,
            lo: f64::NEG_INFINITY,
            hi: f64::INFINITY,
        }
    }

    fn ends(n: usize) -> Vec<(End, End)> {
        vec![(free_end(), free_end()); n]
    }

    /// Two identical Z wires sharing a middle vertical at x=10 must fan onto
    /// tracks at least `sep` apart.
    #[test]
    fn separates_shared_vertical() {
        let z = vec![(0.0, 0.0), (10.0, 0.0), (10.0, 40.0), (20.0, 40.0)];
        let mut paths = vec![z.clone(), z];
        assign(
            &mut paths,
            &[span(0), span(1)],
            &[10.0, 10.0],
            &[vec![], vec![]],
            &ends(2),
        );
        for p in &paths {
            assert!(p
                .windows(2)
                .all(|w| (w[0].0 - w[1].0).abs() < EPS || (w[0].1 - w[1].1).abs() < EPS));
        }
        let (x0, x1) = (paths[0][1].0, paths[1][1].0);
        assert!(
            (x0 - x1).abs() >= 10.0 - EPS,
            "verticals not separated: {x0} vs {x1}"
        );
    }

    /// Same declaration (chain/fan-out siblings) is exempt — no forced spread.
    #[test]
    fn same_decl_not_separated() {
        let z = vec![(0.0, 0.0), (10.0, 0.0), (10.0, 40.0), (20.0, 40.0)];
        let mut paths = vec![z.clone(), z];
        assign(
            &mut paths,
            &[span(0), span(0)],
            &[10.0, 10.0],
            &[vec![], vec![]],
            &ends(2),
        );
        assert!((paths[0][1].0 - paths[1][1].0).abs() < EPS);
    }

    /// Straight, single-segment wires are placed by endpoint allocation, not
    /// here — track assignment leaves them untouched, so an evenly-spread
    /// (even overflowing) bundle isn't re-crammed.
    #[test]
    fn straight_wires_are_fixed() {
        let h = vec![(0.0, 10.0), (50.0, 10.0)];
        let mut paths = vec![h.clone(), h.clone()];
        assign(
            &mut paths,
            &[span(0), span(1)],
            &[10.0, 10.0],
            &[vec![], vec![]],
            &ends(2),
        );
        assert_eq!(paths[0], h);
        assert_eq!(paths[1], h);
    }

    /// A multi-segment wire's endpoint segment slides along its edge to
    /// separate, but stays within the edge clamp.
    #[test]
    fn endpoint_slides_within_clamp() {
        // Two L-wires (horizontal then vertical) sharing the first segment at
        // y=10; the source edge clamps y to [0, 30].
        let l = vec![(0.0, 10.0), (30.0, 10.0), (30.0, 40.0)];
        let src = End {
            horizontal: true,
            lo: 0.0,
            hi: 30.0,
        };
        let tgt = End {
            horizontal: false,
            lo: 0.0,
            hi: 100.0,
        };
        let mut paths = vec![l.clone(), l];
        assign(
            &mut paths,
            &[span(0), span(1)],
            &[10.0, 10.0],
            &[vec![], vec![]],
            &[(src, tgt), (src, tgt)],
        );
        for p in &paths {
            assert!((0.0..=30.0).contains(&p[0].1), "endpoint y out of clamp");
        }
        assert!(
            (paths[0][0].1 - paths[1][0].1).abs() >= 10.0 - EPS,
            "first segments not separated"
        );
    }
}
