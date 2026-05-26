//! Wire emission — orthogonal path, optional markers, optional labels.

use super::markers::{emit_marker, marker_anchor};
use super::values::{attr_num, attr_or_var, escape_xml, num};
use crate::layout::{RoutedText, RoutedWire};
use crate::resolve::{MarkerKind, ResolvedValue, VarTable};
use crate::Options;
use std::fmt::Write;

pub fn render_wire(out: &mut String, w: &RoutedWire, vars: &VarTable, opts: &Options) {
    if w.path.len() < 2 {
        return;
    }
    let stroke = attr_or_var(&w.attrs, "stroke", "stroke", vars, opts);
    let thickness = attr_num(&w.attrs, "thickness").unwrap_or(1.0);
    let dash = wire_dash(&w.attrs);

    writeln!(
        out,
        r#"    <g class="plume-wire" data-from="{}" data-to="{}">"#,
        escape_xml(&w.data_from),
        escape_xml(&w.data_to),
    )
    .unwrap();

    // Shorten the path at each marker-bearing endpoint so the drawn line
    // STOPS where the marker tip will sit. Otherwise the stroke pokes out
    // past the arrowhead, which reads as a rendering bug. The visible gap
    // between the marker tip and the shape's edge (`MARKER_INSET`, 4 px)
    // gives the "almost touching" look the SPEC describes.
    let drawn = shorten_for_markers(&w.path, &w.markers);

    let mut d = format!("M {} {}", num(drawn[0].0), num(drawn[0].1));
    for p in &drawn[1..] {
        write!(d, " L {} {}", num(p.0), num(p.1)).unwrap();
    }
    let dash_attr = if dash.is_empty() {
        String::new()
    } else {
        format!(r#" stroke-dasharray="{}""#, dash)
    };
    writeln!(
        out,
        r#"      <path d="{}" fill="none" stroke="{}" stroke-width="{}"{}/>"#,
        d,
        stroke,
        num(thickness),
        dash_attr,
    )
    .unwrap();

    if w.markers.start != MarkerKind::None {
        if let Some((tip, dir)) = marker_anchor(w.path[1], w.path[0], false) {
            emit_marker(out, "      ", w.markers.start, tip, dir, &stroke, thickness);
        }
    }
    if w.markers.end != MarkerKind::None {
        let last = w.path[w.path.len() - 1];
        let prev = w.path[w.path.len() - 2];
        if let Some((tip, dir)) = marker_anchor(prev, last, false) {
            emit_marker(out, "      ", w.markers.end, tip, dir, &stroke, thickness);
        }
    }

    for t in &w.texts {
        render_wire_text(out, t, vars, opts);
    }

    out.push_str("    </g>\n");
}

/// Same value that `markers::marker_anchor` uses to inset the tip from the
/// raw endpoint. Keep these two in sync — the line should end exactly at
/// the marker tip so nothing visibly pokes past it.
const MARKER_INSET: f64 = 4.0;

fn shorten_for_markers(path: &[(f64, f64)], markers: &crate::resolve::Markers) -> Vec<(f64, f64)> {
    let mut p = path.to_vec();
    if p.len() < 2 {
        return p;
    }
    if markers.end != MarkerKind::None {
        let n = p.len();
        if let Some((nx, ny)) = pulled_back(p[n - 2], p[n - 1], MARKER_INSET) {
            p[n - 1] = (nx, ny);
        }
    }
    if markers.start != MarkerKind::None {
        if let Some((nx, ny)) = pulled_back(p[1], p[0], MARKER_INSET) {
            p[0] = (nx, ny);
        }
    }
    p
}

/// Move `endpoint` along the segment `inner → endpoint`, toward `inner`, by
/// `amount` pixels. Returns `None` if the segment is too short to absorb the
/// shift (in which case we'd rather leave the line untouched than collapse it).
fn pulled_back(inner: (f64, f64), endpoint: (f64, f64), amount: f64) -> Option<(f64, f64)> {
    let dx = endpoint.0 - inner.0;
    let dy = endpoint.1 - inner.1;
    let len = (dx * dx + dy * dy).sqrt();
    if len <= amount + 0.5 {
        return None;
    }
    let ux = dx / len;
    let uy = dy / len;
    Some((endpoint.0 - ux * amount, endpoint.1 - uy * amount))
}

fn wire_dash(attrs: &crate::resolve::AttrMap) -> String {
    for name in ["dashed", "dotted"] {
        if let Some(ResolvedValue::Tuple(items)) = attrs.get(name) {
            let parts: Vec<String> = items
                .iter()
                .filter_map(|v| match v {
                    ResolvedValue::Number(n) => Some(num(*n)),
                    _ => None,
                })
                .collect();
            if !parts.is_empty() {
                return parts.join(",");
            }
        }
    }
    String::new()
}

fn render_wire_text(out: &mut String, t: &RoutedText, vars: &VarTable, opts: &Options) {
    let size = attr_num(&t.attrs, "size").unwrap_or(11.0);
    let fill = attr_or_var(&t.attrs, "fill", "text-color", vars, opts);
    let font = attr_or_var(&t.attrs, "font", "font", vars, opts);
    // Background-coloured stroke painted UNDER the glyph fill. The wire path
    // visually disappears behind the label without us having to clip the path.
    // Halo width tracks font size so big labels get a proportional buffer.
    let halo = attr_or_var(&t.attrs, "halo", "bg", vars, opts);
    let halo_w = (size * 0.4).max(2.0);
    let (x, y) = t.position;
    writeln!(
        out,
        r#"      <text x="{}" y="{}" text-anchor="middle" dominant-baseline="central" font-size="{}" font-family={} fill="{}" paint-order="stroke" stroke="{}" stroke-width="{}" stroke-linejoin="round">{}</text>"#,
        num(x),
        num(y),
        num(size),
        wrap_font(&font),
        fill,
        halo,
        num(halo_w),
        escape_xml(&t.content),
    )
    .unwrap();
}

fn wrap_font(font: &str) -> String {
    if font.starts_with('"') {
        font.to_string()
    } else {
        format!("\"{}\"", font)
    }
}
