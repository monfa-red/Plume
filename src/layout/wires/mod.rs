//! Wire routing — orthogonal visibility-grid + A\*.
//!
//! Pipeline:
//!
//! 1. **Plan** the resolved wires into one `SegmentSpec` per routed link
//!    (chains explode; fan-out specs share a `wire.span`).
//! 2. **Allocate endpoints**: each `(shape, edge)` bin distributes its wires
//!    into evenly-spaced lanes around the edge midpoint.
//! 3. **Build** one orthogonal visibility grid from every shape's clearance
//!    edges, the world frame, and every endpoint coordinate.
//! 4. **Route** each wire with A\* (fewest bends, shape-clearance hard),
//!    then assemble a `RoutedWire` (markers, texts, provenance).
//!
//! The contract these paths must satisfy lives in `validate.rs` (rules R1–R6);
//! see `docs/superpowers/specs/2026-05-28-wire-routing-rules-design.md`.

mod astar;
mod endpoints;
mod geometry;
mod grid;
mod oracle;
mod planning;
mod route_graph;
mod scene;
mod text;
mod validate;

use crate::error::Error;
use crate::layout::ir::{PlacedNode, RoutedWire};
use crate::resolve::{MarkerKind, Markers, Program};

use planning::{plan_segments, SegmentSpec};
use scene::SceneIndex;
use text::place_texts;

pub use validate::{validate_routing, Rule, Severity, Violation};

pub fn route_wires(
    program: &Program,
    scene_nodes: &[PlacedNode],
) -> Result<Vec<RoutedWire>, Error> {
    let scene = SceneIndex::build(scene_nodes, &program.scene.attrs);
    let specs = plan_segments(program, &scene)?;

    // Pad the world by the largest gap so perimeter detours have room outside
    // every shape.
    let max_gap = specs.iter().map(|s| s.gap).fold(0.0_f64, f64::max).max(8.0);
    let world = scene.bounds(max_gap);

    let paths = route_graph::route_all(&specs, &scene, world);
    Ok(specs
        .iter()
        .zip(paths)
        .map(|(spec, path)| build_routed_wire(spec, path))
        .collect())
}

fn build_routed_wire(spec: &SegmentSpec, path: Vec<(f64, f64)>) -> RoutedWire {
    RoutedWire {
        markers: Markers {
            start: if spec.is_first {
                spec.wire.markers.start
            } else {
                MarkerKind::None
            },
            end: if spec.is_last {
                spec.wire.markers.end
            } else {
                MarkerKind::None
            },
        },
        attrs: spec.wire.attrs.clone(),
        texts: if spec.is_first {
            place_texts(&spec.wire.texts, &path)
        } else {
            Vec::new()
        },
        data_from: spec.data_from.clone(),
        data_to: spec.data_to.clone(),
        seg_from: spec.src_id.clone(),
        seg_to: spec.tgt_id.clone(),
        decl_span: spec.wire.span,
        path,
    }
}
