//! Wire routing — orchestration.
//!
//! Routes every wire to the contract in WIRING.md:
//! `select` (turn-aware side search) → `ports` (slots/order/C2–C5/C3, given sides)
//! → `route` (visibility-graph A\*) → `nudge` (global track assignment). A blind
//! provisional pass first gives `ports::PlanHint` lead points for crossing-free slot
//! order. The side search hill-climbs from the geometric seed, scoring candidates by
//! a route-only proxy (`score`); the winner is built and nudged, and kept only if it
//! betters the seed baseline — so the result is monotone and a deterministic function
//! of input. `validate` is the independent contract checker gating the whole thing.

mod geometry;
mod graph;
mod nudge;
mod oracle;
mod ports;
mod route;
mod scene;
mod score;
mod select;
mod text;
mod validate;

pub use validate::{validate_routing, Rule, Severity, Violation};

use crate::ast::Side;
use crate::error::Error;
use crate::layout::ir::{PlacedNode, RoutedWire};
use crate::resolve::{MarkerKind, Markers, Program, ResolvedEndpoint, ResolvedWire};
use geometry::{dumb_route, Pt, Rect};
use ports::{Plan, PlanHint, SegReq};
use scene::SceneIndex;
use score::score;
use text::place_texts;

/// Where one routed segment belongs in its source wire — so markers land on the
/// chain's outer ends and labels on its middle segment.
struct SegMeta<'a> {
    wire: &'a ResolvedWire,
    i: usize,
    segs: usize,
}

/// The flattened routing work: one segment-request per chain edge, in routing order,
/// each with its `SegMeta` (marker/label placement).
struct Reqs<'a> {
    reqs: Vec<SegReq>,
    metas: Vec<SegMeta<'a>>,
}

pub fn route_wires(program: &Program, nodes: &[PlacedNode]) -> Result<Vec<RoutedWire>, Error> {
    let index = SceneIndex::build(nodes);
    let fans = fan_ids(&program.wires);
    let Reqs { reqs, metas } = build_reqs(program, &index, &fans)?;

    // Provisional pass on the geometric seed — its routes give the C4 lead points
    // (where each wire heads) that order slots so they don't needlessly cross. The
    // SAME leads feed the search proxy and the final build, so the winner the search
    // picked is exactly what gets built (the nudge can only further improve it).
    let seed = select::seed_sides(&reqs);
    let leads = lead_hints(&reqs, &route_all(&reqs, &seed, &[], nodes));

    // Turn-aware side search, then build + nudge the winner. Keep it only if it
    // betters the seed baseline (both nudged), so the result is monotone — never
    // worse than the geometric pick — and a deterministic function of the input.
    let sides = select::search(&reqs, &seed, &leads, nodes);
    let chosen = finish(&reqs, &metas, &ports::assign(&reqs, &sides, &leads), nodes);
    let baseline = finish(&reqs, &metas, &ports::assign(&reqs, &seed, &leads), nodes);
    Ok(if score(&chosen, nodes) <= score(&baseline, nodes) {
        chosen
    } else {
        baseline
    })
}

/// Route every segment with the given sides + hints, **without** the nudge — the
/// provisional pass that feeds `lead_hints`.
fn route_all(
    reqs: &[SegReq],
    sides: &[(Side, Side)],
    hints: &[PlanHint],
    nodes: &[PlacedNode],
) -> Vec<Vec<Pt>> {
    let plans = ports::assign(reqs, sides, hints);
    reqs.iter()
        .zip(&plans)
        .map(|(req, plan)| route_one(req, plan, nodes))
        .collect()
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
    for w in &program.wires {
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
        }
    }
    Ok(Reqs { reqs, metas })
}

/// Build just the segment-requests for a scene — for `select`'s tests.
#[cfg(test)]
pub(super) fn build_reqs_for_test(program: &Program, nodes: &[PlacedNode]) -> Vec<SegReq> {
    let index = SceneIndex::build(nodes);
    let fans = fan_ids(&program.wires);
    build_reqs(program, &index, &fans).expect("reqs").reqs
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
    nudge::nudge(&mut out, nodes, true);
    out
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

/// Read each provisional route's real exit headings into `PlanHint` lead points —
/// where each end heads just past its stub — so `ports` orders shared slots by where
/// the wires actually go (crossing-free C4), not a straight-line guess.
fn lead_hints(reqs: &[SegReq], paths: &[Vec<Pt>]) -> Vec<PlanHint> {
    reqs.iter()
        .zip(paths)
        .map(|(req, path)| {
            if req.a_node == req.b_node {
                return PlanHint::default(); // a self-loop routes on its own
            }
            PlanHint {
                lead_a: lead_point(path, true),
                lead_b: lead_point(path, false),
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
