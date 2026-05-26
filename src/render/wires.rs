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

    let mut d = format!("M {} {}", num(w.path[0].0), num(w.path[0].1));
    for p in &w.path[1..] {
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
