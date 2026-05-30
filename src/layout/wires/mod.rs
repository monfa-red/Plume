//! Wire routing — orchestration.
//!
//! Routes every wire to the contract in WIRING.md via the libavoid model
//! (`ports` side/slot choice → `route` visibility-graph A\* → `nudge` global track
//! assignment). It runs in **two passes**: a blind provisional pass whose real
//! geometry feeds back as `ports::PlanHint`s, then an informed second pass. On top,
//! a **crossing-aware convergence** alternative unifies bundles that share an
//! endpoint node and cross, kept only when it strictly betters the scorecard
//! (`quality`) — so the result is monotone and a deterministic function of input.
//! `validate` is the independent contract checker that gates the whole thing.

mod geometry;
mod graph;
mod nudge;
mod oracle;
mod ports;
mod route;
mod scene;
mod score;
mod text;
mod validate;

pub use validate::{validate_routing, Rule, Severity, Violation};

use crate::ast::Side;
use crate::error::Error;
use crate::layout::ir::{PlacedNode, RoutedWire};
use crate::resolve::{MarkerKind, Markers, Program, ResolvedEndpoint, ResolvedWire};
use geometry::{dumb_route, perp_crossing, rect_penetrated_by, seg_rect_distance, Pt, Rect, EPS};
use ports::{Plan, PlanHint, SegReq};
use scene::SceneIndex;
use score::{score, Score};
use text::place_texts;

/// Where one routed segment belongs in its source wire — so markers land on the
/// chain's outer ends and labels on its middle segment.
struct SegMeta<'a> {
    wire: &'a ResolvedWire,
    i: usize,
    segs: usize,
}

/// The flattened routing work: one entry per chain edge, in routing order.
/// `chains[s]` is the source-wire index of segment `s` (a chain's segments share
/// it), so convergence never mistakes a chain passing through a node for two
/// bundles meeting there.
struct Reqs<'a> {
    reqs: Vec<SegReq>,
    metas: Vec<SegMeta<'a>>,
    chains: Vec<usize>,
}

pub fn route_wires(program: &Program, nodes: &[PlacedNode]) -> Result<Vec<RoutedWire>, Error> {
    let index = SceneIndex::build(nodes);
    let fans = fan_ids(&program.wires);
    let Reqs {
        reqs,
        metas,
        chains,
    } = build_reqs(program, &index, &fans)?;

    // Pass 1 (the libavoid two-pass) — side/slot choice runs blind, from
    // straight-line guesses; the provisional routes then report where each wire
    // *actually* went.
    let plan0 = ports::plan(&reqs, &[]);
    let provisional: Vec<Vec<Pt>> = reqs
        .iter()
        .zip(&plan0)
        .map(|(req, plan)| route_one(req, plan, nodes))
        .collect();
    let base = derive_hints(&reqs, &provisional);

    // Candidate A — the informed second pass: C4 orders slots by real exit
    // heading, and a side that skimmed its own node is re-elected (obstacle-aware
    // C1). This is the established result.
    let plan_a = ports::plan(&reqs, &base);
    let a = finish(&reqs, &metas, &plan_a, nodes);

    // Candidate B — crossing-aware convergence: bundles that share an endpoint node
    // and cross in pass 1 are unified onto ONE side so they nest (B3). Each such
    // group is tried on every side its members occupy, the best kept, greedily
    // across groups; a trial is adopted only when it strictly betters the contract
    // scorecard (`quality`). So crossings are monotone non-increasing and the output
    // stays a deterministic function of the input — a no-op when nothing helps.
    let groups = converge_groups(&reqs, &chains, &provisional);
    if groups.is_empty() {
        return Ok(a);
    }
    let mut best = a;
    let mut best_q = score(&best, nodes);
    let mut overlay = vec![(None, None); reqs.len()];
    for group in &groups {
        if let Some((trial, wires, q)) = try_unify_group(
            group, &plan_a, &overlay, &base, &reqs, &metas, nodes, best_q,
        ) {
            overlay = trial;
            best = wires;
            best_q = q;
        }
    }
    Ok(best)
}

/// Per-segment side overrides for the two ends — the convergence reside overlay.
type Resides = Vec<(Option<Side>, Option<Side>)>;

