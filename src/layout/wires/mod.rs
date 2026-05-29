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
use scene::SceneIndex;
use text::place_texts;

pub fn route_wires(program: &Program, nodes: &[PlacedNode]) -> Result<Vec<RoutedWire>, Error> {
    let index = SceneIndex::build(nodes);
    let mut out = Vec::new();
    for w in &program.wires {
        route_chain(w, &index, nodes, &mut out)?;
    }
    Ok(out)
}

/// Split a chain `e0 -> e1 -> … -> eN` into one routed segment per pair, with
/// the wire's markers on the outer ends only and its labels on the middle
/// segment.
fn route_chain(
    w: &ResolvedWire,
    index: &SceneIndex,
    nodes: &[PlacedNode],
    out: &mut Vec<RoutedWire>,
) -> Result<(), Error> {
    let eps = &w.endpoints;
    if eps.len() < 2 {
        return Ok(()); // resolve guarantees ≥ 2; be defensive
    }
    let segs = eps.len() - 1;
    let mid = segs / 2;
    let chain_from = eps[0].path.clone();
    let chain_to = eps[segs].path.clone();
    let clearance = oracle::clearance(&w.attrs);

    for i in 0..segs {
        let a = rect_for(index, &eps[i])?;
        let b = rect_for(index, &eps[i + 1])?;
        let (sa, sb) = ports::pick_sides(a, eps[i].side, b, eps[i + 1].side);
        // Each chain segment is an independent wire; route it past the obstacles
        // for its own pair, falling back to the dumb route only if boxed in.
        let obstacles = scene::obstacles_for(nodes, [&eps[i].path, &eps[i + 1].path]);
        let path = route::route(a, sa, b, sb, &obstacles, clearance)
            .unwrap_or_else(|| dumb_route(a.port(sa), sa, b.port(sb), sb));

        out.push(RoutedWire {
            markers: Markers {
                start: if i == 0 {
                    w.markers.start
                } else {
                    MarkerKind::None
                },
                end: if i == segs - 1 {
                    w.markers.end
                } else {
                    MarkerKind::None
                },
            },
            texts: if i == mid {
                place_texts(&path, &w.texts)
            } else {
                Vec::new()
            },
            path,
            attrs: w.attrs.clone(),
            data_from: chain_from.clone(),
            data_to: chain_to.clone(),
            seg_from: eps[i].path.clone(),
            seg_to: eps[i + 1].path.clone(),
            decl_span: w.span,
        });
    }
    Ok(())
}

fn rect_for(index: &SceneIndex, ep: &ResolvedEndpoint) -> Result<Rect, Error> {
    index.rect(&ep.path).ok_or_else(|| {
        Error::at(
            ep.span,
            format!("wire endpoint '{}' has no placed node", ep.path),
        )
    })
}
