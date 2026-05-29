//! Per-primitive SVG geometry. One emitter per `ShapeKind`; most produce a
//! single SVG element, `cyl` and `cloud` build a small composition.

use super::values::{attr_num, attr_or_var, attr_points, attr_str, escape_xml, num};
use crate::layout::PlacedNode;
use crate::resolve::{ShapeKind, VarTable};
use crate::Options;
use std::fmt::Write;

pub fn render_geometry(
    out: &mut String,
    n: &PlacedNode,
    depth: usize,
    vars: &VarTable,
    opts: &Options,
) {
    let indent = "  ".repeat(depth);
    let stroke = attr_or_var(&n.attrs, "stroke", "stroke", vars, opts);
    let fill = attr_or_var(&n.attrs, "fill", "fill", vars, opts);
    let thickness = attr_num(&n.attrs, "thickness").unwrap_or(1.0);

    match n.shape {
        ShapeKind::Rect => emit_rect(out, n, &indent, &fill, &stroke, thickness),
        ShapeKind::Slant => emit_slant(out, n, &indent, &fill, &stroke, thickness),
        ShapeKind::Hex => emit_hex(out, n, &indent, &fill, &stroke, thickness),
        ShapeKind::Diamond => emit_diamond(out, n, &indent, &fill, &stroke, thickness),
        ShapeKind::Cyl => emit_cyl(out, n, &indent, &fill, &stroke, thickness),
        ShapeKind::Cloud => emit_cloud(out, n, &indent, &fill, &stroke, thickness),
        ShapeKind::Oval => emit_oval(out, n, &indent, &fill, &stroke, thickness),
        ShapeKind::Text => emit_text(out, n, &indent, vars, opts),
        ShapeKind::Line => emit_line(out, n, &indent, &stroke, thickness),
        ShapeKind::Poly => emit_poly(out, n, &indent, &fill, &stroke, thickness),
        ShapeKind::Path => emit_path(out, n, &indent, &fill, &stroke, thickness),
        ShapeKind::Icon => emit_icon(out, n, &indent, &stroke, vars, opts),
        ShapeKind::Image => emit_image(out, n, &indent),
    }
}

fn dim_excluding_stroke(n: &PlacedNode, thickness: f64) -> (f64, f64) {
    let w = (n.bbox.w() - thickness).max(0.0);
    let h = (n.bbox.h() - thickness).max(0.0);
    (w, h)
}

fn emit_rect(
    out: &mut String,
    n: &PlacedNode,
    indent: &str,
    fill: &str,
    stroke: &str,
    thickness: f64,
) {
    let (w, h) = dim_excluding_stroke(n, thickness);
    let radius = attr_num(&n.attrs, "radius").unwrap_or(0.0);
    let dash = stroke_dasharray(n, thickness);
    writeln!(
        out,
        r#"{}<rect x="{}" y="{}" width="{}" height="{}" rx="{}" ry="{}" fill="{}" stroke="{}" stroke-width="{}"{}/>"#,
        indent,
        num(-w / 2.0),
        num(-h / 2.0),
        num(w),
        num(h),
        num(radius),
        num(radius),
        fill,
        stroke,
        num(thickness),
        dash,
    )
    .unwrap();
}

fn emit_oval(
    out: &mut String,
    n: &PlacedNode,
    indent: &str,
    fill: &str,
    stroke: &str,
    thickness: f64,
) {
    let (w, h) = dim_excluding_stroke(n, thickness);
    writeln!(
        out,
        r#"{}<ellipse cx="0" cy="0" rx="{}" ry="{}" fill="{}" stroke="{}" stroke-width="{}"/>"#,
        indent,
        num(w / 2.0),
        num(h / 2.0),
        fill,
        stroke,
        num(thickness),
    )
    .unwrap();
}

