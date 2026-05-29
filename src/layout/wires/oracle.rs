//! The distance oracle — the one place a wire's clearance is read and combined.
//!
//! `clearance` is a per-wire property (WIRING §Definitions): the minimum gap a
//! wire keeps both from obstacle nodes and from other wires — one value covers
//! both. `separation` is how far two wires stay from each other.

use crate::resolve::{AttrMap, ResolvedValue};

/// Language default when a wire sets no `clearance` (WIRING: default 16; mirrors
/// the `clearance` layout constant seeded in `resolve::vars`).
pub const DEFAULT_CLEARANCE: f64 = 16.0;

/// The clearance a wire keeps, in px — its resolved `clearance` attr, or
/// [`DEFAULT_CLEARANCE`] when unset. Clearance is a baked layout value, never a
/// themeable `var()`, so it always resolves to a plain number.
pub fn clearance(attrs: &AttrMap) -> f64 {
    match attrs.get("clearance") {
        Some(ResolvedValue::Number(n)) => *n,
        _ => DEFAULT_CLEARANCE,
    }
}

/// The gap two wires keep from each other: `max(clearance1, clearance2)`
/// (WIRING §Definitions, `separation(w1, w2)`).
pub fn separation(a: &AttrMap, b: &AttrMap) -> f64 {
    clearance(a).max(clearance(b))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wire_attrs(clearance: Option<f64>) -> AttrMap {
        let mut a = AttrMap::new();
        if let Some(c) = clearance {
            a.insert("clearance", ResolvedValue::Number(c));
        }
        a
    }

    #[test]
    fn clearance_defaults_to_16_when_unset() {
        assert_eq!(clearance(&wire_attrs(None)), 16.0);
    }

    #[test]
    fn clearance_reads_the_wire_attr() {
        assert_eq!(clearance(&wire_attrs(Some(8.0))), 8.0);
    }

    #[test]
    fn separation_takes_the_larger_clearance() {
        assert_eq!(
            separation(&wire_attrs(Some(8.0)), &wire_attrs(Some(20.0))),
            20.0
        );
        assert_eq!(separation(&wire_attrs(None), &wire_attrs(Some(4.0))), 16.0);
    }
}
