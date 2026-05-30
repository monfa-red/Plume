//! Nudge — the global track-assignment pass (WIRING appendix step 3, B2/B6).
//!
//! Per-wire A* leaves parallel wires that share a route stacked on top of each
//! other: A3 (shared parallel runs) and B2 (sub-separation). The router can't see
//! this — it routes one wire at a time. Here we look at every wire at once and
//! spread interior segments that lie on the same line and overlap onto distinct
//! tracks `separation` apart.
//!
//! The move is structure-preserving: a wire's ports and the two segments attached
//! to them are pinned (so attachment A2 is untouched), and every *interior* vertex
//! is rebuilt as the meeting point of its two — possibly shifted — segments. Since
//! consecutive segments are perpendicular, that meeting point is well defined and
//! the polyline stays orthogonal and connected by construction.
//!
//! A separation is only committed if it keeps the affected wires **safe** — no
//! node pierced or grazed (B1/B2 wire↔node), attachment preserved (A2), no
//! self-cross (A5). A channel too tight to separate safely is left as it was; its
//! sub-separation is genuine overflow for `nudge` to leave flagged, not a route to
//! force into a node.

use super::geometry::{clean, range_overlap, rect_penetrated_by, seg_rect_distance, Pt, Rect, EPS};
use super::oracle;
use super::scene::{obstacles_for, SceneIndex};
use crate::layout::ir::{PlacedNode, RoutedWire};
use std::collections::BTreeMap;

/// One candidate track placement: where each `(wire, segment)` should sit.
type Placement = Vec<((usize, usize), f64)>;

/// The scene data a wire's nudge-safety check needs that is constant across the
/// side-search — its solid obstacles and its own endpoint rects. Built once per
/// scene (each is a scene-tree walk) and reused for every candidate the search
/// nudges, instead of rebuilt on every call.
pub struct WireScene {
    obstacles: Vec<Rect>, // the solid nodes this wire must clear
    endpoints: Vec<Rect>, // its own endpoint rects (empty for a self-loop)
}

/// Build the per-wire scene data, in wire order. The wires' endpoints don't change
/// during the search, so the caller computes this once and threads it into every
/// [`nudge_with`].
pub fn build_scenes(wires: &[RoutedWire], nodes: &[PlacedNode]) -> Vec<WireScene> {
    let index = SceneIndex::build(nodes);
    wires
        .iter()
        .map(|w| WireScene {
            obstacles: obstacles_for(nodes, [&w.seg_from, &w.seg_to]),
            endpoints: if w.seg_from == w.seg_to {
                Vec::new() // a self-loop hugs its node at clearance — exempt
            } else {
                [&w.seg_from, &w.seg_to]
                    .iter()
                    .filter_map(|id| index.rect(id))
                    .collect()
            },
        })
        .collect()
}

/// Per-wire safety inputs for one nudge pass: the scene data (borrowed, constant) plus
/// this route's clearance and original endpoint gap (which the trial must not deepen).
struct Safety<'a> {
    obstacles: &'a [Rect],
    endpoints: &'a [Rect],
    clearance: f64,
    orig_gap: f64,
}

/// Spread shared / near-parallel runs onto clean tracks, committing only the
/// separations that keep every wire clear of nodes. Mutates each wire's polyline.
pub fn nudge(wires: &mut [RoutedWire], nodes: &[PlacedNode]) {
    let scenes = build_scenes(wires, nodes);
    nudge_with(wires, &scenes, true);
}