fn emit_hex(
    out: &mut String,
    n: &PlacedNode,
    indent: &str,
    fill: &str,
    stroke: &str,
    thickness: f64,
) {
    let (w, h) = dim_excluding_stroke(n, thickness);
    // Flat-top hex per SPEC section 7. Two horizontal edges, four slanted edges.
    let pts = [
        (-w / 2.0, 0.0),
        (-w / 4.0, -h / 2.0),
        (w / 4.0, -h / 2.0),
        (w / 2.0, 0.0),
        (w / 4.0, h / 2.0),
        (-w / 4.0, h / 2.0),
    ];
    emit_polygon(out, indent, &pts, fill, stroke, thickness);
}

fn emit_diamond(
    out: &mut String,
    n: &PlacedNode,
    indent: &str,
    fill: &str,
    stroke: &str,
    thickness: f64,
) {
    let (w, h) = dim_excluding_stroke(n, thickness);
    let pts = [
        (0.0, -h / 2.0),
        (w / 2.0, 0.0),
        (0.0, h / 2.0),
        (-w / 2.0, 0.0),
    ];
    emit_polygon(out, indent, &pts, fill, stroke, thickness);
}

fn emit_slant(
    out: &mut String,
    n: &PlacedNode,
    indent: &str,
    fill: &str,
    stroke: &str,
    thickness: f64,
) {
    let (w, h) = dim_excluding_stroke(n, thickness);
    let skew_deg = attr_num(&n.attrs, "skew").unwrap_or(15.0);
    let shift = (skew_deg.to_radians()).tan() * h / 2.0;
    let pts = [
        (-w / 2.0 + shift, -h / 2.0),
        (w / 2.0 + shift, -h / 2.0),
        (w / 2.0 - shift, h / 2.0),
        (-w / 2.0 - shift, h / 2.0),
    ];
    emit_polygon(out, indent, &pts, fill, stroke, thickness);
}

fn emit_cyl(
    out: &mut String,
    n: &PlacedNode,
    indent: &str,
    fill: &str,
    stroke: &str,
    thickness: f64,
) {
    // Cylinder = ellipse top + body rect + ellipse bottom. The bbox carries
    // total height; the ellipse rx == w/2, ry ≈ h/10.
    let (w, h) = dim_excluding_stroke(n, thickness);
    let rx = w / 2.0;
    let ry = (h / 10.0).max(2.0);
    let top_cy = -h / 2.0 + ry;
    let bottom_cy = h / 2.0 - ry;
    // Body as a path that draws sides + bottom arc (fill the cylinder).
    writeln!(
        out,
        r#"{}<path d="M {} {} L {} {} A {} {} 0 0 0 {} {} L {} {} A {} {} 0 0 0 {} {} Z" fill="{}" stroke="{}" stroke-width="{}"/>"#,
        indent,
        num(-rx), num(top_cy),
        num(-rx), num(bottom_cy),
        num(rx), num(ry), num(rx), num(bottom_cy),
        num(rx), num(top_cy),
        num(rx), num(ry), num(-rx), num(top_cy),
        fill, stroke, num(thickness),
    ).unwrap();
    // Top ellipse rim (visible curve on top).
    writeln!(
        out,
        r#"{}<ellipse cx="0" cy="{}" rx="{}" ry="{}" fill="{}" stroke="{}" stroke-width="{}"/>"#,
        indent,
        num(top_cy),
        num(rx),
        num(ry),
        fill,
        stroke,
        num(thickness),
    )
    .unwrap();
}

