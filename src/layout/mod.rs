mod anchors;
mod flex;
mod grid;
mod ir;
mod primitives;
mod text;
mod values;
mod wires;

pub use ir::*;

use crate::error::Error;
use crate::resolve::{Program, ResolvedInst, ResolvedValue, ShapeKind, VarTable};
use crate::span::Span;

use anchors::AbsolutePos;
use flex::Axis;

pub fn layout(program: &Program) -> Result<LaidOut, Error> {
    // Lay out top-level scene children.
    let mut top_nodes = Vec::with_capacity(program.scene.nodes.len());
    for inst in &program.scene.nodes {
        top_nodes.push(layout_inst(inst, &program.vars)?);
    }

    // Apply scene-level layout to top-level children (scene itself is a
    // container; its attrs drive how its children are positioned).
    let scene_bbox = lay_out_container_children(
        &mut top_nodes,
        &program.scene.attrs,
        &program.vars,
        Span::empty(),
    )?;

    // Route wires after nodes are placed.
    let routed_wires = wires::route_wires(program, &top_nodes)?;

    // Compute viewbox = (scene bbox ∪ wire paths) + canvas-pad on every side.
    let pad = values::layout_var(&program.vars, "canvas-pad").unwrap_or(20.0);
    let mut bbox = scene_bbox;
    for w in &routed_wires {
        for (x, y) in &w.path {
            bbox.min_x = bbox.min_x.min(*x);
            bbox.min_y = bbox.min_y.min(*y);
            bbox.max_x = bbox.max_x.max(*x);
            bbox.max_y = bbox.max_y.max(*y);
        }
    }
    let vb = ViewBox {
        x: bbox.min_x - pad,
        y: bbox.min_y - pad,
        w: bbox.w() + 2.0 * pad,
        h: bbox.h() + 2.0 * pad,
    };

    Ok(LaidOut {
        viewbox: vb,
        scene_attrs: program.scene.attrs.clone(),
        nodes: top_nodes,
        wires: routed_wires,
        vars: program.vars.clone(),
    })
}

/// Recursively lay out a single instance into a PlacedNode.
///
/// Bottom-up: lay out children first, then size this node around them. For
/// leaf primitives (no children), the shape's dimensions drive the bbox.
fn layout_inst(inst: &ResolvedInst, vars: &VarTable) -> Result<PlacedNode, Error> {
    // Recurse into children first.
    let mut children: Vec<PlacedNode> = Vec::with_capacity(inst.children.len());
    for c in &inst.children {
        children.push(layout_inst(c, vars)?);
    }

    // Determine this node's bbox + arrange children inside.
    let bbox = if children.is_empty() {
        // Leaf primitive.
        primitives::leaf_bbox(inst, vars)?
    } else {
        // Container or closed shape with content.
        let content_bbox = lay_out_container_children(&mut children, &inst.attrs, vars, inst.span)?;

        let has_explicit_layout = inst.attrs.get("layout").is_some();
        let only_text_content =
            !has_explicit_layout && children.iter().all(|c| c.shape == ShapeKind::Text);

        if let Some(explicit) = explicit_size(inst, vars)? {
            // Explicit w/h overrides auto-size.
            explicit
        } else if only_text_content {
            // Closed shape with text-only children → auto-size to text + text-pad.
            primitives::auto_sized_bbox(inst, content_bbox, vars, true)?
        } else if has_explicit_layout {
            // Container — size to content + padding.
            primitives::auto_sized_bbox(inst, content_bbox, vars, false)?
        } else {
            // Mixed content without explicit layout: size loosely to content.
            primitives::auto_sized_bbox(inst, content_bbox, vars, false)?
        }
    };

    let rotation = inst
        .attrs
        .get("rotation")
        .and_then(|v| match v {
            ResolvedValue::Number(n) => Some(*n),
            _ => None,
        })
        .unwrap_or(0.0);

    Ok(PlacedNode {
        id: inst.id.clone(),
        shape: inst.shape,
        type_chain: inst.type_chain.clone(),
        applied_styles: inst.applied_styles.clone(),
        label: inst.label.clone(),
        attrs: inst.attrs.clone(),
        markers: inst.markers.clone(),
        cx: 0.0,
        cy: 0.0,
        bbox,
        rotation,
        children,
        span: inst.span,
    })
}

