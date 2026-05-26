mod ir;
mod shapes;
mod styles;
mod vars;

pub use ir::*;

use crate::ast::{
    AttrItem, Block, DefaultsBlock, File, ShapeInst, ShapesBlock, StylesBlock, TypeRef, WireOp,
    WiresBlock,
};
use crate::error::Error;
use crate::span::Span;
use std::collections::HashMap;

pub fn resolve(file: File) -> Result<Program, Error> {
    check_block_order(&file.blocks)?;

    let mut defaults_block: Option<&DefaultsBlock> = None;
    let mut styles_block: Option<&StylesBlock> = None;
    let mut shapes_block: Option<&ShapesBlock> = None;
    let mut scene_block_opt = None;
    let mut wires_block: Option<&WiresBlock> = None;

    for b in &file.blocks {
        match b {
            Block::Defaults(b) => defaults_block = Some(b),
            Block::Styles(b) => styles_block = Some(b),
            Block::Shapes(b) => shapes_block = Some(b),
            Block::Scene(b) => scene_block_opt = Some(b),
            Block::Wires(b) => wires_block = Some(b),
        }
    }

    let scene_block = scene_block_opt
        .ok_or_else(|| Error::at(Span::empty(), "missing required 'scene' block"))?;

    // Vars
    let mut vars = vars::built_in_defaults();
    if let Some(d) = defaults_block {
        vars::apply_defaults_block(&mut vars, &d.entries)?;
    }

    // Styles
    let styles_table = match styles_block {
        Some(b) => styles::StyleTable::build(&b.styles, &vars)?,
        None => styles::StyleTable::build(&[], &vars)?,
    };

    // Shapes
    let shapes_table = match shapes_block {
        Some(b) => shapes::ShapesTable::build(&b.shapes, &styles_table, &vars)?,
        None => shapes::ShapesTable::build(&[], &styles_table, &vars)?,
    };

    // Scene
    let mut id_seen: HashMap<String, Span> = HashMap::new();
    let scene_attrs_items = resolve_attrs(&scene_block.items, &styles_table, &vars)?;
    let scene_attrs = collapse(&scene_attrs_items);

    let mut scene_nodes = Vec::new();
    for inst in &scene_block.body {
        scene_nodes.push(resolve_inst(
            inst,
            &shapes_table,
            &styles_table,
            &vars,
            &mut id_seen,
        )?);
    }

    // Wires (after scene IDs are known)
    let wires = match wires_block {
        Some(b) => resolve_wires(b, &styles_table, &vars, &id_seen)?,
        None => Vec::new(),
    };

    Ok(Program {
        vars,
        scene: ResolvedScene {
            attrs: scene_attrs,
            nodes: scene_nodes,
        },
        wires,
    })
}

// ───────────────────────────── Block ordering ─────────────────────────────

fn check_block_order(blocks: &[Block]) -> Result<(), Error> {
    let position = |b: &Block| -> usize {
        match b {
            Block::Defaults(_) => 0,
            Block::Styles(_) => 1,
            Block::Shapes(_) => 2,
            Block::Scene(_) => 3,
            Block::Wires(_) => 4,
        }
    };
    let name = |b: &Block| -> &'static str {
        match b {
            Block::Defaults(_) => "defaults",
            Block::Styles(_) => "styles",
            Block::Shapes(_) => "shapes",
            Block::Scene(_) => "scene",
            Block::Wires(_) => "wires",
        }
    };
    let span_of = |b: &Block| -> Span {
        match b {
            Block::Defaults(b) => b.span,
            Block::Styles(b) => b.span,
            Block::Shapes(b) => b.span,
            Block::Scene(b) => b.span,
            Block::Wires(b) => b.span,
        }
    };
    let names = ["defaults", "styles", "shapes", "scene", "wires"];

    let mut seen: HashMap<&str, ()> = HashMap::new();
    let mut max_seen = 0usize;
    for b in blocks {
        let pos = position(b);
        let n = name(b);
        if seen.contains_key(n) {
            return Err(Error::at(span_of(b), format!("duplicate '{}' block", n)));
        }
        if pos < max_seen {
            let later = names[max_seen];
            return Err(Error::at(
                span_of(b),
                format!("'{}' must appear before '{}'", n, later),
            ));
        }
        max_seen = pos;
        seen.insert(n, ());
    }
    Ok(())
}

