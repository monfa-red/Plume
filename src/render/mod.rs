mod markers;
mod primitives;
mod style_block;
mod values;
mod wires;

use crate::layout::{LaidOut, PlacedNode};
use crate::Options;
use values::{build_classes, escape_xml, num};

pub fn render(laid_out: &LaidOut, opts: &Options) -> String {
    let mut out = String::with_capacity(2048);
    let vb = &laid_out.viewbox;

    use std::fmt::Write;
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

    style_block::emit(&mut out, &laid_out.vars, opts);
    out.push_str("  <defs/>\n");

    out.push_str("  <g class=\"plume-scene\">\n");
    for node in &laid_out.nodes {
        render_node(&mut out, node, 2, &laid_out.vars, opts);
    }
    out.push_str("  </g>\n");

    if laid_out.wires.is_empty() {
        out.push_str("  <g class=\"plume-wires\"/>\n");
    } else {
        out.push_str("  <g class=\"plume-wires\">\n");
        for wire in &laid_out.wires {
            wires::render_wire(&mut out, wire, &laid_out.vars, opts);
        }
        out.push_str("  </g>\n");
    }

    out.push_str("</svg>\n");
    out
}

fn render_node(
    out: &mut String,
    n: &PlacedNode,
    depth: usize,
    vars: &crate::resolve::VarTable,
    opts: &Options,
) {
    use std::fmt::Write;
    let indent = "  ".repeat(depth);
    let classes = build_classes(n.shape.as_str(), &n.type_chain, &n.applied_styles);
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
        r#"{}<g class="{}"{}{}>"#,
        indent, classes, id_attr, transform
    )
    .unwrap();

    primitives::render_geometry(out, n, depth + 1, vars, opts);
    for child in &n.children {
        render_node(out, child, depth + 1, vars, opts);
    }

    writeln!(out, "{}</g>", indent).unwrap();
}
