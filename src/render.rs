use crate::layout::{LaidOut, PlacedNode, RoutedText, RoutedWire};
use crate::resolve::{MarkerKind, Markers, ResolvedValue, ShapeKind};
use std::fmt::Write;

const STYLE_BLOCK: &str = r#"  <style>@layer plume.defaults { :root, .plume { --plume-fill: white; --plume-stroke: #444; --plume-text-color: #222; --plume-font: system-ui, -apple-system, sans-serif; } }</style>
"#;

pub fn render(laid_out: &LaidOut) -> String {
    let mut out = String::with_capacity(1024);
    let vb = &laid_out.viewbox;

    writeln!(
        out,
        r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="{} {} {} {}" width="{}" height="{}" class="plume">"#,
        num(vb.x),
        num(vb.y),
        num(vb.w),
        num(vb.h),
        num(vb.w),
        num(vb.h),
    )
    .unwrap();

    out.push_str(STYLE_BLOCK);
    out.push_str("  <defs/>\n");
    out.push_str("  <g class=\"plume-scene\">\n");
    for node in &laid_out.nodes {
        render_node(&mut out, node, 2);
    }
    out.push_str("  </g>\n");

    if laid_out.wires.is_empty() {
        out.push_str("  <g class=\"plume-wires\"/>\n");
    } else {
        out.push_str("  <g class=\"plume-wires\">\n");
        for wire in &laid_out.wires {
            render_wire(&mut out, wire);
        }
        out.push_str("  </g>\n");
    }
    out.push_str("</svg>\n");

    out
}

fn render_wire(out: &mut String, w: &RoutedWire) {
    if w.path.len() < 2 {
        return;
    }
    let stroke = attr_str(&w.attrs, "stroke", "var(--plume-stroke)");
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
        d.push_str(&format!(" L {} {}", num(p.0), num(p.1)));
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

    let inset = 4.0_f64;
    if w.markers.start != MarkerKind::None {
        if let Some((tip, dir)) = wire_marker_anchor(&w.path, true, inset) {
            render_wire_marker(out, w.markers.start, tip, dir, &stroke, thickness);
        }
    }
    if w.markers.end != MarkerKind::None {
        if let Some((tip, dir)) = wire_marker_anchor(&w.path, false, inset) {
            render_wire_marker(out, w.markers.end, tip, dir, &stroke, thickness);
        }
    }

    for t in &w.texts {
        render_wire_text(out, t);
    }

    out.push_str("    </g>\n");
}

fn wire_dash(attrs: &crate::resolve::AttrMap) -> String {
    if let Some(ResolvedValue::Tuple(items)) = attrs.get("dashed") {
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
    if let Some(ResolvedValue::Tuple(items)) = attrs.get("dotted") {
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
    String::new()
}

/// Position the marker tip inset from the endpoint along the last segment.
fn wire_marker_anchor(
    path: &[(f64, f64)],
    at_start: bool,
    inset: f64,
) -> Option<((f64, f64), (f64, f64))> {
    if path.len() < 2 {
        return None;
    }
    let (anchor, neighbor) = if at_start {
        (path[0], path[1])
    } else {
        (path[path.len() - 1], path[path.len() - 2])
    };
    let dx = anchor.0 - neighbor.0;
    let dy = anchor.1 - neighbor.1;
    let len = (dx * dx + dy * dy).sqrt();
    if len < 1e-9 {
        return Some((anchor, (1.0, 0.0)));
    }
    let ux = dx / len;
    let uy = dy / len;
    let tip = (anchor.0 - ux * inset, anchor.1 - uy * inset);
    Some((tip, (ux, uy)))
}

fn render_wire_marker(
    out: &mut String,
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
                r#"      <polygon class="plume-marker plume-marker-arrow" points="{},{} {},{} {},{}" fill="{}"/>"#,
                num(tip.0),
                num(tip.1),
                num(lx),
                num(ly),
                num(rx),
                num(ry),
                stroke,
            )
            .unwrap();
        }
        MarkerKind::Dot => {
            writeln!(
                out,
                r#"      <circle class="plume-marker plume-marker-dot" cx="{}" cy="{}" r="{}" fill="{}"/>"#,
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
                r#"      <polygon class="plume-marker plume-marker-diamond" points="{},{} {},{} {},{} {},{}" fill="{}"/>"#,
                num(tip.0),
                num(tip.1),
                num(lx),
                num(ly),
                num(bx),
                num(by),
                num(rx),
                num(ry),
                stroke,
            )
            .unwrap();
        }
        MarkerKind::Crow => {
            // Crow's foot: three lines fanning back from the tip.
            let bx = tip.0 - ux * size;
            let by = tip.1 - uy * size;
            let lx = bx + px * size * 0.5;
            let ly = by + py * size * 0.5;
            let rx = bx - px * size * 0.5;
            let ry = by - py * size * 0.5;
            writeln!(
                out,
                r#"      <path class="plume-marker plume-marker-crow" d="M {} {} L {} {} M {} {} L {} {} M {} {} L {} {}" stroke="{}" stroke-width="{}" fill="none"/>"#,
                num(tip.0),
                num(tip.1),
                num(bx),
                num(by),
                num(tip.0),
                num(tip.1),
                num(lx),
                num(ly),
                num(tip.0),
                num(tip.1),
                num(rx),
                num(ry),
                stroke,
                num(thickness),
            )
            .unwrap();
        }
        MarkerKind::None => {}
    }
}