fn emit_cloud(
    out: &mut String,
    n: &PlacedNode,
    indent: &str,
    fill: &str,
    stroke: &str,
    thickness: f64,
) {
    // Stylized cloud. Reference path is sized for 100 × 60; scale to bbox.
    let (w, h) = dim_excluding_stroke(n, thickness);
    let sx = w / 100.0;
    let sy = h / 60.0;
    let pt = |x: f64, y: f64| (x * sx - w / 2.0, y * sy - h / 2.0);
    let d = format!(
        "M {a} Q {b} {c} Q {d} {e} Q {f} {g} Q {h} {i} Q {j} {k} Q {l} {m} Z",
        a = fmt_pt(pt(25.0, 60.0)),
        b = fmt_pt(pt(5.0, 60.0)),
        c = fmt_pt(pt(5.0, 40.0)),
        d = fmt_pt(pt(5.0, 20.0)),
        e = fmt_pt(pt(25.0, 25.0)),
        f = fmt_pt(pt(30.0, 5.0)),
        g = fmt_pt(pt(55.0, 15.0)),
        h = fmt_pt(pt(75.0, 5.0)),
        i = fmt_pt(pt(75.0, 25.0)),
        j = fmt_pt(pt(95.0, 30.0)),
        k = fmt_pt(pt(95.0, 50.0)),
        l = fmt_pt(pt(95.0, 70.0)),
        m = fmt_pt(pt(75.0, 60.0)),
    );
    writeln!(
        out,
        r#"{}<path d="{}" fill="{}" stroke="{}" stroke-width="{}"/>"#,
        indent,
        d,
        fill,
        stroke,
        num(thickness),
    )
    .unwrap();
}