/// One convergence trial: the overlay that produced it, the resulting wires, and
/// the scorecard they earned.
type UnifyTrial = (Resides, Vec<RoutedWire>, Score);

/// Try unifying one convergence group onto each side its members occupy (in
/// candidate A's plan), layered on the accumulated `overlay`. Returns the trial of
/// the side that best betters `best_q`, or `None` when none does — so the search is
/// monotone and, sides tried in a fixed order, deterministic. Forced ends can't
/// move and are left in place.
#[allow(clippy::too_many_arguments)]
fn try_unify_group(
    group: &[(usize, bool)],
    plan_a: &[Plan],
    overlay: &[(Option<Side>, Option<Side>)],
    base: &[PlanHint],
    reqs: &[SegReq],
    metas: &[SegMeta],
    nodes: &[PlacedNode],
    best_q: Score,
) -> Option<UnifyTrial> {
    let mut sides: Vec<Side> = group.iter().map(|&(r, e)| side_of(&plan_a[r], e)).collect();
    sides.sort_by_key(|&s| side_rank(s));
    sides.dedup();

    let mut best: Option<UnifyTrial> = None;
    let mut bar = best_q;
    for side in sides {
        let mut trial = overlay.to_vec();
        for &(r, e) in group {
            let forced = if e {
                reqs[r].forced_a
            } else {
                reqs[r].forced_b
            };
            if forced.is_some() {
                continue;
            }
            if e {
                trial[r].0 = Some(side);
            } else {
                trial[r].1 = Some(side);
            }
        }
        let wires = finish(
            reqs,
            metas,
            &ports::plan(reqs, &overlay_resides(base, &trial)),
            nodes,
        );
        let q = score(&wires, nodes);
        if q < bar {
            bar = q;
            best = Some((trial, wires, q));
        }
    }
    best
}

/// WIRING C1 side order, as a sortable rank (right → bottom → left → top is the
/// tie-break; here we only need a stable total order for determinism).
fn side_rank(s: Side) -> u8 {
    match s {
        Side::Top => 0,
        Side::Right => 1,
        Side::Bottom => 2,
        Side::Left => 3,
    }
}

/// Flatten every chain `e0 -> … -> eN` into one segment-request per pair, in
/// declaration-then-chain order — the deterministic routing order. A fan group's
/// shared end carries a trunk id (E2): siblings collapse to one slot there and the
/// validator exempts their coincident run.
fn build_reqs<'a>(
    program: &'a Program,
    index: &SceneIndex,
    fans: &FanIds,
) -> Result<Reqs<'a>, Error> {
    let mut reqs = Vec::new();
    let mut metas = Vec::new();
    let mut chains = Vec::new();
    for (wi, w) in program.wires.iter().enumerate() {
        let eps = &w.endpoints;
        if eps.len() < 2 {
            continue; // resolve guarantees ≥ 2; be defensive
        }
        let clearance = oracle::clearance(&w.attrs);
        let last = eps.len() - 2;
        for i in 0..eps.len() - 1 {
            reqs.push(SegReq {
                a_node: eps[i].path.clone(),
                a: rect_for(index, &eps[i])?,
                forced_a: eps[i].side,
                b_node: eps[i + 1].path.clone(),
                b: rect_for(index, &eps[i + 1])?,
                forced_b: eps[i + 1].side,
                clearance,
                // Only the chain's outer ends can be a fan hub: its shared source
                // is the first segment's `a`, its shared target the last's `b`.
                fan_a: if i == 0 { fans.source(w) } else { None },
                fan_b: if i == last { fans.target(w) } else { None },
            });
            metas.push(SegMeta {
                wire: w,
                i,
                segs: eps.len() - 1,
            });
            chains.push(wi); // every segment of one chain shares its wire index
        }
    }
    Ok(Reqs {
        reqs,
        metas,
        chains,
    })
}

