//! Wire routing.
//!
//! Phase 0: a deliberately dumb router — a perpendicular-attached orthogonal
//! route per wire (straight / L / Z), ignoring obstacles and other wires. Later
//! phases (see PLAN.md) replace the geometry with a visibility-graph + A* engine
//! and add port assignment and nudging; the validator (`validate`) gates each
//! step against the contract in WIRING.md.

mod geometry;
mod graph;
mod oracle;
mod ports;
mod route;
mod scene;
mod text;
mod validate;

pub use validate::{validate_routing, Rule, Severity, Violation};

use crate::error::Error;
use crate::layout::ir::{PlacedNode, RoutedWire};
use crate::resolve::{MarkerKind, Markers, Program, ResolvedEndpoint, ResolvedWire};
use geometry::{dumb_route, Rect};
use ports::SegReq;
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
    //    in declaration-then-chain order — the deterministic routing order.
    let mut reqs = Vec::new();
    let mut metas = Vec::new();
    for w in &program.wires {
        let eps = &w.endpoints;
        if eps.len() < 2 {
            continue; // resolve guarantees ≥ 2; be defensive
        }
        let clearance = oracle::clearance(&w.attrs);
        for i in 0..eps.len() - 1 {
            reqs.push(SegReq {
                a_node: eps[i].path.clone(),
                a: rect_for(&index, &eps[i])?,
                forced_a: eps[i].side,
                b_node: eps[i + 1].path.clone(),
                b: rect_for(&index, &eps[i + 1])?,
                forced_b: eps[i + 1].side,
                clearance,
            });
            metas.push(SegMeta {
                wire: w,
                i,
                segs: eps.len() - 1,
            });
        }
    }

    // 2. Plan every segment's ports together (sides + slots, WIRING C).
    let plans = ports::plan(&reqs);

    // 3. Route each segment, in order, past the obstacles for its own pair —
    //    falling back to the dumb route only if its ports are boxed in.
    let mut out = Vec::with_capacity(reqs.len());
    for (req, (plan, meta)) in reqs.iter().zip(plans.iter().zip(&metas)) {
        let obstacles = scene::obstacles_for(nodes, [&req.a_node, &req.b_node]);
        let path = route::route(
            plan.port_a,
            plan.side_a,
            plan.port_b,
            plan.side_b,
            &obstacles,
            req.clearance,
        )
        .unwrap_or_else(|| dumb_route(plan.port_a, plan.side_a, plan.port_b, plan.side_b));
        out.push(build_wire(meta, req, path));
    }
    Ok(out)
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
    }
}

fn rect_for(index: &SceneIndex, ep: &ResolvedEndpoint) -> Result<Rect, Error> {
    index.rect(&ep.path).ok_or_else(|| {
        Error::at(
            ep.span,
            format!("wire endpoint '{}' has no placed node", ep.path),
        )
    })
}
