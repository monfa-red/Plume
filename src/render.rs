use crate::layout::{LaidOut, PlacedNode};
use crate::resolve::Shape;
use std::fmt::Write;

const STYLE_BLOCK: &str = r#"  <style>@layer plume.defaults { :root, .plume { --plume-fill: white; --plume-stroke: #444; --plume-text-color: #222; --plume-font: system-ui, -apple-system, sans-serif; } }</style>
"#;

pub fn render(laid_out: &LaidOut) -> String {
    let mut out = String::with_capacity(512);
    let vb = &laid_out.viewbox;

    writeln!(
        out,
        r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="{} {} {} {}" width="{}" height="{}" class="plume">"#,
        num(vb.x), num(vb.y), num(vb.w), num(vb.h),
        num(vb.w), num(vb.h),
    ).unwrap();

    out.push_str(STYLE_BLOCK);
    out.push_str("  <defs/>\n");
    out.push_str("  <g class=\"plume-scene\">\n");
    for node in &laid_out.nodes {
        render_node(&mut out, node);
    }
    out.push_str("  </g>\n");
    out.push_str("  <g class=\"plume-wires\"/>\n");
    out.push_str("</svg>\n");

    out
}

fn render_node(out: &mut String, n: &PlacedNode) {
    match n.shape {
        Shape::Rect => render_rect(out, n),
    }
}

fn render_rect(out: &mut String, n: &PlacedNode) {
    writeln!(
        out,
        r#"    <g class="plume-node plume-shape-rect" transform="translate({},{})">"#,
        num(n.cx),
        num(n.cy)
    )
    .unwrap();

    writeln!(
        out,
        r#"      <rect x="{}" y="{}" width="{}" height="{}" fill="var(--plume-fill)" stroke="var(--plume-stroke)" stroke-width="1"/>"#,
        num(-n.w / 2.0), num(-n.h / 2.0), num(n.w), num(n.h)
    ).unwrap();

    if let Some(label) = &n.label {
        writeln!(
            out,
            r#"      <text x="0" y="0" text-anchor="middle" dominant-baseline="central" font-size="13" font-family="var(--plume-font)" fill="var(--plume-text-color)">{}</text>"#,
            escape_xml(label)
        ).unwrap();
    }

    out.push_str("    </g>\n");
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