/// Route every segment with `plans`, build each `RoutedWire`, then run the global
/// nudge (track assignment / separation / crossing-min within clusters).
fn finish(
    reqs: &[SegReq],
    metas: &[SegMeta],
    plans: &[Plan],
    nodes: &[PlacedNode],
) -> Vec<RoutedWire> {
    let mut out = Vec::with_capacity(reqs.len());
    for (req, (plan, meta)) in reqs.iter().zip(plans.iter().zip(metas)) {
        out.push(build_wire(meta, req, route_one(req, plan, nodes)));
    }
    nudge::nudge(&mut out, nodes);
    out
}

/// Lay the convergence reside-overlay onto the base hints: a unification side wins
/// over a base skim-reside (it is the more global decision); leads are kept.
fn overlay_resides(base: &[PlanHint], unify: &[(Option<Side>, Option<Side>)]) -> Vec<PlanHint> {
    base.iter()
        .zip(unify)
        .map(|(h, &(ua, ub))| PlanHint {
            lead_a: h.lead_a,
            lead_b: h.lead_b,
            reside_a: ua.or(h.reside_a),
            reside_b: ub.or(h.reside_b),
        })
        .collect()
}

/// Crossing-aware convergence (WIRING B3 / C1). Find the groups of wire-ends that
/// **converge and cross**: wires that share an endpoint node and whose provisional
/// routes perpendicular-cross there picked *different* sides of that node, so they
/// can't nest. Each group is the set of `(segment, end)` meeting at one node;
/// unifying them onto a single side lets them nest. Excluded: fan siblings (E2 — a
/// permitted coincident run) and two segments of one chain (`chains[i]==chains[j]`,
/// the wire passing through the node, not two bundles meeting). Groups are keyed by
/// node and their members sorted, so the result is deterministic.
fn converge_groups(
    reqs: &[SegReq],
    chains: &[usize],
    provisional: &[Vec<Pt>],
) -> Vec<Vec<(usize, bool)>> {
    use std::collections::{BTreeMap, BTreeSet};
    let n = reqs.len();
    let mut groups: BTreeMap<&str, BTreeSet<(usize, bool)>> = BTreeMap::new();
    for i in 0..n {
        if reqs[i].a_node == reqs[i].b_node {
            continue; // self-loop — has no convergence partner
        }
        for j in (i + 1)..n {
            if chains[i] == chains[j]
                || reqs[j].a_node == reqs[j].b_node
                || fan_pair(&reqs[i], &reqs[j])
            {
                continue;
            }
            if !paths_cross(&provisional[i], &provisional[j]) {
                continue;
            }
            for node in shared_nodes(&reqs[i], &reqs[j]) {
                if let (Some(ei), Some(ej)) = (end_at(&reqs[i], node), end_at(&reqs[j], node)) {
                    let g = groups.entry(node).or_default();
                    g.insert((i, ei));
                    g.insert((j, ej));
                }
            }
        }
    }
    groups
        .into_values()
        .map(|s| s.into_iter().collect())
        .collect()
}

/// Two segment-requests are fan siblings when they share a fan-trunk id on any end.
fn fan_pair(a: &SegReq, b: &SegReq) -> bool {
    let shares = |x: Option<u32>| x.is_some() && (x == b.fan_a || x == b.fan_b);
    shares(a.fan_a) || shares(a.fan_b)
}

/// Do the two provisional polylines cross perpendicularly anywhere (a B3 crossing)?
fn paths_cross(a: &[Pt], b: &[Pt]) -> bool {
    let segs = |p: &[Pt]| -> Vec<(Pt, Pt)> { p.windows(2).map(|s| (s[0], s[1])).collect() };
    let (sa, sb) = (segs(a), segs(b));
    sa.iter().any(|x| sb.iter().any(|y| perp_crossing(*x, *y)))
}

/// The endpoint nodes two segment-requests have in common.
fn shared_nodes<'a>(a: &'a SegReq, b: &SegReq) -> Vec<&'a str> {
    let mut out = Vec::new();
    for node in [a.a_node.as_str(), a.b_node.as_str()] {
        if (node == b.a_node || node == b.b_node) && !out.contains(&node) {
            out.push(node);
        }
    }
    out
}

/// Which end of a request touches `node`: `Some(true)` = the a-end, `Some(false)` =
/// the b-end, `None` if neither.
fn end_at(r: &SegReq, node: &str) -> Option<bool> {
    if r.a_node == node {
        Some(true)
    } else if r.b_node == node {
        Some(false)
    } else {
        None
    }
}