/// Position children within their container per its `layout=` attr.
/// Returns the bounding bbox of all placed children, in container-local coords.
fn lay_out_container_children(
    children: &mut [PlacedNode],
    container_attrs: &crate::resolve::AttrMap,
    vars: &VarTable,
    span: Span,
) -> Result<Bbox, Error> {
    if children.is_empty() {
        return Ok(Bbox::empty());
    }

    // Separate flow vs absolutely-positioned children.
    let mut flow_indices: Vec<usize> = Vec::new();
    let mut abs_indices: Vec<usize> = Vec::new();
    for (i, c) in children.iter().enumerate() {
        if c.attrs.get("at").is_some() {
            abs_indices.push(i);
        } else {
            flow_indices.push(i);
        }
    }

    // Lay out the flow children per the container's layout attr.
    let layout_mode = container_attrs
        .get("layout")
        .and_then(|v| match v {
            ResolvedValue::Ident(s) => Some(s.as_str()),
            _ => None,
        })
        .unwrap_or("column");

    let flow_bbox = if !flow_indices.is_empty() {
        let mut flow_children: Vec<PlacedNode> =
            flow_indices.iter().map(|i| children[*i].clone()).collect();
        let bbox = match layout_mode {
            "row" => {
                flex::lay_out_flex(Axis::Row, &mut flow_children, container_attrs, vars, span)?
            }
            "column" => flex::lay_out_flex(
                Axis::Column,
                &mut flow_children,
                container_attrs,
                vars,
                span,
            )?,
            "grid" => grid::lay_out_grid(&mut flow_children, container_attrs, vars, span)?,
            other => {
                return Err(Error::at(span, format!("unknown layout '{}'", other)));
            }
        };
        for (slot, placed) in flow_indices.iter().zip(flow_children.into_iter()) {
            children[*slot] = placed;
        }
        bbox
    } else {
        Bbox::empty()
    };

    // Absolutely positioned children.
    for i in &abs_indices {
        let pos = anchors::parse_at(children[*i].attrs.get("at").unwrap(), children[*i].span)?;
        let offset = match children[*i].attrs.get("offset") {
            Some(v) => anchors::parse_offset(v, children[*i].span)?,
            None => (0.0, 0.0),
        };
        let (target_cx, target_cy) = match pos {
            AbsolutePos::Coord(x, y) => (x, y),
            AbsolutePos::Anchor(name) => {
                anchors::resolve_anchor(name, flow_bbox, children[*i].bbox)
            }
        };
        // `at=(x,y)` puts the bbox CENTER at (x,y) per §6 rule 1.
        let cb = children[*i].bbox;
        let local_off_x = (cb.min_x + cb.max_x) / 2.0;
        let local_off_y = (cb.min_y + cb.max_y) / 2.0;
        children[*i].cx = target_cx + offset.0 - local_off_x;
        children[*i].cy = target_cy + offset.1 - local_off_y;
    }

    // Compose union.
    let mut union = if !flow_indices.is_empty() {
        flow_bbox
    } else {
        Bbox::empty()
    };
    for i in &abs_indices {
        let cb = children[*i].bbox.shifted(children[*i].cx, children[*i].cy);
        union = if flow_indices.is_empty() && *i == abs_indices[0] {
            cb
        } else {
            union.union(cb)
        };
    }
    Ok(union)
}

/// If the container has explicit w/h, return a centered bbox of that size
/// (including stroke contribution).
fn explicit_size(inst: &ResolvedInst, vars: &VarTable) -> Result<Option<Bbox>, Error> {
    let w = inst.attrs.get("w").and_then(extract_num);
    let h = inst.attrs.get("h").and_then(extract_num);
    if let (Some(w), Some(h)) = (w, h) {
        let stroke = inst
            .attrs
            .get("thickness")
            .and_then(extract_num)
            .or_else(|| values::layout_var(vars, "thickness"))
            .unwrap_or(1.0)
            / 2.0;
        Ok(Some(Bbox::centered(w, h).inflate(stroke)))
    } else {
        Ok(None)
    }
}

fn extract_num(v: &ResolvedValue) -> Option<f64> {
    match v {
        ResolvedValue::Number(n) => Some(*n),
        ResolvedValue::LiveVar { baked: Some(b), .. } => extract_num(b),
        _ => None,
    }
}