/// Nudge with precomputed [`WireScene`]s — the side-search reuses one set across all
/// the candidates it evaluates, so the scene index / obstacle sets aren't rebuilt per
/// call. `scenes` is indexed like `wires`. `thorough` sweeps the full band of track
/// centres for the best node-safe placement; the search passes `false` for a coarser,
/// much cheaper sweep — the crossing count it optimises is set by the track *order*,
/// not the exact centre, so the cheaper sweep ranks candidates just as well.
pub fn nudge_with(wires: &mut [RoutedWire], scenes: &[WireScene], thorough: bool) {
    let segs = collect_interior(wires);
    let originals: Vec<Vec<Pt>> = wires.iter().map(|w| w.path.clone()).collect();
    let safety: Vec<Safety> = wires
        .iter()
        .zip(&originals)
        .zip(scenes)
        .map(|((w, orig), sc)| Safety {
            obstacles: &sc.obstacles,
            endpoints: &sc.endpoints,
            clearance: oracle::clearance(&w.attrs),
            orig_gap: endpoint_gap(orig, &sc.endpoints),
        })
        .collect();
    let mut moved: BTreeMap<(usize, usize), f64> = BTreeMap::new();

    for group in cluster(&segs) {
        let mut affected: Vec<usize> = group.iter().map(|&i| segs[i].wire).collect();
        affected.sort_unstable();
        affected.dedup();
        // Among placements that keep every affected wire clear of nodes, commit the
        // one introducing the fewest crossings between those wires (B3): a bundle's
        // rails cross only when ordered against their stubs, so trying the track
        // orderings (the sorted one first — unchanged clusters stay put) and
        // scoring by crossings reorders them apart. Widest separation first, then
        // narrower (C5 overflow). A run with no safe placement is left as it was.
        let mut best: Option<(usize, Placement)> = None;
        'search: for order in track_orders(&segs, &group) {
            for trial in candidate_placements(&segs, &order, thorough) {
                // Apply the trial onto `moved` in place (a cluster's segments are
                // disjoint from already-moved ones, so its keys aren't present yet),
                // rebuild each affected wire ONCE, score, then revert — far cheaper
                // than cloning the whole map and rebuilding twice per candidate.
                for &(k, v) in &trial {
                    moved.insert(k, v);
                }
                let rebuilt: Vec<Vec<Pt>> = affected
                    .iter()
                    .map(|&wi| rebuild(&originals[wi], wi, &moved))
                    .collect();
                for (k, _) in &trial {
                    moved.remove(k);
                }
                let safe = affected
                    .iter()
                    .zip(&rebuilt)
                    .all(|(&wi, path)| is_safe(path, &originals[wi], &safety[wi]));
                if !safe {
                    continue;
                }
                let crossings = count_crossings(&rebuilt);
                if best.as_ref().map_or(true, |(b, _)| crossings < *b) {
                    best = Some((crossings, trial));
                }
                if crossings == 0 {
                    break 'search; // sorted order tried first, so this is the tidiest
                }
            }
        }
        if let Some((_, trial)) = best {
            moved.extend(trial);
        }
    }

    if moved.is_empty() {
        return;
    }
    for (wi, w) in wires.iter_mut().enumerate() {
        w.path = rebuild(&originals[wi], wi, &moved);
    }
}

/// One nudgeable (interior) segment lifted out of a wire's polyline.
struct Segment {
    wire: usize,
    seg: usize, // connects path[seg] → path[seg + 1]
    horizontal: bool,
    pos: f64, // the constant coordinate (y if horizontal, else x)
    lo: f64,
    hi: f64, // extent along the varying axis
    clearance: f64,
    // How far the segment may slide before a neighbouring stub collapses: each
    // adjacent perpendicular segment's far end pins one side.
    lower: f64,
    upper: f64,
}

/// Lift every interior segment (not the first or last — those hold the ports).
fn collect_interior(wires: &[RoutedWire]) -> Vec<Segment> {
    let mut segs = Vec::new();
    for (wire, w) in wires.iter().enumerate() {
        let p = &w.path;
        if p.len() < 4 {
            continue; // < 2 interior vertices ⇒ no interior segment
        }
        let clearance = oracle::clearance(&w.attrs);
        for seg in 1..p.len() - 2 {
            let (a, b) = (p[seg], p[seg + 1]);
            let horizontal = (a.1 - b.1).abs() < EPS;
            let perp = |pt: Pt| if horizontal { pt.1 } else { pt.0 };
            let (pos, lo, hi) = if horizontal {
                (a.1, a.0.min(b.0), a.0.max(b.0))
            } else {
                (a.0, a.1.min(b.1), a.1.max(b.1))
            };
            // Each neighbour's far end caps the slide on the side it sits: a stub
            // collapses if the segment reaches it.
            let (mut lower, mut upper) = (f64::NEG_INFINITY, f64::INFINITY);
            for c in [perp(p[seg - 1]), perp(p[seg + 2])] {
                if c < pos - EPS {
                    lower = lower.max(c);
                } else if c > pos + EPS {
                    upper = upper.min(c);
                }
            }
            segs.push(Segment {
                wire,
                seg,
                horizontal,
                pos,
                lo,
                hi,
                clearance,
                lower,
                upper,
            });
        }
    }
    segs
}

