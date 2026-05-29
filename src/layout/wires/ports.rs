//! Port selection — which side of each endpoint a wire leaves and enters.
//!
//! Phase 2 is minimal (WIRING C1, first clause): take the geometry-preferred
//! side unless the wire forces one (`a.r`), attaching at the side's midpoint.
//! Uniform slots and crossing-aware ordering for many wires on a side arrive in
//! Phase 3.

use super::geometry::{pick_edges, Rect};
use crate::ast::Side;

/// The (source, target) sides for a wire between two rects: the forced side if
/// the endpoint named one, else the side geometry prefers.
pub fn pick_sides(
    a: Rect,
    forced_a: Option<Side>,
    b: Rect,
    forced_b: Option<Side>,
) -> (Side, Side) {
    let (geo_a, geo_b) = pick_edges(a, b);
    (forced_a.unwrap_or(geo_a), forced_b.unwrap_or(geo_b))
}