/// The side a plan attaches on at the named end.
fn side_of(plan: &Plan, is_a: bool) -> Side {
    if is_a {
        plan.side_a
    } else {
        plan.side_b
    }
}

/// Route one segment: a self-loop wraps a corner (E3); otherwise A* around the
/// obstacles for its pair, falling back to the dumb route only if boxed in.
fn route_one(req: &SegReq, plan: &Plan, nodes: &[PlacedNode]) -> Vec<Pt> {
    if req.a_node == req.b_node {
        return route::self_loop(req.a, plan.side_a, plan.side_b, req.clearance);
    }
    let obstacles = scene::obstacles_for(nodes, [&req.a_node, &req.b_node]);
    route::route(
        plan.port_a,
        plan.side_a,
        plan.port_b,
        plan.side_b,
        &obstacles,
        req.clearance,
        [req.a, req.b],
    )
    .unwrap_or_else(|| dumb_route(plan.port_a, plan.side_a, plan.port_b, plan.side_b))
}

/// Read each provisional route's geometry back into planning hints (the two-pass
/// feedback): where each end heads just past its stub, and — when an end's route
/// skimmed its own node — the side it turned toward, so the next pass leaves there.
fn derive_hints(reqs: &[SegReq], paths: &[Vec<Pt>]) -> Vec<PlanHint> {
    reqs.iter()
        .zip(paths)
        .map(|(req, path)| {
            if req.a_node == req.b_node {
                return PlanHint::default(); // a self-loop routes on its own
            }
            PlanHint {
                lead_a: lead_point(path, true),
                lead_b: lead_point(path, false),
                reside_a: reside(path, req.a, req.forced_a, req.clearance, true),
                reside_b: reside(path, req.b, req.forced_b, req.clearance, false),
            }
        })
        .collect()
}

/// The point a wire heads to just past its attaching stub (its second vertex from
/// the relevant end) — the crossing-free C4 order key. `None` for a degenerate path.
fn lead_point(path: &[Pt], is_a: bool) -> Option<Pt> {
    let n = path.len();
    if n < 2 {
        return None;
    }
    let off = 2.min(n - 1);
    Some(if is_a { path[off] } else { path[n - 1 - off] })
}

/// The side an end should move to: `Some` only when its provisional route skimmed
/// its own node (and the side isn't forced) — the perpendicular side the wire
/// turned toward, so leaving from there avoids threading the sub-clearance gap.
fn reside(
    path: &[Pt],
    rect: Rect,
    forced: Option<Side>,
    clearance: f64,
    is_a: bool,
) -> Option<Side> {
    if forced.is_some() || !end_skims(path, rect, clearance) {
        return None;
    }
    let n = path.len();
    if n < 3 {
        return None;
    }
    let (from, to) = if is_a {
        (path[1], path[2])
    } else {
        (path[n - 2], path[n - 3])
    };
    let (dx, dy) = (to.0 - from.0, to.1 - from.1);
    Some(if dx.abs() >= dy.abs() {
        if dx >= 0.0 {
            Side::Right
        } else {
            Side::Left
        }
    } else if dy >= 0.0 {
        Side::Bottom
    } else {
        Side::Top
    })
}

/// Does a non-stub (interior) segment run within `clearance` of `rect` — i.e. did
/// the wire skim this endpoint?
fn end_skims(path: &[Pt], rect: Rect, clearance: f64) -> bool {
    if path.len() < 4 {
        return false; // < 3 segments ⇒ no interior segment, only stubs
    }
    let segs: Vec<(Pt, Pt)> = path.windows(2).map(|s| (s[0], s[1])).collect();
    segs[1..segs.len() - 1]
        .iter()
        .any(|s| rect_penetrated_by(rect, *s) || seg_rect_distance(rect, *s) + EPS < clearance)
}