// ───────────────────────────── Reserved names ─────────────────────────────

pub(super) fn is_reserved(name: &str) -> bool {
    matches!(
        name,
        // Block names
        "defaults" | "styles" | "shapes" | "scene" | "wires"
        // Layout values
        | "row" | "column" | "grid"
        | "start" | "center" | "end" | "stretch" | "between" | "around" | "evenly"
        // Anchors
        | "top" | "bottom" | "left" | "right"
        | "top-left" | "top-right" | "bottom-left" | "bottom-right"
        | "out-top" | "out-bottom" | "out-left" | "out-right"
        | "out-top-left" | "out-top-right" | "out-bottom-left" | "out-bottom-right"
        | "mid"
        // Primitives
        | "rect" | "oval" | "line" | "arrow" | "path" | "poly" | "text"
        | "hex" | "slant" | "cyl" | "diamond" | "cloud" | "icon" | "image"
        // Templates
        | "group" | "circle" | "badge" | "button" | "card" | "note"
        | "db" | "table" | "cell" | "dim"
        // Constants
        | "true" | "false" | "none" | "auto"
        // Functions
        | "var" | "rgb" | "rgba" | "hsl"
    )
}

// ───────────────────────────── Attr resolution ─────────────────────────────

fn resolve_attrs(
    items: &[AttrItem],
    styles: &styles::StyleTable,
    vars: &VarTable,
) -> Result<Vec<ResolvedAttr>, Error> {
    let mut out = Vec::new();
    for item in items {
        match item {
            AttrItem::Attr(a) => {
                let value = match &a.value {
                    Some(v) => Some(vars::resolve_value(v, vars)?),
                    None => None,
                };
                out.push(ResolvedAttr {
                    name: a.name.clone(),
                    value,
                    span: a.span,
                });
            }
            AttrItem::Style(s) => {
                let inner = styles
                    .lookup(&s.name)
                    .ok_or_else(|| Error::at(s.span, format!("unknown style '.{}'", s.name)))?;
                out.extend(inner.iter().cloned());
            }
        }
    }
    Ok(out)
}

/// Collapse an ordered list of attrs (after specificity merging) into the final
/// map. Marker-related attrs are stripped — they're handled by `resolve_markers`.
fn collapse(items: &[ResolvedAttr]) -> AttrMap {
    let mut map = AttrMap::new();
    for item in items {
        if is_marker_attr(&item.name) {
            continue;
        }
        let value = match &item.value {
            Some(v) => v.clone(),
            None => bare_attr_default(&item.name).unwrap_or(ResolvedValue::Ident("true".into())),
        };
        map.insert(item.name.clone(), value);
    }
    map
}

fn is_marker_attr(name: &str) -> bool {
    matches!(name, "marker" | "marker-start" | "marker-end")
}

fn bare_attr_default(name: &str) -> Option<ResolvedValue> {
    Some(match name {
        "dashed" => {
            ResolvedValue::Tuple(vec![ResolvedValue::Number(4.0), ResolvedValue::Number(4.0)])
        }
        "dotted" => {
            ResolvedValue::Tuple(vec![ResolvedValue::Number(1.0), ResolvedValue::Number(3.0)])
        }
        "double" => ResolvedValue::Tuple(vec![
            ResolvedValue::Number(4.0),
            ResolvedValue::Number(-4.0),
        ]),
        "shadow" => ResolvedValue::Tuple(vec![
            ResolvedValue::Number(2.0),
            ResolvedValue::Number(2.0),
            ResolvedValue::Number(4.0),
            ResolvedValue::LiveVar {
                name: "shadow".into(),
                raw: false,
                baked: None,
            },
        ]),
        _ => return None,
    })
}

// ───────────────────────────── Marker resolution ─────────────────────────────

fn resolve_markers(
    items: &[ResolvedAttr],
    default_start: MarkerKind,
    default_end: MarkerKind,
) -> Result<Markers, Error> {
    let mut start = default_start;
    let mut end = default_end;
    for item in items {
        match item.name.as_str() {
            "marker" => {
                let m = expect_marker(&item.value, item.span)?;
                start = m;
                end = m;
            }
            "marker-start" => {
                start = expect_marker(&item.value, item.span)?;
            }
            "marker-end" => {
                end = expect_marker(&item.value, item.span)?;
            }
            _ => {}
        }
    }
    Ok(Markers { start, end })
}