/// Group segments that share a line and overlap: same orientation, extents
/// overlapping, and closer than their separation. Transitive (union-find), so a
/// run of segments stepping `separation` apart still forms one channel.
fn cluster(segs: &[Segment]) -> Vec<Vec<usize>> {
    let mut uf = UnionFind::new(segs.len());
    for i in 0..segs.len() {
        for j in i + 1..segs.len() {
            let (a, b) = (&segs[i], &segs[j]);
            let separation = a.clearance.max(b.clearance);
            if a.horizontal == b.horizontal
                && (a.pos - b.pos).abs() < separation - EPS
                && range_overlap(a.lo, a.hi, b.lo, b.hi)
            {
                uf.union(i, j);
            }
        }
    }
    uf.groups()
}

/// Candidate track placements for a channel, best first: widest separation before
/// narrower, and within each the most centred band position before the edges. The
/// caller commits the first one that proves node-safe; a single line over a clear
/// channel separates fully, a crowded one compacts (C5 overflow), and one boxed in
/// finds nothing and is left alone.
fn candidate_placements(segs: &[Segment], order: &[usize], thorough: bool) -> Vec<Placement> {
    let k = order.len();
    if k < 2 {
        return Vec::new();
    }
    let sep = order.iter().map(|&i| segs[i].clearance).fold(0.0, f64::max);

    // The band the tracks must stay within so no neighbour stub collapses, with a
    // sliver of margin to keep each stub a real segment.
    const MARGIN: f64 = 1.0;
    let lo = order
        .iter()
        .map(|&i| segs[i].lower)
        .fold(f64::NEG_INFINITY, f64::max)
        + MARGIN;
    let hi = order
        .iter()
        .map(|&i| segs[i].upper)
        .fold(f64::INFINITY, f64::min)
        - MARGIN;
    let mean = order.iter().map(|&i| segs[i].pos).sum::<f64>() / k as f64;

    let track = |centre: f64, spacing: f64, rank: usize| {
        centre + (rank as f64 - (k as f64 - 1.0) / 2.0) * spacing
    };

    let mut out = Vec::new();
    for frac in [1.0, 0.85, 0.7, 0.55, 0.4, 0.25] {
        let spacing = sep * frac;
        let span = (k as f64 - 1.0) * spacing;
        // Sweep the band of feasible centres (a node can block the obvious middle,
        // so the clear gap may be off to one side); the stub bounds may be open, so
        // cap the sweep to a finite window around the run. Try centres nearest the
        // original position first, so a wire moves as little as it must.
        let window = span + 8.0 * sep;
        let c_lo = (lo + span / 2.0).max(mean - window);
        let c_hi = (hi - span / 2.0).min(mean + window);
        if c_hi < c_lo {
            continue; // this spacing won't fit between the stub bounds
        }
        let steps: usize = if thorough { 16 } else { 4 };
        let anchor = mean.clamp(c_lo, c_hi);
        let mut centres: Vec<f64> = (0..=steps)
            .map(|i| c_lo + (c_hi - c_lo) * i as f64 / steps as f64)
            .collect();
        centres.sort_by(|a, b| (a - anchor).abs().total_cmp(&(b - anchor).abs()));
        for centre in centres {
            out.push(
                order
                    .iter()
                    .enumerate()
                    .map(|(rank, &i)| ((segs[i].wire, segs[i].seg), track(centre, spacing, rank)))
                    .collect(),
            );
        }
    }
    out
}

