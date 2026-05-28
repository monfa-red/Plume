//! The single authority for wire clearance distances.
//!
//! Every phase that needs "how far must a wire stay from this shape?" or
//! "how far apart must these two wires sit?" calls these functions — never
//! its own inline math. Per the rules-spec, shape clearance is the shape's
//! parent-container gap (already computed per-node by `SceneIndex`), and wire
//! separation is the larger of the two wires' `gap` attrs.

use super::scene::SceneIndex;
use crate::layout::ir::RoutedWire;
use crate::layout::values::layout_var;
use crate::resolve::{ResolvedValue, VarTable};

/// Minimum distance a wire must keep from obstacle `shape` — the gap of the
/// shape's parent container (scene gap for a top-level shape). `0.0` if the
/// shape is unknown.
pub fn shape_clearance(scene: &SceneIndex, shape: &str) -> f64 {
    scene.clearance(shape).unwrap_or(0.0)
}

/// Minimum distance two wires must keep from each other — the larger of their
/// gaps, so the more generous wire wins.
pub fn wire_separation(gap_a: f64, gap_b: f64) -> f64 {
    gap_a.max(gap_b)
}

/// The wire's own gap: its `gap` attr, else the `--plume-wire-gap` layout
/// default (16). Mirrors `planning::wire_gap` so the validator measures wires
/// the same way the router spaced them.
pub fn wire_gap(wire: &RoutedWire, vars: &VarTable) -> f64 {
    if let Some(ResolvedValue::Number(n)) = wire.attrs.get("gap") {
        return *n;
    }
    layout_var(vars, "wire-gap").unwrap_or(16.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn separation_is_the_larger_gap() {
        assert_eq!(wire_separation(8.0, 16.0), 16.0);
        assert_eq!(wire_separation(20.0, 5.0), 20.0);
    }
}