fn expect_marker(value: &Option<ResolvedValue>, span: Span) -> Result<MarkerKind, Error> {
    match value {
        Some(ResolvedValue::Ident(s)) => MarkerKind::parse(s)
            .ok_or_else(|| Error::at(span, format!("invalid marker value '{}'", s))),
        _ => Err(Error::at(span, "marker attr requires an identifier value")),
    }
}

fn default_markers_for_shape(kind: ShapeKind) -> Markers {
    match kind {
        ShapeKind::Arrow => Markers {
            start: MarkerKind::None,
            end: MarkerKind::Arrow,
        },
        _ => Markers::default(),
    }
}

fn default_markers_for_op(op: WireOp) -> Markers {
    use WireOp::*;
    match op {
        Arrow | ArrowDash | ArrowDot => Markers {
            start: MarkerKind::None,
            end: MarkerKind::Arrow,
        },
        LArrow | LArrowDash | LArrowDot => Markers {
            start: MarkerKind::Arrow,
            end: MarkerKind::None,
        },
        Biarrow | BiarrowDash | BiarrowDot => Markers {
            start: MarkerKind::Arrow,
            end: MarkerKind::Arrow,
        },
    }
}

// ───────────────────────────── Scene tree ─────────────────────────────

fn resolve_inst(
    inst: &ShapeInst,
    shapes: &shapes::ShapesTable,
    styles: &styles::StyleTable,
    vars: &VarTable,
    id_seen: &mut HashMap<String, Span>,
) -> Result<ResolvedInst, Error> {
    let resolved_shape = shapes.resolve(&inst.ty.name, inst.ty.span)?;

    // Collect style names applied directly on this inst (left-to-right).
    let applied_styles: Vec<String> = inst
        .items
        .iter()
        .filter_map(|i| match i {
            AttrItem::Style(s) => Some(s.name.clone()),
            AttrItem::Attr(_) => None,
        })
        .collect();

    // ID uniqueness + reserved check.
    if let Some(id) = &inst.id {
        if is_reserved(id) {
            return Err(Error::at(inst.span, format!("'{}' is reserved", id)));
        }
        if let Some(_prev) = id_seen.get(id) {
            return Err(Error::at(inst.span, format!("duplicate scene id '{}'", id)));
        }
        id_seen.insert(id.clone(), inst.span);
    }

    // Merge: type-default attrs + inline (styles + attrs) in source order.
    let inline = resolve_attrs(&inst.items, styles, vars)?;
    let mut ordered = resolved_shape.attrs.clone();
    ordered.extend(inline);

    // Markers (source-order-sensitive pass).
    let defaults = default_markers_for_shape(resolved_shape.kind);
    let markers = resolve_markers(&ordered, defaults.start, defaults.end)?;

    let attrs = collapse(&ordered);

    // Body: shape-def intrinsic children, then label sugar (for non-Text shapes
    // — Text shapes treat label as their own content), then inline children.
    let mut body_items: Vec<ShapeInst> = resolved_shape.body_items.clone();
    let own_label = if resolved_shape.kind == ShapeKind::Text {
        inst.label.clone()
    } else {
        if let Some(label) = &inst.label {
            body_items.push(label_sugar_text(label, inst.span));
        }
        None
    };
    if let Some(b) = &inst.body {
        body_items.extend(b.iter().cloned());
    }

    let mut children = Vec::new();
    for child in &body_items {
        children.push(resolve_inst(child, shapes, styles, vars, id_seen)?);
    }

    Ok(ResolvedInst {
        id: inst.id.clone(),
        shape: resolved_shape.kind,
        type_chain: resolved_shape.type_chain,
        applied_styles,
        label: own_label,
        attrs,
        markers,
        children,
        span: inst.span,
    })
}

fn label_sugar_text(text: &str, span: Span) -> ShapeInst {
    ShapeInst {
        id: None,
        ty: TypeRef {
            name: "text".to_string(),
            span,
        },
        label: Some(text.to_string()),
        items: Vec::new(),
        body: None,
        span,
    }
}