/// The track orderings to try for a cluster, tidiest first: the position-sorted
/// order (so a cluster with no crossing keeps its natural layout), then the other
/// permutations so a crossing can be reordered away. Capped — a wide channel keeps
/// the sorted order rather than exploring a factorial of layouts.
fn track_orders(segs: &[Segment], group: &[usize]) -> Vec<Vec<usize>> {
    let mut sorted = group.to_vec();
    sorted.sort_by(|&a, &b| {
        segs[a]
            .pos
            .total_cmp(&segs[b].pos)
            .then(segs[a].wire.cmp(&segs[b].wire))
            .then(segs[a].seg.cmp(&segs[b].seg))
    });
    if sorted.len() > 4 {
        return vec![sorted];
    }
    let mut out = Vec::new();
    heap_permute(sorted.len(), &mut sorted, &mut out);
    out
}

/// Heap's algorithm: every permutation of `a`, with `a` itself emitted first.
fn heap_permute(k: usize, a: &mut [usize], out: &mut Vec<Vec<usize>>) {
    if k <= 1 {
        out.push(a.to_vec());
        return;
    }
    for i in 0..k {
        heap_permute(k - 1, a, out);
        if k % 2 == 0 {
            a.swap(i, k - 1);
        } else {
            a.swap(0, k - 1);
        }
    }
}

/// Count perpendicular crossings between the (already rebuilt) affected wires — the
/// score the nudge minimises when choosing a track order (B3).
fn count_crossings(paths: &[Vec<Pt>]) -> usize {
    use super::geometry::perp_crossing;
    let segs = |p: &[Pt]| -> Vec<(Pt, Pt)> { p.windows(2).map(|s| (s[0], s[1])).collect() };
    let mut count = 0;
    for i in 0..paths.len() {
        for j in (i + 1)..paths.len() {
            let (si, sj) = (segs(&paths[i]), segs(&paths[j]));
            for a in &si {
                for b in &sj {
                    if perp_crossing(*a, *b) {
                        count += 1;
                    }
                }
            }
        }
    }
    count
}

/// Rebuild a wire's polyline from its (possibly shifted) segment positions: ports
/// pinned, every interior vertex the intersection of its two segments.
fn rebuild(original: &[Pt], wire: usize, moved: &BTreeMap<(usize, usize), f64>) -> Vec<Pt> {
    let n = original.len();
    if n < 3 {
        return original.to_vec();
    }
    let horizontal: Vec<bool> = (0..n - 1)
        .map(|i| (original[i].1 - original[i + 1].1).abs() < EPS)
        .collect();
    let pos: Vec<f64> = (0..n - 1)
        .map(|i| {
            let base = if horizontal[i] {
                original[i].1
            } else {
                original[i].0
            };
            moved.get(&(wire, i)).copied().unwrap_or(base)
        })
        .collect();

    let mut out = Vec::with_capacity(n);
    out.push(original[0]); // port — pinned
    for i in 1..n - 1 {
        // Vertex between perpendicular segments i-1 and i: x from whichever is
        // vertical, y from whichever is horizontal.
        let v = if horizontal[i - 1] {
            (pos[i], pos[i - 1])
        } else {
            (pos[i - 1], pos[i])
        };
        out.push(v);
    }
    out.push(original[n - 1]); // port — pinned
    clean(out)
}

