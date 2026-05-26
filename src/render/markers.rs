//! Marker geometry (arrow / dot / diamond / crow). Shared between inline
//! `:line` / `:arrow` primitives and wire rendering.

use super::values::num;
use crate::layout::PlacedNode;
use crate::resolve::{MarkerKind, Markers};
use std::fmt::Write;

/// Markers for inline `:line` / `:arrow` primitives. Source-order marker
/// resolution already lives in `resolve`; here we just emit shapes.
#[allow(clippy::too_many_arguments)]
pub fn emit_inline_markers(
    out: &mut String,
    indent: &str,
    n: &PlacedNode,
    from: (f64, f64),
    to: (f64, f64),
    stroke: &str,
    thickness: f64,
    arrow_default: bool,
) {
    let m = if n.markers.start == MarkerKind::None && n.markers.end == MarkerKind::None {
        // Fall back to the `:line` (none) / `:arrow` (end=arrow) defaults if
        // resolve produced an empty Markers struct.
        if arrow_default {
            Markers {
                start: MarkerKind::None,
                end: MarkerKind::Arrow,
            }
        } else {
            return;
        }
    } else {
        n.markers.clone()
    };

    if m.start != MarkerKind::None {
        if let Some((tip, dir)) = marker_anchor(from, to, true) {
            emit_marker(out, indent, m.start, tip, dir, stroke, thickness);
        }
    }
    if m.end != MarkerKind::None {
        if let Some((tip, dir)) = marker_anchor(from, to, false) {
            emit_marker(out, indent, m.end, tip, dir, stroke, thickness);
        }
    }
}

/// Position the marker tip inset 4 px from the line endpoint, with the
/// direction unit-vector pointing outward.
pub fn marker_anchor(
    from: (f64, f64),
    to: (f64, f64),
    at_start: bool,
) -> Option<((f64, f64), (f64, f64))> {
    let (anchor, neighbor) = if at_start { (from, to) } else { (to, from) };
    let dx = anchor.0 - neighbor.0;
    let dy = anchor.1 - neighbor.1;
    let len = (dx * dx + dy * dy).sqrt();
    if len < 1e-9 {
        return Some((anchor, (1.0, 0.0)));
    }
    let ux = dx / len;
    let uy = dy / len;
    // Visible gap between the marker tip and the shape edge. Kept small —
    // the user's intuition is that the tip should "almost touch" the shape.
    // The line stop point (in render/wires.rs `shorten_for_markers`) sits
    // further back along the segment so the marker body still covers it.
    let inset = 1.0_f64.min(len * 0.5);
    let tip = (anchor.0 - ux * inset, anchor.1 - uy * inset);
    Some((tip, (ux, uy)))
}

pub fn emit_marker(
    out: &mut String,
    indent: &str,
    kind: MarkerKind,
    tip: (f64, f64),
    direction: (f64, f64),
    stroke: &str,
    thickness: f64,
) {
    let size = 10.0_f64.max(thickness * 5.0);
    let ux = direction.0;
    let uy = direction.1;
    let px = -uy;
    let py = ux;
    match kind {
        MarkerKind::Arrow => {
            let bx = tip.0 - ux * size;
            let by = tip.1 - uy * size;
            let lx = bx + px * size * 0.5;
            let ly = by + py * size * 0.5;
            let rx = bx - px * size * 0.5;
            let ry = by - py * size * 0.5;
            writeln!(
                out,
                r#"{}<polygon class="plume-marker plume-marker-arrow" points="{},{} {},{} {},{}" fill="{}"/>"#,
                indent,
                num(tip.0), num(tip.1),
                num(lx), num(ly),
                num(rx), num(ry),
                stroke,
            ).unwrap();
        }
        MarkerKind::Dot => {
            writeln!(
                out,
                r#"{}<circle class="plume-marker plume-marker-dot" cx="{}" cy="{}" r="{}" fill="{}"/>"#,
                indent,
                num(tip.0),
                num(tip.1),
                num(size / 3.0),
                stroke,
            )
            .unwrap();
        }
        MarkerKind::Diamond => {
            let bx = tip.0 - ux * size;
            let by = tip.1 - uy * size;
            let mx = (tip.0 + bx) / 2.0;
            let my = (tip.1 + by) / 2.0;
            let lx = mx + px * size * 0.4;
            let ly = my + py * size * 0.4;
            let rx = mx - px * size * 0.4;
            let ry = my - py * size * 0.4;
            writeln!(
                out,
                r#"{}<polygon class="plume-marker plume-marker-diamond" points="{},{} {},{} {},{} {},{}" fill="{}"/>"#,
                indent,
                num(tip.0), num(tip.1),
                num(lx), num(ly),
                num(bx), num(by),
                num(rx), num(ry),
                stroke,
            ).unwrap();
        }
        MarkerKind::Crow => {
            let bx = tip.0 - ux * size;
            let by = tip.1 - uy * size;
            let lx = bx + px * size * 0.5;
            let ly = by + py * size * 0.5;
            let rx = bx - px * size * 0.5;
            let ry = by - py * size * 0.5;
            writeln!(
                out,
                r#"{}<path class="plume-marker plume-marker-crow" d="M {} {} L {} {} M {} {} L {} {} M {} {} L {} {}" stroke="{}" stroke-width="{}" fill="none"/>"#,
                indent,
                num(tip.0), num(tip.1), num(bx), num(by),
                num(tip.0), num(tip.1), num(lx), num(ly),
                num(tip.0), num(tip.1), num(rx), num(ry),
                stroke, num(thickness),
            ).unwrap();
        }
        MarkerKind::None => {}
    }
}