/// Assemble one `RoutedWire`, placing markers on the chain's outer ends and the
/// wire's labels on its middle segment.
fn build_wire(meta: &SegMeta, req: &SegReq, path: Vec<(f64, f64)>) -> RoutedWire {
    let w = meta.wire;
    let eps = &w.endpoints;
    RoutedWire {
        markers: Markers {
            start: if meta.i == 0 {
                w.markers.start
            } else {
                MarkerKind::None
            },
            end: if meta.i == meta.segs - 1 {
                w.markers.end
            } else {
                MarkerKind::None
            },
        },
        texts: if meta.i == meta.segs / 2 {
            place_texts(&path, &w.texts)
        } else {
            Vec::new()
        },
        path,
        attrs: w.attrs.clone(),
        data_from: eps[0].path.clone(),
        data_to: eps[meta.segs].path.clone(),
        seg_from: req.a_node.clone(),
        seg_to: req.b_node.clone(),
        decl_span: w.span,
        fan_from: req.fan_a,
        fan_to: req.fan_b,
    }
}

/// Fan-trunk ids keyed by declaration + hub node. Within one declaration (a wire
/// statement, identified by its span), a node that is the shared **source** of ≥2
/// expanded wires is a fan-out hub, and the shared **target** of ≥2 a fan-in hub
/// (E2 / WIRING defs). Each hub gets a stable id; siblings collapse onto one slot
/// there and the validator exempts their coincident trunk. Chains (one wire) and
/// bundles (separate declarations) form no hub, so they stay fully separated.
struct FanIds {
    source: std::collections::BTreeMap<(usize, usize, String), u32>,
    target: std::collections::BTreeMap<(usize, usize, String), u32>,
}

impl FanIds {
    fn source(&self, w: &ResolvedWire) -> Option<u32> {
        let eps = &w.endpoints;
        self.source
            .get(&(w.span.start, w.span.end, eps[0].path.clone()))
            .copied()
    }

    fn target(&self, w: &ResolvedWire) -> Option<u32> {
        let eps = &w.endpoints;
        self.target
            .get(&(w.span.start, w.span.end, eps[eps.len() - 1].path.clone()))
            .copied()
    }
}

fn fan_ids(wires: &[ResolvedWire]) -> FanIds {
    use std::collections::BTreeMap;
    let mut src_count: BTreeMap<(usize, usize, String), usize> = BTreeMap::new();
    let mut tgt_count: BTreeMap<(usize, usize, String), usize> = BTreeMap::new();
    for w in wires {
        let eps = &w.endpoints;
        if eps.len() < 2 {
            continue;
        }
        *src_count
            .entry((w.span.start, w.span.end, eps[0].path.clone()))
            .or_default() += 1;
        *tgt_count
            .entry((w.span.start, w.span.end, eps[eps.len() - 1].path.clone()))
            .or_default() += 1;
    }
    // Mint ids from a single counter (so source and target ids never collide) in
    // sorted key order, keeping the assignment deterministic.
    let mut next = 0u32;
    let mut mint = |counts: BTreeMap<(usize, usize, String), usize>| {
        let mut ids = BTreeMap::new();
        for (key, count) in counts {
            if count >= 2 {
                ids.insert(key, next);
                next += 1;
            }
        }
        ids
    };
    let source = mint(src_count);
    let target = mint(tgt_count);
    FanIds { source, target }
}