/// A nudged wire is safe when it pierces or grazes no non-endpoint node (B1/B2
/// wire↔node), keeps `clearance` from its own endpoint nodes except its attaching
/// stubs (B2), still attaches perpendicularly at both ports (A2), and doesn't
/// cross itself (A5). The endpoint check is *relative*: some skims are
/// geometrically forced (a node within `clearance` of a port), so the nudge may
/// preserve one but must never deepen it or create a new one.
fn is_safe(path: &[Pt], original: &[Pt], safety: &Safety) -> bool {
    if !is_orthogonal(path) || !attachment_preserved(path, original) {
        return false;
    }
    let segs: Vec<(Pt, Pt)> = path.windows(2).map(|s| (s[0], s[1])).collect();

    for obs in safety.obstacles {
        let bad = |s: &(Pt, Pt)| {
            rect_penetrated_by(*obs, *s) || seg_rect_distance(*obs, *s) + EPS < safety.clearance
        };
        if segs.iter().any(bad) {
            return false;
        }
    }

    if !safety.endpoints.is_empty() {
        let after = endpoint_gap(path, safety.endpoints);
        if after < 0.0 {
            return false; // pierced its own endpoint
        }
        if after + EPS < safety.clearance.min(safety.orig_gap) {
            return false; // would deepen / create an endpoint skim
        }
    }
    !self_crosses(&segs)
}

/// Smallest distance from a wire's non-stub (interior) segments to its own endpoint
/// rects — `∞` when there are no interior segments, negative when one pierces an
/// endpoint's interior. The attaching stubs (first/last) are exempt.
fn endpoint_gap(path: &[Pt], endpoints: &[Rect]) -> f64 {
    let segs: Vec<(Pt, Pt)> = path.windows(2).map(|s| (s[0], s[1])).collect();
    if segs.len() < 3 {
        return f64::INFINITY;
    }
    let interior = &segs[1..segs.len() - 1];
    let mut gap = f64::INFINITY;
    for r in endpoints {
        for s in interior {
            if rect_penetrated_by(*r, *s) {
                return f64::NEG_INFINITY;
            }
            gap = gap.min(seg_rect_distance(*r, *s));
        }
    }
    gap
}

/// Every segment is axis-aligned, non-zero, and turns 90° from the last (A1) — a
/// placement that leaves a collinear or zero-length artifact (e.g. a track that
/// grazed a neighbour) is rejected rather than shipped.
fn is_orthogonal(path: &[Pt]) -> bool {
    if path.len() < 2 {
        return false;
    }
    let mut prev_h: Option<bool> = None;
    for s in path.windows(2) {
        let (h, v) = ((s[0].1 - s[1].1).abs() < EPS, (s[0].0 - s[1].0).abs() < EPS);
        if h == v || prev_h == Some(h) {
            return false; // diagonal/zero-length, or two same-orientation in a row
        }
        prev_h = Some(h);
    }
    true
}

/// The first and last segments still run in their original direction — so the
/// nudge didn't collapse a port stub and flip the attaching segment (A2).
fn attachment_preserved(path: &[Pt], original: &[Pt]) -> bool {
    let dir = |p: &[Pt], i: usize, j: usize| (p[i].0 - p[j].0).abs() < EPS; // vertical?
    path.len() >= 2
        && original.len() >= 2
        && dir(path, 0, 1) == dir(original, 0, 1)
        && dir(path, path.len() - 1, path.len() - 2)
            == dir(original, original.len() - 1, original.len() - 2)
}

/// Any non-adjacent segments meet → the wire crosses itself (A5).
fn self_crosses(segs: &[(Pt, Pt)]) -> bool {
    use super::geometry::segments_intersect;
    for i in 0..segs.len() {
        for j in i + 2..segs.len() {
            if segments_intersect(segs[i], segs[j]) {
                return true;
            }
        }
    }
    false
}

/// Minimal union-find over segment indices.
struct UnionFind {
    parent: Vec<usize>,
}

impl UnionFind {
    fn new(n: usize) -> Self {
        Self {
            parent: (0..n).collect(),
        }
    }

    fn find(&mut self, x: usize) -> usize {
        let mut root = x;
        while self.parent[root] != root {
            root = self.parent[root];
        }
        let mut cur = x;
        while self.parent[cur] != root {
            let next = self.parent[cur];
            self.parent[cur] = root;
            cur = next;
        }
        root
    }

    fn union(&mut self, a: usize, b: usize) {
        let (ra, rb) = (self.find(a), self.find(b));
        if ra != rb {
            self.parent[ra] = rb;
        }
    }

