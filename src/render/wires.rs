//! Wire emission — the orthogonal path, optional markers, optional labels.

use super::markers::{emit_marker, line_inset, marker_anchor};
use super::values::{attr_num, attr_or_var, escape_xml, format_value, num};
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

    // Stop the drawn line where the marker body will sit so the stroke never
    // pokes past it (and never leaves a gap before a dot).
    let drawn = shorten_for_markers(&w.path, &w.markers, thickness);
    let mut d = format!("M {} {}", num(drawn[0].0), num(drawn[0].1));
    for p in &drawn[1..] {
        write!(d, " L {} {}", num(p.0), num(p.1)).unwrap();
    }
    let dash_attr = if dash.is_empty() {
        String::new()
    } else {
        format!(r#" stroke-dasharray="{dash}""#)
    };
    writeln!(
        out,
        r#"      <path d="{d}" fill="none" stroke="{stroke}" stroke-width="{}"{dash_attr}/>"#,
        num(thickness),
    )
    .unwrap();

    if w.markers.start != MarkerKind::None {
        if let Some((tip, dir)) = marker_anchor(w.path[1], w.path[0], false) {
            emit_marker(out, "      ", w.markers.start, tip, dir, &stroke, thickness);
        }
    }
    if w.markers.end != MarkerKind::None {
        let n = w.path.len();
        if let Some((tip, dir)) = marker_anchor(w.path[n - 2], w.path[n - 1], false) {
            emit_marker(out, "      ", w.markers.end, tip, dir, &stroke, thickness);
        }
    }

    for t in &w.texts {
        render_wire_text(out, t, vars, opts);
    }

    out.push_str("    </g>\n");
}

/// Pull each marker-bearing endpoint back along its segment so the line stops where
/// that marker's body begins (per-marker, [`line_inset`]).
fn shorten_for_markers(
    path: &[(f64, f64)],
    markers: &crate::resolve::Markers,
    thickness: f64,
) -> Vec<(f64, f64)> {
    let mut p = path.to_vec();
    if p.len() < 2 {
        return p;
    }
    if markers.end != MarkerKind::None {
        let n = p.len();
        if let Some(q) = pulled_back(p[n - 2], p[n - 1], line_inset(markers.end, thickness)) {
            p[n - 1] = q;
        }
    }
    if markers.start != MarkerKind::None {
        if let Some(q) = pulled_back(p[1], p[0], line_inset(markers.start, thickness)) {
            p[0] = q;
        }
    }
    p
}

/// Move `endpoint` toward `inner` by `amount`. `None` if the segment is too
/// short to absorb the shift.
fn pulled_back(inner: (f64, f64), endpoint: (f64, f64), amount: f64) -> Option<(f64, f64)> {
    let (dx, dy) = (endpoint.0 - inner.0, endpoint.1 - inner.1);
    let len = (dx * dx + dy * dy).sqrt();
    if len <= amount + 0.5 {
        return None;
    }
    Some((
        endpoint.0 - dx / len * amount,
        endpoint.1 - dy / len * amount,
    ))
}

/// `stroke-style:dashed|dotted` → a dash pattern sized against thickness, in
/// step with the primitive renderer.
fn wire_dash(attrs: &crate::resolve::AttrMap) -> String {
    let thickness = attr_num(attrs, "thickness").unwrap_or(1.0);
    match attrs.get("stroke-style") {
        Some(ResolvedValue::Ident(s)) => match s.as_str() {
            "dashed" => format!("{},{}", num(thickness * 4.0), num(thickness * 4.0)),
            "dotted" => format!("{},{}", num(thickness), num(thickness * 3.0)),
            _ => String::new(),
        },
        _ => String::new(),
    }
}

fn render_wire_text(out: &mut String, t: &RoutedText, vars: &VarTable, opts: &Options) {
    let size = attr_num(&t.attrs, "size").unwrap_or(11.0);
    let fill = if let Some(v) = t.attrs.get("fill") {
        format_value(v, vars, opts)
    } else if let Some(v) = t.attrs.get("color") {
        format_value(v, vars, opts)
    } else {
        "currentColor".to_string()
    };
    let font = attr_or_var(&t.attrs, "font", "font", vars, opts);
    // Background-coloured halo painted under the glyphs so the wire reads as
    // passing behind the label without clipping the path.
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
        format!("\"{font}\"")
    }
}