fn rect_for(index: &SceneIndex, ep: &ResolvedEndpoint) -> Result<Rect, Error> {
    index.rect(&ep.path).ok_or_else(|| {
        Error::at(
            ep.span,
            format!("wire endpoint '{}' has no placed node", ep.path),
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(min_x: f64, min_y: f64, max_x: f64, max_y: f64) -> Rect {
        Rect {
            min_x,
            min_y,
            max_x,
            max_y,
        }
    }

    fn seg_req(a_node: &str, b_node: &str) -> SegReq {
        SegReq {
            a_node: a_node.into(),
            a: rect(0.0, 0.0, 10.0, 10.0),
            forced_a: None,
            b_node: b_node.into(),
            b: rect(100.0, 0.0, 140.0, 40.0),
            forced_b: None,
            clearance: 16.0,
            fan_a: None,
            fan_b: None,
        }
    }

    // Two crossing routes whose ends both land on `r`: w0 enters r's bottom, w1
    // enters r's left; distinct chains (0, 1), so they're independent bundles.
    fn crossing_pair() -> (Vec<SegReq>, Vec<usize>, Vec<Vec<Pt>>) {
        let reqs = vec![seg_req("aa", "r"), seg_req("bb", "r")];
        let chains = vec![0, 1];
        let provisional = vec![
            vec![(0.0, 100.0), (120.0, 100.0), (120.0, 40.0)], // aa→r, into bottom
            vec![(200.0, 200.0), (50.0, 200.0), (50.0, 20.0), (100.0, 20.0)], // bb→r, into left
        ];
        (reqs, chains, provisional)
    }

    #[test]
    fn converging_wires_that_cross_form_one_group() {
        // They share target `r` and aren't fan siblings, so both target-ends are
        // grouped for unification onto a single side.
        let (reqs, chains, provisional) = crossing_pair();
        let groups = converge_groups(&reqs, &chains, &provisional);
        assert_eq!(groups, vec![vec![(0, false), (1, false)]]);
    }

    #[test]
    fn three_wires_converging_on_one_node_form_a_single_group() {
        // w0, w1, w2 all end at `r` and pairwise-cross → one group of three ends, so
        // all three unify onto one side rather than only the first pair.
        let reqs = vec![seg_req("aa", "r"), seg_req("bb", "r"), seg_req("cc", "r")];
        let chains = vec![0, 1, 2];
        let into_bottom = |x: f64| vec![(0.0, 100.0), (x, 100.0), (x, 40.0)];
        let into_left = |y: f64| vec![(200.0, y), (50.0, y), (50.0, 20.0), (100.0, 20.0)];
        let provisional = vec![into_bottom(120.0), into_left(200.0), into_left(180.0)];
        let groups = converge_groups(&reqs, &chains, &provisional);
        assert_eq!(groups, vec![vec![(0, false), (1, false), (2, false)]]);
    }

    #[test]
    fn converging_wires_that_do_not_cross_are_left_alone() {
        let reqs = vec![seg_req("aa", "r"), seg_req("bb", "r")];
        let chains = vec![0, 1];
        let provisional = vec![
            vec![(0.0, 100.0), (110.0, 100.0), (110.0, 40.0)],
            vec![(0.0, 140.0), (120.0, 140.0), (120.0, 40.0)],
        ];
        assert!(
            converge_groups(&reqs, &chains, &provisional).is_empty(),
            "no crossing → no convergence group"
        );
    }

    #[test]
    fn fan_siblings_that_cross_form_no_group() {
        // A fan trunk is a permitted coincident run (E2) — never unified against itself.
        let (mut reqs, chains, provisional) = crossing_pair();
        reqs[0].fan_b = Some(1);
        reqs[1].fan_b = Some(1);
        assert!(
            converge_groups(&reqs, &chains, &provisional).is_empty(),
            "fan siblings are exempt"
        );
    }

    #[test]
    fn non_converging_crossing_wires_form_no_group() {
        // Two wires that cross but share NO endpoint node are an ordinary B3
        // crossing, not a convergence.
        let reqs = vec![seg_req("aa", "p"), seg_req("bb", "q")];
        let chains = vec![0, 1];
        let provisional = vec![
            vec![(0.0, 100.0), (120.0, 100.0), (120.0, 40.0)],
            vec![(200.0, 200.0), (50.0, 200.0), (50.0, 20.0), (100.0, 20.0)],
        ];
        assert!(
            converge_groups(&reqs, &chains, &provisional).is_empty(),
            "no shared endpoint → not a convergence"
        );
    }

    #[test]
    fn adjacent_segments_of_one_chain_form_no_group() {
        // A chain `aa -> bb -> cc` is ONE wire; its two segments share the interior
        // node `bb` but are the wire passing through, not two bundles meeting there.
        let reqs = vec![seg_req("aa", "bb"), seg_req("bb", "cc")];
        let chains = vec![0, 0]; // both segments belong to the same chain wire
        let provisional = vec![
            vec![(0.0, 100.0), (120.0, 100.0), (120.0, 40.0)],
            vec![(200.0, 200.0), (50.0, 200.0), (50.0, 20.0), (100.0, 20.0)],
        ];
        assert!(
            converge_groups(&reqs, &chains, &provisional).is_empty(),
            "one chain's own segments are not a convergence"
        );
    }
}