    fn groups(&mut self) -> Vec<Vec<usize>> {
        let mut by_root: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
        for i in 0..self.parent.len() {
            let r = self.find(i);
            by_root.entry(r).or_default().push(i);
        }
        by_root.into_values().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::resolve::{AttrMap, Markers};
    use crate::span::Span;

    fn nodes(src: &str) -> Vec<PlacedNode> {
        let toks = crate::lexer::lex(src).expect("lex");
        let file = crate::parser::parse(&toks).expect("parse");
        let prog = crate::resolve::resolve(file).expect("resolve");
        crate::layout::layout(&prog).expect("layout").nodes
    }

    fn wire(path: Vec<Pt>, from: &str, to: &str) -> RoutedWire {
        RoutedWire {
            path,
            markers: Markers::default(),
            attrs: AttrMap::new(),
            texts: Vec::new(),
            data_from: from.into(),
            data_to: to.into(),
            seg_from: from.into(),
            seg_to: to.into(),
            decl_span: Span::empty(),
            fan_from: None,
            fan_to: None,
        }
    }

    // Two boxes far apart with a wide channel between them (`b` is a reserved
    // side word, so the ids are `aa`/`bb`).
    const SCENE: &str = "{ |scene| layout:row gap:200 }\n\
                         aa |rect| size:(40,200)\n\
                         bb |rect| size:(40,200)\n";

    #[test]
    fn overlapping_interior_runs_split_into_tracks() {
        // Two aa→bb wires whose middle horizontal segments sit on top of each
        // other end up `separation` (16) apart — the wide channel makes it safe.
        let ns = nodes(SCENE);
        let mut wires = vec![
            wire(
                vec![(-100.0, -8.0), (0.0, -8.0), (0.0, 8.0), (100.0, 8.0)],
                "aa",
                "bb",
            ),
            wire(
                vec![(-100.0, 8.0), (0.0, 8.0), (0.0, -8.0), (100.0, -8.0)],
                "aa",
                "bb",
            ),
        ];
        nudge(&mut wires, &ns);
        let gap = (wires[0].path[1].0 - wires[1].path[1].0).abs();
        assert!((gap - 16.0).abs() < 1e-6, "tracks 16 apart, got {gap}");
    }

    #[test]
    fn bundle_rails_are_ordered_to_avoid_a_self_cross() {
        // Two parallel wires leave aa's right at slightly different heights, share
        // a down-rail, and drop. Splitting that rail in declaration order would
        // send the upper wire's descent across the lower wire's stub. The nudge
        // must pick the track order that avoids the crossing.
        let ns = nodes(SCENE);
        let mut wires = vec![
            wire(
                vec![(-100.0, -4.0), (0.0, -4.0), (0.0, 100.0), (100.0, 100.0)],
                "aa",
                "bb",
            ),
            wire(
                vec![(-100.0, 4.0), (0.0, 4.0), (0.0, 100.0), (100.0, 100.0)],
                "aa",
                "bb",
            ),
        ];
        nudge(&mut wires, &ns);
        let count = perp_crossings(&wires[0], &wires[1]);
        assert_eq!(
            count, 0,
            "the bundle must not cross itself: {count} crossings"
        );
    }

    fn perp_crossings(a: &RoutedWire, b: &RoutedWire) -> usize {
        use super::super::geometry::perp_crossing;
        let segs =
            |w: &RoutedWire| -> Vec<(Pt, Pt)> { w.path.windows(2).map(|s| (s[0], s[1])).collect() };
        let (sa, sb) = (segs(a), segs(b));
        sa.iter()
            .flat_map(|x| sb.iter().map(move |y| (x, y)))
            .filter(|(x, y)| perp_crossing(**x, **y))
            .count()
    }

    #[test]
    fn a_lone_wire_is_left_untouched() {
        let ns = nodes(SCENE);
        let mut wires = vec![wire(
            vec![(-100.0, 0.0), (0.0, 0.0), (0.0, 20.0), (100.0, 20.0)],
            "aa",
            "bb",
        )];
        let before = wires[0].path.clone();
        nudge(&mut wires, &ns);
        assert_eq!(wires[0].path, before);
    }
}