fn render_wire_text(out: &mut String, t: &RoutedText) {
    let size = attr_num(&t.attrs, "size").unwrap_or(11.0);
    let fill = attr_str(&t.attrs, "fill", "var(--plume-text-color)");
    // For Sprint 4, place the text just above the route point (small lift).
    let lift = size * 0.7;
    let nx = -t.tangent.1;
    let ny = t.tangent.0;
    let x = t.position.0 + nx * lift;
    let y = t.position.1 + ny * lift;
    writeln!(
        out,
        r#"      <text x="{}" y="{}" text-anchor="middle" dominant-baseline="central" font-size="{}" font-family="var(--plume-font)" fill="{}">{}</text>"#,
        num(x),
        num(y),
        num(size),
        fill,
        escape_xml(&t.content),
    )
    .unwrap();
}

fn render_node(out: &mut String, n: &PlacedNode, depth: usize) {
    let indent = "  ".repeat(depth);
    let transform = if n.rotation != 0.0 {
        format!(
            r#" transform="translate({},{}) rotate({})""#,
            num(n.cx),
            num(n.cy),
            num(n.rotation)
        )
    } else {
        format!(r#" transform="translate({},{})""#, num(n.cx), num(n.cy))
    };

    let id_attr = match &n.id {
        Some(id) => format!(r#" data-id="{}""#, escape_xml(id)),
        None => String::new(),
    };

    writeln!(
        out,
        r#"{}<g class="plume-node plume-shape-{}"{}{}>"#,
        indent,
        n.shape.as_str(),
        id_attr,
        transform,
    )
    .unwrap();

    render_geometry(out, n, depth + 1);

    for child in &n.children {
        render_node(out, child, depth + 1);
    }

    writeln!(out, "{}</g>", indent).unwrap();
}

fn render_geometry(out: &mut String, n: &PlacedNode, depth: usize) {
    let indent = "  ".repeat(depth);
    let fill = attr_str(&n.attrs, "fill", "var(--plume-fill)");
    let stroke = attr_str(&n.attrs, "stroke", "var(--plume-stroke)");
    let thickness = attr_num(&n.attrs, "thickness").unwrap_or(1.0);

    match n.shape {
        ShapeKind::Rect
        | ShapeKind::Slant
        | ShapeKind::Hex
        | ShapeKind::Diamond
        | ShapeKind::Cyl
        | ShapeKind::Cloud => {
            // For Sprint 3 we render every closed shape as its bbox rect.
            // Sprint 5 swaps in shape-specific geometry (hex polygon, cyl
            // ellipses + lines, etc.).
            let w = bbox_width_excluding_stroke(n, thickness);
            let h = bbox_height_excluding_stroke(n, thickness);
            let radius = attr_num(&n.attrs, "radius").unwrap_or(0.0);
            writeln!(
                out,
                r#"{}<rect x="{}" y="{}" width="{}" height="{}" rx="{}" ry="{}" fill="{}" stroke="{}" stroke-width="{}"/>"#,
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
            )
            .unwrap();
        }
        ShapeKind::Oval => {
            let rx = attr_num(&n.attrs, "r")
                .or_else(|| attr_num(&n.attrs, "rx"))
                .unwrap_or((n.bbox.w() - thickness) / 2.0);
            let ry = attr_num(&n.attrs, "r")
                .or_else(|| attr_num(&n.attrs, "ry"))
                .unwrap_or((n.bbox.h() - thickness) / 2.0);
            writeln!(
                out,
                r#"{}<ellipse cx="0" cy="0" rx="{}" ry="{}" fill="{}" stroke="{}" stroke-width="{}"/>"#,
                indent,
                num(rx),
                num(ry),
                fill,
                stroke,
                num(thickness),
            )
            .unwrap();
        }
        ShapeKind::Text => {
            let size = attr_num(&n.attrs, "size").unwrap_or(13.0);
            let label = n.label.as_deref().unwrap_or("");
            let text_fill = attr_str(&n.attrs, "fill", "var(--plume-text-color)");
            writeln!(
                out,
                r#"{}<text x="0" y="0" text-anchor="middle" dominant-baseline="central" font-size="{}" font-family="var(--plume-font)" fill="{}">{}</text>"#,
                indent,
                num(size),
                text_fill,
                escape_xml(label),
            )
            .unwrap();
        }
        ShapeKind::Line | ShapeKind::Arrow => {
            let from = attr_pair(&n.attrs, "from").unwrap_or((0.0, 0.0));
            let to = attr_pair(&n.attrs, "to").unwrap_or((0.0, 0.0));
            writeln!(
                out,
                r#"{}<line x1="{}" y1="{}" x2="{}" y2="{}" stroke="{}" stroke-width="{}"/>"#,
                indent,
                num(from.0),
                num(from.1),
                num(to.0),
                num(to.1),
                stroke,
                num(thickness),
            )
            .unwrap();
            render_markers(out, &n.markers, from, to, &stroke, thickness, &indent);
        }
        ShapeKind::Icon | ShapeKind::Image | ShapeKind::Poly | ShapeKind::Path => {
            // Sprint 5: actual rendering. Sprint 3 emits a placeholder bbox.
            let w = n.bbox.w();
            let h = n.bbox.h();
            writeln!(
                out,
                r#"{}<rect x="{}" y="{}" width="{}" height="{}" fill="none" stroke="{}" stroke-width="1" stroke-dasharray="2,2"/>"#,
                indent,
                num(n.bbox.min_x),
                num(n.bbox.min_y),
                num(w),
                num(h),
                stroke,
            )
            .unwrap();
        }
    }
}

fn render_markers(
    out: &mut String,
    markers: &Markers,
    from: (f64, f64),
    to: (f64, f64),
    stroke: &str,
    thickness: f64,
    indent: &str,
) {
    if markers.start != MarkerKind::None {
        render_marker(out, markers.start, to, from, stroke, thickness, indent);
    }
    if markers.end != MarkerKind::None {
        render_marker(out, markers.end, from, to, stroke, thickness, indent);
    }
}

/// Emit a marker at point `tip` (the endpoint), pointing away from `tail`.
fn render_marker(
    out: &mut String,
    kind: MarkerKind,
    tail: (f64, f64),
    tip: (f64, f64),
    stroke: &str,
    thickness: f64,
    indent: &str,
) {
    let size = (10.0_f64).max(thickness * 5.0);
    let dx = tip.0 - tail.0;
    let dy = tip.1 - tail.1;
    let len = (dx * dx + dy * dy).sqrt().max(0.001);
    let ux = dx / len;
    let uy = dy / len;
    let px = -uy;
    let py = ux;
    match kind {
        MarkerKind::Arrow => {
            let tipx = tip.0;
            let tipy = tip.1;
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
                num(tipx),
                num(tipy),
                num(lx),
                num(ly),
                num(rx),
                num(ry),
                stroke,
            )
            .unwrap();
        }
        MarkerKind::Dot => {
            writeln!(
                out,
                r#"{}<circle class="plume-marker plume-marker-dot" cx="{}" cy="{}" r="{}" fill="{}"/>"#,
                indent,
                num(tip.0),
                num(tip.1),
                num(size / 4.0),
                stroke,
            )
            .unwrap();
        }
        _ => {
            // diamond / crow: placeholders for Sprint 5.
            writeln!(
                out,
                r#"{}<circle class="plume-marker plume-marker-{:?}" cx="{}" cy="{}" r="{}" fill="{}"/>"#,
                indent,
                kind,
                num(tip.0),
                num(tip.1),
                num(size / 4.0),
                stroke,
            )
            .unwrap();
        }
    }
}

fn bbox_width_excluding_stroke(n: &PlacedNode, thickness: f64) -> f64 {
    (n.bbox.w() - thickness).max(0.0)
}

fn bbox_height_excluding_stroke(n: &PlacedNode, thickness: f64) -> f64 {
    (n.bbox.h() - thickness).max(0.0)
}

fn attr_str(attrs: &crate::resolve::AttrMap, name: &str, fallback: &str) -> String {
    match attrs.get(name) {
        Some(ResolvedValue::Hex(h)) => format!("#{}", h),
        Some(ResolvedValue::Ident(s)) => s.clone(),
        Some(ResolvedValue::String(s)) => s.clone(),
        Some(ResolvedValue::LiveVar { name, raw, .. }) => {
            if *raw {
                format!("var(--{})", name)
            } else {
                format!("var(--plume-{})", name)
            }
        }
        Some(ResolvedValue::Call(c)) => format_call(c),
        _ => fallback.to_string(),
    }
}

fn format_call(c: &crate::resolve::ResolvedCall) -> String {
    let args: Vec<String> = c
        .args
        .iter()
        .map(|a| match a {
            ResolvedValue::Number(n) => num(*n),
            ResolvedValue::Ident(s) => s.clone(),
            ResolvedValue::LiveVar { name, raw, .. } => {
                if *raw {
                    format!("var(--{})", name)
                } else {
                    format!("var(--plume-{})", name)
                }
            }
            _ => String::from("/* unsupported */"),
        })
        .collect();
    format!("{}({})", c.name, args.join(", "))
}

fn attr_num(attrs: &crate::resolve::AttrMap, name: &str) -> Option<f64> {
    attrs.get(name).and_then(|v| match v {
        ResolvedValue::Number(n) => Some(*n),
        ResolvedValue::LiveVar { baked: Some(b), .. } => match **b {
            ResolvedValue::Number(n) => Some(n),
            _ => None,
        },
        _ => None,
    })
}

fn attr_pair(attrs: &crate::resolve::AttrMap, name: &str) -> Option<(f64, f64)> {
    match attrs.get(name)? {
        ResolvedValue::Tuple(items) if items.len() == 2 => {
            let x = match &items[0] {
                ResolvedValue::Number(n) => *n,
                _ => return None,
            };
            let y = match &items[1] {
                ResolvedValue::Number(n) => *n,
                _ => return None,
            };
            Some((x, y))
        }
        _ => None,
    }
}

fn num(n: f64) -> String {
    if n.is_finite() && n == n.trunc() && n.abs() < 1e15 {
        return (n as i64).to_string();
    }
    let s = format!("{:.4}", n);
    let trimmed = s.trim_end_matches('0').trim_end_matches('.');
    if trimmed.is_empty() || trimmed == "-" {
        "0".to_string()
    } else {
        trimmed.to_string()
    }
}

fn escape_xml(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        match ch {
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '&' => out.push_str("&amp;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            _ => out.push(ch),
        }
    }
    out
}