// ───────────────────────────── Wires ─────────────────────────────

fn resolve_wires(
    block: &WiresBlock,
    styles: &styles::StyleTable,
    vars: &VarTable,
    scene_ids: &HashMap<String, Span>,
) -> Result<Vec<ResolvedWire>, Error> {
    let block_attrs = resolve_attrs(&block.items, styles, vars)?;

    let mut wires = Vec::new();
    for w in &block.wires {
        for ep in &w.endpoints {
            if !scene_ids.contains_key(&ep.id) {
                return Err(Error::at(
                    ep.span,
                    format!("wire references undefined id '{}'", ep.id),
                ));
            }
        }

        let inline = resolve_attrs(&w.items, styles, vars)?;
        let mut ordered = block_attrs.clone();
        ordered.extend(inline);

        let defaults = default_markers_for_op(w.op);
        let markers = resolve_markers(&ordered, defaults.start, defaults.end)?;
        let attrs = collapse(&ordered);

        let endpoints = w
            .endpoints
            .iter()
            .map(|ep| ResolvedEndpoint {
                id: ep.id.clone(),
                anchor: ep.anchor,
                span: ep.span,
            })
            .collect();

        let mut texts = Vec::new();
        if let Some(label) = &w.label {
            texts.push(ResolvedText {
                text: label.clone(),
                at: WireAt::Mid,
                attrs: AttrMap::new(),
                span: w.span,
            });
        }
        if let Some(body) = &w.body {
            for t in body {
                let t_attrs_items = resolve_attrs(&t.items, styles, vars)?;
                let mut at = WireAt::Mid;
                let mut t_map = AttrMap::new();
                for item in &t_attrs_items {
                    if item.name == "at" {
                        if let Some(v) = &item.value {
                            at = WireAt::parse(v).ok_or_else(|| {
                                Error::at(
                                    item.span,
                                    format!(
                                        ":text anchor '{:?}' is wire-only; use start/mid/end/0..1",
                                        v
                                    ),
                                )
                            })?;
                        }
                    } else {
                        let value = item.value.clone().unwrap_or_else(|| {
                            bare_attr_default(&item.name)
                                .unwrap_or(ResolvedValue::Ident("true".into()))
                        });
                        t_map.insert(item.name.clone(), value);
                    }
                }
                texts.push(ResolvedText {
                    text: t.text.clone(),
                    at,
                    attrs: t_map,
                    span: t.span,
                });
            }
        }

        wires.push(ResolvedWire {
            endpoints,
            op: w.op,
            attrs,
            markers,
            texts,
            span: w.span,
        });
    }
    Ok(wires)
}

