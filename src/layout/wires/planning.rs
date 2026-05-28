//! Plan a wire's resolved form into one `SegmentSpec` per routed link.
//!
//! A chain `a -> b -> c` becomes two specs (a→b, b→c). A fan-out
//! `a -> b & c` becomes two specs that share a `wire.span` (so endpoint
//! allocation can recognise them as siblings of one decl). A bundle of
//! parallel wires between the same pair is still N separate specs — they
//! get unified later by `stamping::group_bundles`.

use super::geometry::{nearest_edge, side_to_edge, AbsBbox, Edge};
use super::scene::SceneIndex;
use crate::error::Error;
use crate::layout::values::layout_var;
use crate::resolve::{Program, ResolvedValue, ResolvedWire, VarTable};
use crate::span::Span;

/// One link in a wire chain — the basic unit the router handles.
pub struct SegmentSpec<'a> {
    pub wire: &'a ResolvedWire,
    pub src_id: String,
    pub tgt_id: String,
    pub src_bbox: AbsBbox,
    pub tgt_bbox: AbsBbox,
    /// `.side` override on the source endpoint, if any — forces that edge.
    pub src_forced: Option<Edge>,
    /// `.side` override on the target endpoint, if any.
    pub tgt_forced: Option<Edge>,
    /// Geometry-picked default edge (used when `*_forced` is `None`).
    pub src_default_edge: Edge,
    pub tgt_default_edge: Edge,
    pub gap: f64,
    /// True iff this is the first segment in its chain — only the first
    /// segment carries the chain's start marker and any wire-text labels.
    pub is_first: bool,
    pub is_last: bool,
    /// First/last endpoint IDs of the chain, emitted as `data-from` /
    /// `data-to` on the rendered group.
    pub data_from: String,
    pub data_to: String,
}

pub fn plan_segments<'a>(
    program: &'a Program,
    scene: &SceneIndex,
) -> Result<Vec<SegmentSpec<'a>>, Error> {
    let mut out = Vec::new();
    for wire in &program.wires {
        let n = wire.endpoints.len();
        let from_id = wire.endpoints.first().unwrap().path.clone();
        let to_id = wire.endpoints.last().unwrap().path.clone();
        let gap = wire_gap(wire, &program.vars);
        for i in 0..(n - 1) {
            let src_id = wire.endpoints[i].path.clone();
            let tgt_id = wire.endpoints[i + 1].path.clone();
            if src_id == tgt_id {
                return Err(Error::at(
                    wire.span,
                    "self-loops are not yet routed (SPEC §10 self-loop is deferred)",
                ));
            }
            let src = scene
                .lookup(&src_id)
                .ok_or_else(|| undefined_wire_id(&src_id, wire.endpoints[i].span))?;
            let tgt = scene
                .lookup(&tgt_id)
                .ok_or_else(|| undefined_wire_id(&tgt_id, wire.endpoints[i + 1].span))?;
            let src_forced = wire.endpoints[i].side.map(side_to_edge);
            let tgt_forced = wire.endpoints[i + 1].side.map(side_to_edge);
            let src_default_edge = src_forced
                .unwrap_or_else(|| nearest_edge(&src.bbox, (tgt.bbox.cx(), tgt.bbox.cy())));
            let tgt_default_edge = tgt_forced
                .unwrap_or_else(|| nearest_edge(&tgt.bbox, (src.bbox.cx(), src.bbox.cy())));
            out.push(SegmentSpec {
                wire,
                src_id,
                tgt_id,
                src_bbox: src.bbox,
                tgt_bbox: tgt.bbox,
                src_forced,
                tgt_forced,
                src_default_edge,
                tgt_default_edge,
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

fn wire_gap(wire: &ResolvedWire, vars: &VarTable) -> f64 {
    if let Some(ResolvedValue::Number(n)) = wire.attrs.get("gap") {
        return *n;
    }
    layout_var(vars, "wire-gap").unwrap_or(16.0)
}

fn undefined_wire_id(id: &str, span: Span) -> Error {
    Error::at(span, format!("wire references undefined id '{}'", id))
}