fn emit_text(out: &mut String, n: &PlacedNode, indent: &str, vars: &VarTable, opts: &Options) {
    let size = attr_num(&n.attrs, "size").unwrap_or(13.0);
    let label = n.label.as_deref().unwrap_or("");
    // On |text|, `color` is an alias for `fill` (CSS-style). If neither is set,
    // fall back to `currentColor` so SVG inheritance from any ancestor `color`
    // takes over — the root scene seeds this with `--plume-text-color`.
    let fill = if let Some(v) = n.attrs.get("fill") {
        crate::render::values::format_value(v, vars, opts)
    } else if let Some(v) = n.attrs.get("color") {
        crate::render::values::format_value(v, vars, opts)
    } else {
        "currentColor".to_string()
    };
    let font = attr_or_var(&n.attrs, "font", "font", vars, opts);
    let weight = attr_str(&n.attrs, "weight", "normal", vars, opts);
    let weight_attr = if weight != "normal" {
        format!(r#" font-weight="{}""#, weight)
    } else {
        String::new()
    };
    writeln!(
        out,
        r#"{}<text x="0" y="0" text-anchor="middle" dominant-baseline="central" font-size="{}" font-family={} fill="{}"{}>{}</text>"#,
        indent,
        num(size),
        wrap_font(&font),
        fill,
        weight_attr,
        escape_xml(label),
    )
    .unwrap();
}

fn wrap_font(font: &str) -> String {
    // String values come back already quoted ("..."); other forms (var(...),
    // raw idents) need their own quote wrapper for SVG attribute syntax.
    if font.starts_with('"') {
        font.to_string()
    } else {
        format!("\"{}\"", font)
    }
}

fn emit_line(out: &mut String, n: &PlacedNode, indent: &str, stroke: &str, thickness: f64) {
    let points = attr_points(&n.attrs, "points").unwrap_or_default();
    if points.len() < 2 {
        return;
    }

    // 2 points → SVG <line>; 3+ → SVG <polyline> with fill=none.
    let dash = stroke_dasharray(n, thickness);
    if points.len() == 2 {
        let (from, to) = (points[0], points[1]);
        writeln!(
            out,
            r#"{}<line x1="{}" y1="{}" x2="{}" y2="{}" stroke="{}" stroke-width="{}"{}/>"#,
            indent,
            num(from.0),
            num(from.1),
            num(to.0),
            num(to.1),
            stroke,
            num(thickness),
            dash,
        )
        .unwrap();
    } else {
        let pts: Vec<String> = points
            .iter()
            .map(|(x, y)| format!("{},{}", num(*x), num(*y)))
            .collect();
        writeln!(
            out,
            r#"{}<polyline points="{}" fill="none" stroke="{}" stroke-width="{}"{}/>"#,
            indent,
            pts.join(" "),
            stroke,
            num(thickness),
            dash,
        )
        .unwrap();
    }

    // Markers go at the first and last points.
    let from = points[0];
    let to = points[points.len() - 1];
    super::markers::emit_inline_markers(out, indent, n, from, to, stroke, thickness);
}

/// Emit `stroke-dasharray="..."` for `stroke-style=dashed|dotted`, else empty.
fn stroke_dasharray(n: &PlacedNode, thickness: f64) -> String {
    match n.attrs.get("stroke-style") {
        Some(crate::resolve::ResolvedValue::Ident(s)) => match s.as_str() {
            "dashed" => format!(
                r#" stroke-dasharray="{},{}""#,
                num(thickness * 4.0),
                num(thickness * 4.0)
            ),
            "dotted" => format!(
                r#" stroke-dasharray="{},{}""#,
                num(thickness),
                num(thickness * 3.0)
            ),
            _ => String::new(),
        },
        _ => String::new(),
    }
}

fn emit_poly(
    out: &mut String,
    n: &PlacedNode,
    indent: &str,
    fill: &str,
    stroke: &str,
    thickness: f64,
) {
    let points = attr_points(&n.attrs, "points").unwrap_or_default();
    emit_polygon(out, indent, &points, fill, stroke, thickness);
}

fn emit_path(
    out: &mut String,
    n: &PlacedNode,
    indent: &str,
    fill: &str,
    stroke: &str,
    thickness: f64,
) {
    let d = match n.attrs.get("d") {
        Some(crate::resolve::ResolvedValue::String(s)) => s.clone(),
        _ => return,
    };
    writeln!(
        out,
        r#"{}<path d="{}" fill="{}" stroke="{}" stroke-width="{}"/>"#,
        indent,
        escape_xml(&d),
        fill,
        stroke,
        num(thickness),
    )
    .unwrap();
}

fn emit_icon(
    out: &mut String,
    n: &PlacedNode,
    indent: &str,
    stroke: &str,
    vars: &VarTable,
    opts: &Options,
) {
    // Material Symbols embedding lands in a follow-up; for Sprint 5 we render
    // a placeholder square so layout is visible and the icon's name is
    // discoverable through the SVG.
    let size = attr_num(&n.attrs, "size").unwrap_or(24.0);
    let name = attr_str(&n.attrs, "name", "?", vars, opts);
    let fill = attr_or_var(&n.attrs, "fill", "stroke", vars, opts);
    writeln!(
        out,
        r#"{}<rect x="{}" y="{}" width="{}" height="{}" fill="none" stroke="{}" stroke-width="1"/>"#,
        indent,
        num(-size / 2.0),
        num(-size / 2.0),
        num(size),
        num(size),
        stroke,
    )
    .unwrap();
    writeln!(
        out,
        r#"{}<text x="0" y="0" text-anchor="middle" dominant-baseline="central" font-size="{}" fill="{}">{}</text>"#,
        indent,
        num(size * 0.4),
        fill,
        escape_xml(&name),
    )
    .unwrap();
}

fn emit_image(out: &mut String, n: &PlacedNode, indent: &str) {
    let href = match n.attrs.get("href") {
        Some(crate::resolve::ResolvedValue::String(s)) => s.clone(),
        _ => return,
    };
    // Image dimensions come from its bbox (driven by `size=`).
    let w = n.bbox.w();
    let h = n.bbox.h();
    writeln!(
        out,
        r#"{}<image href="{}" x="{}" y="{}" width="{}" height="{}"/>"#,
        indent,
        escape_xml(&href),
        num(-w / 2.0),
        num(-h / 2.0),
        num(w),
        num(h),
    )
    .unwrap();
}

fn emit_polygon(
    out: &mut String,
    indent: &str,
    points: &[(f64, f64)],
    fill: &str,
    stroke: &str,
    thickness: f64,
) {
    let pts_str: Vec<String> = points
        .iter()
        .map(|(x, y)| format!("{},{}", num(*x), num(*y)))
        .collect();
    writeln!(
        out,
        r#"{}<polygon points="{}" fill="{}" stroke="{}" stroke-width="{}"/>"#,
        indent,
        pts_str.join(" "),
        fill,
        stroke,
        num(thickness),
    )
    .unwrap();
}

fn fmt_pt((x, y): (f64, f64)) -> String {
    format!("{} {}", num(x), num(y))
}