// ───────────────────────────── Tests ─────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn resolve_str(src: &str) -> Program {
        let tokens = crate::lexer::lex(src).expect("lex");
        let file = crate::parser::parse(&tokens).expect("parse");
        resolve(file).expect("resolve")
    }

    #[test]
    fn marker_order_marker_before_marker_end() {
        // marker=arrow (sets both), then marker-end=dot (overrides end only).
        let p = resolve_str(
            "scene { a :rect \"A\"\n b :rect \"B\" }\n\
             wires { a -> b marker=arrow marker-end=dot }\n",
        );
        let w = &p.wires[0];
        assert_eq!(w.markers.start, MarkerKind::Arrow);
        assert_eq!(w.markers.end, MarkerKind::Dot);
    }

    #[test]
    fn marker_order_marker_end_before_marker() {
        // marker-end=dot first, then marker=arrow overrides both.
        let p = resolve_str(
            "scene { a :rect \"A\"\n b :rect \"B\" }\n\
             wires { a -> b marker-end=dot marker=arrow }\n",
        );
        let w = &p.wires[0];
        assert_eq!(w.markers.start, MarkerKind::Arrow);
        assert_eq!(w.markers.end, MarkerKind::Arrow);
    }

    #[test]
    fn wire_op_default_markers() {
        // `<->` defaults to arrow on both ends.
        let p = resolve_str(
            "scene { a :rect \"A\"\n b :rect \"B\" }\n\
             wires { a <-> b }\n",
        );
        let w = &p.wires[0];
        assert_eq!(w.markers.start, MarkerKind::Arrow);
        assert_eq!(w.markers.end, MarkerKind::Arrow);
    }

    #[test]
    fn defaults_override_layout_var_keeps_kind_and_bakes_value() {
        let p = resolve_str("defaults { gap=30 }\nscene { :rect \"x\" }\n");
        let entry = p.vars.get("gap").expect("gap present");
        assert_eq!(entry.kind, VarKind::Layout);
        match &entry.value {
            ResolvedValue::Number(n) => assert_eq!(*n, 30.0),
            other => panic!("expected Number(30), got {:?}", other),
        }
    }

    #[test]
    fn var_ref_to_layout_var_carries_baked_value() {
        // A scene attr that references a layout var should resolve to LiveVar
        // with the baked numeric value attached.
        let p = resolve_str(
            "defaults { gap=25 }\n\
             scene padding=var(gap) { :rect \"x\" }\n",
        );
        let padding = p.scene.attrs.get("padding").expect("padding attr");
        match padding {
            ResolvedValue::LiveVar { name, raw, baked } => {
                assert_eq!(name, "gap");
                assert!(!raw);
                match baked.as_deref() {
                    Some(ResolvedValue::Number(n)) => assert_eq!(*n, 25.0),
                    other => panic!("expected baked Number(25), got {:?}", other),
                }
            }
            other => panic!("expected LiveVar, got {:?}", other),
        }
    }

    #[test]
    fn var_ref_to_visual_var_has_no_baked() {
        let p = resolve_str("scene { a :rect \"X\" fill=var(accent) }\n");
        let a = &p.scene.nodes[0];
        let fill = a.attrs.get("fill").expect("fill attr");
        match fill {
            ResolvedValue::LiveVar { name, raw, baked } => {
                assert_eq!(name, "accent");
                assert!(!raw);
                assert!(baked.is_none(), "visual var should not be baked");
            }
            other => panic!("expected LiveVar, got {:?}", other),
        }
    }

    #[test]
    fn raw_css_var_passthrough() {
        let p = resolve_str("scene { a :rect \"X\" fill=var(--my-token) }\n");
        let a = &p.scene.nodes[0];
        let fill = a.attrs.get("fill").expect("fill attr");
        match fill {
            ResolvedValue::LiveVar { name, raw, .. } => {
                assert_eq!(name, "my-token");
                assert!(*raw);
            }
            other => panic!("expected LiveVar, got {:?}", other),
        }
    }

    #[test]
    fn label_sugar_creates_text_child_on_non_text_shape() {
        let p = resolve_str("scene { :rect \"hello\" }\n");
        let r = &p.scene.nodes[0];
        assert_eq!(r.shape, ShapeKind::Rect);
        assert!(r.label.is_none(), "non-text shape keeps no label");
        assert_eq!(r.children.len(), 1);
        let t = &r.children[0];
        assert_eq!(t.shape, ShapeKind::Text);
        assert_eq!(t.label.as_deref(), Some("hello"));
    }

    #[test]
    fn text_label_stays_on_text_inst() {
        let p = resolve_str("scene { :text \"hello\" }\n");
        let t = &p.scene.nodes[0];
        assert_eq!(t.shape, ShapeKind::Text);
        assert_eq!(t.label.as_deref(), Some("hello"));
        assert!(t.children.is_empty());
    }

    #[test]
    fn shape_inheritance_resolves_to_primitive_kind() {
        let p = resolve_str("shapes { psu :rect radius=5 }\nscene { :psu \"PSU\" }\n");
        let n = &p.scene.nodes[0];
        assert_eq!(n.shape, ShapeKind::Rect);
        // The shape's own attrs are layered onto the inst's attrs.
        assert!(n.attrs.get("radius").is_some());
    }

    #[test]
    fn style_composition_expands_attrs_in_order() {
        let p = resolve_str(
            "styles {\n  base thickness=1 stroke=#444\n  warn .base stroke=orange\n}\n\
             scene { :rect \"x\" .warn }\n",
        );
        let n = &p.scene.nodes[0];
        // .base then .warn → orange wins.
        match n.attrs.get("stroke") {
            Some(ResolvedValue::Ident(s)) => assert_eq!(s, "orange"),
            other => panic!("expected stroke=orange, got {:?}", other),
        }
        // .base contributes thickness=1.
        assert!(n.attrs.get("thickness").is_some());
    }
}
