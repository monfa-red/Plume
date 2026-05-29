//! Wire routing.
//!
//! Phase 0: a deliberately dumb router — a perpendicular-attached orthogonal
//! route per wire (straight / L / Z), ignoring obstacles and other wires. Later
//! phases (see PLAN.md) replace the geometry with a visibility-graph + A* engine
//! and add port assignment and nudging; the validator (`validate`) gates each
//! step against the contract in WIRING.md.

mod geometry;
mod graph;
mod nudge;
mod oracle;
mod ports;
mod route;
mod scene;
mod text;
mod validate;

pub use validate::{validate_routing, Rule, Severity, Violation};

use crate::ast::Side;
use crate::error::Error;
use crate::layout::ir::{PlacedNode, RoutedWire};
use crate::resolve::{MarkerKind, Markers, Program, ResolvedEndpoint, ResolvedWire};
use geometry::{dumb_route, rect_penetrated_by, seg_rect_distance, Pt, Rect, EPS};
use ports::{Plan, PlanHint, SegReq};
use scene::SceneIndex;
use text::place_texts;

/// Where one routed segment belongs in its source wire — so markers land on the
/// chain's outer ends and labels on its middle segment.
struct SegMeta<'a> {
    wire: &'a ResolvedWire,
    i: usize,
    segs: usize,
}

pub fn route_wires(program: &Program, nodes: &[PlacedNode]) -> Result<Vec<RoutedWire>, Error> {
    let index = SceneIndex::build(nodes);

    // 1. Flatten every chain `e0 -> … -> eN` into one segment-request per pair,
    //    in declaration-then-chain order — the deterministic routing order. A
    //    fan group's shared end carries a trunk id (E2): siblings collapse to one
    //    slot there and the validator exempts their coincident run.
    let fans = fan_ids(&program.wires);
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
                a: rect_for(&index, &eps[i])?,
                forced_a: eps[i].side,
                b_node: eps[i + 1].path.clone(),
                b: rect_for(&index, &eps[i + 1])?,
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

    // 2. Plan + route in two passes (the libavoid model). Side and slot choice run
    //    blind on the first pass (straight-line guesses); the provisional routes
    //    then say where each wire *actually* went, so the second pass re-elects a
    //    side that forced an endpoint skim and orders slots by real exit heading —
    //    removing avoidable crossings and skims at the source.
    let plans = ports::plan(&reqs, &[]);
    let provisional: Vec<Vec<(f64, f64)>> = reqs
        .iter()
        .zip(&plans)
        .map(|(req, plan)| route_one(req, plan, nodes))
        .collect();
    let hints = derive_hints(&reqs, &provisional);
    let plans = ports::plan(&reqs, &hints);

    // 3. Final route with the informed plan, building each RoutedWire.
    let mut out = Vec::with_capacity(reqs.len());
    for (req, (plan, meta)) in reqs.iter().zip(plans.iter().zip(&metas)) {
        out.push(build_wire(meta, req, route_one(req, plan, nodes)));
    }

    // 4. Global nudge: spread shared/near-parallel runs onto clean tracks,
    //    committing only the separations that keep wires clear of nodes.
    nudge::nudge(&mut out, nodes);
    Ok(out)
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