// ───────────────────────────── Tests ─────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn lay_out(src: &str) -> LaidOut {
        let tokens = crate::lexer::lex(src).expect("lex");
        let file = crate::parser::parse(&tokens).expect("parse");
        let program = crate::resolve::resolve(file).expect("resolve");
        layout(&program).expect("layout")
    }

    #[test]
    fn rect_with_explicit_dims_keeps_those_dims() {
        let l = lay_out("scene { :rect w=200 h=80 }\n");
        let n = &l.nodes[0];
        // bbox includes stroke contribution of thickness/2 each side (= 0.5).
        assert!((n.bbox.w() - 201.0).abs() < 0.01, "bbox.w={}", n.bbox.w());
        assert!((n.bbox.h() - 81.0).abs() < 0.01, "bbox.h={}", n.bbox.h());
    }

    #[test]
    fn rect_with_label_auto_sizes_to_text_plus_pad() {
        let l = lay_out("scene { :rect \"hi\" }\n");
        let n = &l.nodes[0];
        // "hi" ≈ 2 × 13 × 0.55 = 14.3 wide. + 16 text-pad each side = 46.3.
        // Plus stroke (1) = 47.3.
        assert!(
            n.bbox.w() > 30.0 && n.bbox.w() < 60.0,
            "got w={}",
            n.bbox.w()
        );
    }

    #[test]
    fn oval_uses_rx_ry() {
        let l = lay_out("scene { :oval rx=50 ry=25 }\n");
        let n = &l.nodes[0];
        // Oval bbox = 2*rx by 2*ry, plus stroke.
        assert!((n.bbox.w() - 101.0).abs() < 0.01);
        assert!((n.bbox.h() - 51.0).abs() < 0.01);
    }

    #[test]
    fn row_layout_stacks_horizontally() {
        let l = lay_out(
            "scene layout=row gap=10 {\n\
               :rect w=100 h=40\n\
               :rect w=60 h=40\n\
             }\n",
        );
        assert_eq!(l.nodes.len(), 2);
        // Two rects + 10 gap = 100 + 10 + 60 = 170 total; centered on origin
        // means cx ≈ -35 for first, +50 for second (approx; depends on stroke).
        let dx = l.nodes[1].cx - l.nodes[0].cx;
        // gap (10) + 100/2 + 60/2 (centers) = 10 + 80 = 90; allow stroke
        assert!((dx - 90.0).abs() < 2.0, "dx={}", dx);
        // Cross axis same.
        assert!((l.nodes[0].cy - l.nodes[1].cy).abs() < 0.01);
    }

    #[test]
    fn column_layout_stacks_vertically() {
        let l = lay_out(
            "scene layout=column gap=20 {\n\
               :rect w=100 h=40\n\
               :rect w=100 h=60\n\
             }\n",
        );
        let dy = l.nodes[1].cy - l.nodes[0].cy;
        // gap (20) + 40/2 + 60/2 = 20 + 50 = 70
        assert!((dy - 70.0).abs() < 2.0, "dy={}", dy);
        assert!((l.nodes[0].cx - l.nodes[1].cx).abs() < 0.01);
    }

    #[test]
    fn grid_places_by_col_row() {
        let l = lay_out(
            "scene layout=grid cols=3 gap=20 {\n\
               :rect w=80 h=40 col=1 row=1\n\
               :rect w=80 h=40 col=3 row=1\n\
               :rect w=80 h=40 col=2 row=2\n\
             }\n",
        );
        assert_eq!(l.nodes.len(), 3);
        // Verify horizontal ordering of cols.
        assert!(l.nodes[0].cx < l.nodes[1].cx);
        // Verify second-row node is below first-row nodes.
        assert!(l.nodes[2].cy > l.nodes[0].cy);
    }

    #[test]
    fn at_coord_places_absolutely() {
        let l = lay_out("scene { :rect w=40 h=40 at=(100, 50) }\n");
        let n = &l.nodes[0];
        assert!((n.cx - 100.0).abs() < 0.01, "cx={}", n.cx);
        assert!((n.cy - 50.0).abs() < 0.01, "cy={}", n.cy);
    }

    #[test]
    fn viewbox_wraps_content_with_canvas_pad() {
        let l = lay_out("scene { :rect w=100 h=40 }\n");
        // canvas-pad defaults to 20. Content is 101 × 41 (stroke).
        assert!((l.viewbox.w - 141.0).abs() < 0.01, "w={}", l.viewbox.w);
        assert!((l.viewbox.h - 81.0).abs() < 0.01, "h={}", l.viewbox.h);
    }

    #[test]
    fn defaults_override_layout_var_changes_layout_math() {
        // Override gap default. Two children should now sit 60 apart, not 20.
        let l = lay_out(
            "defaults { gap=60 }\n\
             scene layout=row {\n\
               :rect w=40 h=40\n\
               :rect w=40 h=40\n\
             }\n",
        );
        let dx = l.nodes[1].cx - l.nodes[0].cx;
        // 60 gap + 20 + 20 (half widths) = 100
        assert!((dx - 100.0).abs() < 2.0, "dx={}", dx);
    }

    #[test]
    fn wire_between_two_rects_produces_path() {
        let l = lay_out(
            "scene layout=row gap=80 {\n\
               a :rect w=60 h=40\n\
               b :rect w=60 h=40\n\
             }\n\
             wires { a -> b }\n",
        );
        assert_eq!(l.wires.len(), 1);
        let w = &l.wires[0];
        assert!(w.path.len() >= 2, "path={:?}", w.path);
        // Markers: `->` defaults to end=Arrow, start=None.
        assert_eq!(w.markers.start, crate::resolve::MarkerKind::None);
        assert_eq!(w.markers.end, crate::resolve::MarkerKind::Arrow);
    }

    #[test]
    fn chain_wire_splits_into_subwires_with_markers_on_outer_ends() {
        let l = lay_out(
            "scene layout=row gap=40 {\n\
               a :rect w=40 h=40\n\
               b :rect w=40 h=40\n\
               c :rect w=40 h=40\n\
             }\n\
             wires { a -> b -> c }\n",
        );
        assert_eq!(l.wires.len(), 2, "expected 2 sub-wires for a→b→c");
        // First sub-wire keeps the start (None for `->`); end is suppressed.
        assert_eq!(l.wires[0].markers.end, crate::resolve::MarkerKind::None);
        // Last sub-wire keeps the end (Arrow).
        assert_eq!(l.wires[1].markers.end, crate::resolve::MarkerKind::Arrow);
    }

    #[test]
    fn auto_edge_picks_right_when_target_to_the_right() {
        let l = lay_out(
            "scene layout=row gap=80 {\n\
               a :rect w=60 h=40\n\
               b :rect w=60 h=40\n\
             }\n\
             wires { a -> b }\n",
        );
        let w = &l.wires[0];
        // Source (a) sits to the left of target (b); auto-edge picks the
        // right edge of a as the exit. First point should be on a's right
        // boundary (x ≈ a.cx + w/2).
        let a = &l.nodes[0];
        let a_right = a.cx + a.bbox.w() / 2.0;
        assert!(
            (w.path[0].0 - a_right).abs() < 1.0,
            "first point x={} != a's right edge {}",
            w.path[0].0,
            a_right
        );
    }

    #[test]
    fn bracket_anchor_overrides_auto_edge() {
        // Forcing source's left edge even though target is to the right.
        let l = lay_out(
            "scene layout=row gap=80 {\n\
               a :rect w=60 h=40\n\
               b :rect w=60 h=40\n\
             }\n\
             wires { a[left] -> b }\n",
        );
        let w = &l.wires[0];
        let a = &l.nodes[0];
        let a_left = a.cx - a.bbox.w() / 2.0;
        assert!(
            (w.path[0].0 - a_left).abs() < 1.0,
            "anchor should pin first point to a's left edge",
        );
    }

    #[test]
    fn wire_label_lands_on_path() {
        let l = lay_out(
            "scene layout=row gap=80 {\n\
               a :rect w=60 h=40\n\
               b :rect w=60 h=40\n\
             }\n\
             wires { a -> b \"CAN\" }\n",
        );
        let w = &l.wires[0];
        assert_eq!(w.texts.len(), 1);
        assert_eq!(w.texts[0].content, "CAN");
        // Mid-position should sit between path endpoints (approximately).
        let mid_x = (w.path[0].0 + w.path.last().unwrap().0) / 2.0;
        assert!(
            (w.texts[0].position.0 - mid_x).abs() < 30.0,
            "label x={} far from midpoint {}",
            w.texts[0].position.0,
            mid_x
        );
    }

    #[test]
    fn full_spec_example_routes_all_wires() {
        let src = std::fs::read_to_string("samples/full_example.plume").unwrap();
        let tokens = crate::lexer::lex(&src).expect("lex");
        let file = crate::parser::parse(&tokens).expect("parse");
        let program = crate::resolve::resolve(file).expect("resolve");
        let l = layout(&program).expect("layout");
        // §20 has 5 wire decls totalling 10 sub-segments:
        //   outlet→drive→bus48→fuse→caps (4)
        //   outlet→ctrl→bus24            (2)
        //   bus48→ssr→bands              (2)
        //   fadec↔drive                  (1)
        //   estop→fuse                   (1)
        assert_eq!(l.wires.len(), 10, "got {} routed wires", l.wires.len());
    }

    #[test]
    fn full_spec_example_lays_out_without_error() {
        let src = std::fs::read_to_string("samples/full_example.plume").unwrap();
        let tokens = crate::lexer::lex(&src).expect("lex");
        let file = crate::parser::parse(&tokens).expect("parse");
        let program = crate::resolve::resolve(file).expect("resolve");
        let l = layout(&program).expect("layout");
        // Smoke check: scene viewbox should be non-trivial and there should be
        // at least the top-level nodes we expect (outlet/rails/consumers/fadec/fd1).
        assert!(l.viewbox.w > 100.0);
        assert!(l.viewbox.h > 100.0);
        assert!(l.nodes.len() >= 5);
    }
}
