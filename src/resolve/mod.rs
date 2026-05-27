mod ir;
mod shapes;
mod styles;
mod vars;

pub use ir::*;

use crate::ast::{
    AttrItem, BodyItem, DefsBlock, DefsEntry, EndpointGroup, File, LineStyle, SceneConfig,
    ShapeDef, ShapeInst, Side, StyleDef, TypeDefaults, TypeRef, VarOverride, WireConfig, WireDecl,
    WireEndpoint, WireMarker, WireOp,
};
use crate::error::Error;
use crate::span::Span;
use std::collections::HashMap;

#[allow(dead_code)]
pub fn resolve(file: File) -> Result<Program, Error> {
    resolve_with_theme(file, &[])
}

pub fn resolve_with_theme(file: File, theme: &[(String, String)]) -> Result<Program, Error> {
    // ─── Phase 2.1 — vars & defs setup ───
    let mut vars = vars::built_in_defaults();
    vars::apply_theme(&mut vars, theme);

    let split = split_defs(&file.defs);

    if !split.var_overrides.is_empty() {
        vars::apply_var_overrides(&mut vars, &split.var_overrides)?;
    }

    let styles_table = styles::StyleTable::build(&split.style_defs, &vars)?;
    let shapes_table = shapes::ShapesTable::build(
        &split.shape_defs,
        &split.type_defaults,
        &styles_table,
        &vars,
    )?;

    // ─── Phase 2.2 — partition top-level stmts ───
    let (root_nodes, root_wires) = partition_stmts(&file.stmts);

    // ─── Phase 2.3 — collect referenced endpoint ids, auto-create unknown ones ───
    // SPEC §5: a wire endpoint referencing an undeclared id auto-creates an
    // empty |rect| at scene root with `label = id`.
    let declared_ids = collect_declared_ids(&root_nodes);
    let mut auto_created: Vec<ShapeInst> = Vec::new();
    let mut auto_seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    for wire in &root_wires {
        collect_auto_created(wire, &declared_ids, &mut auto_seen, &mut auto_created);
    }

    // ─── Phase 2.4 — resolve scene tree ───
    // Apply scene config to root scene attrs.
    let scene_attrs = match split.scene_config {
        Some(cfg) => {
            let resolved = resolve_attrs(&cfg.items, &styles_table, &vars)?;
            collapse(&resolved)
        }
        None => default_scene_attrs(&vars),
    };

    let mut id_seen: HashMap<String, Span> = HashMap::new();
    let mut scene_nodes = Vec::new();
    let mut internal_wires_lifted: Vec<LiftedWire> = Vec::new();

    for inst in &root_nodes {
        let resolved = resolve_inst(
            inst,
            &shapes_table,
            &styles_table,
            &vars,
            &mut id_seen,
            &[],
            &mut internal_wires_lifted,
        )?;
        scene_nodes.push(resolved);
    }
    for inst in &auto_created {
        let resolved = resolve_inst(
            inst,
            &shapes_table,
            &styles_table,
            &vars,
            &mut id_seen,
            &[],
            &mut internal_wires_lifted,
        )?;
        scene_nodes.push(resolved);
    }

    // ─── Phase 2.5 — build the dot-path → node lookup ───
    // Suffix-match against this map when resolving wire endpoints.
    let path_index = build_path_index(&scene_nodes);

    // ─── Phase 2.6 — resolve wires (root + lifted internal) ───
    // Pre-resolve |wire| defaults once — layered as lowest specificity under
    // styles and per-wire attrs.
    let wires_defaults = match split.wire_config {
        Some(cfg) => resolve_attrs(&cfg.items, &styles_table, &vars)?,
        None => Vec::new(),
    };
    let mut wires = Vec::new();
    for w in &root_wires {
        for resolved in resolve_wire(w, &styles_table, &vars, &path_index, &[], &wires_defaults)? {
            wires.push(resolved);
        }
    }
    for lifted in &internal_wires_lifted {
        for resolved in resolve_wire(
            &lifted.wire,
            &styles_table,
            &vars,
            &path_index,
            &lifted.prefix,
            &wires_defaults,
        )? {
            wires.push(resolved);
        }
    }

    Ok(Program {
        vars,
        scene: ResolvedScene {
            attrs: scene_attrs,
            nodes: scene_nodes,
        },
        wires,
    })
}

// ─────────────────────────── Defs partitioning ───────────────────────────

struct SplitDefs<'a> {
    scene_config: Option<&'a SceneConfig>,
    wire_config: Option<&'a WireConfig>,
    type_defaults: Vec<&'a TypeDefaults>,
    var_overrides: Vec<&'a VarOverride>,
    style_defs: Vec<&'a StyleDef>,
    shape_defs: Vec<&'a ShapeDef>,
}

fn split_defs(defs: &Option<DefsBlock>) -> SplitDefs<'_> {
    let mut scene_config = None;
    let mut wire_config = None;
    let mut type_defaults = Vec::new();
    let mut var_overrides = Vec::new();
    let mut style_defs = Vec::new();
    let mut shape_defs = Vec::new();
    if let Some(block) = defs {
        for entry in &block.entries {
            match entry {
                DefsEntry::SceneConfig(s) => scene_config = Some(s),
                DefsEntry::WireConfig(w) => wire_config = Some(w),
                DefsEntry::TypeDefaults(t) => type_defaults.push(t),
                DefsEntry::VarOverride(v) => var_overrides.push(v),
                DefsEntry::StyleDef(s) => style_defs.push(s),
                DefsEntry::ShapeDef(s) => shape_defs.push(s),
            }
        }
    }
    SplitDefs {
        scene_config,
        wire_config,
        type_defaults,
        var_overrides,
        style_defs,
        shape_defs,
    }
}

fn default_scene_attrs(vars: &VarTable) -> AttrMap {
    // SPEC §6 default when |scene| is omitted: `layout:row gap:--gap padding:--canvas-pad`.
    let mut m = AttrMap::new();
    m.insert("layout", ResolvedValue::Ident("row".into()));
    if let Some(e) = vars.get("gap") {
        m.insert(
            "gap",
            ResolvedValue::LiveVar {
                name: "gap".into(),
                raw: false,
                baked: Some(Box::new(e.value.clone())),
            },
        );
    }
    if let Some(e) = vars.get("canvas-pad") {
        m.insert(
            "padding",
            ResolvedValue::LiveVar {
                name: "canvas-pad".into(),
                raw: false,
                baked: Some(Box::new(e.value.clone())),
            },
        );
    }
    m
}

// ─────────────────────────── Stmt partitioning ───────────────────────────

fn partition_stmts(stmts: &[crate::ast::Stmt]) -> (Vec<ShapeInst>, Vec<WireDecl>) {
    let mut nodes = Vec::new();
    let mut wires = Vec::new();
    for s in stmts {
        match s {
            crate::ast::Stmt::Node(n) => nodes.push(n.clone()),
            crate::ast::Stmt::Wire(w) => wires.push(w.clone()),
        }
    }
    (nodes, wires)
}

// ─────────────────────────── Auto-create ───────────────────────────

fn collect_declared_ids(nodes: &[ShapeInst]) -> std::collections::HashSet<String> {
    let mut out = std::collections::HashSet::new();
    for n in nodes {
        walk_collect_ids(n, &mut out);
    }
    out
}

fn walk_collect_ids(inst: &ShapeInst, out: &mut std::collections::HashSet<String>) {
    if let Some(id) = &inst.id {
        out.insert(id.clone());
    }
    if let Some(body) = &inst.body {
        for item in body {
            if let BodyItem::Inst(child) = item {
                walk_collect_ids(child, out);
            }
        }
    }
}

fn collect_auto_created(
    wire: &WireDecl,
    declared: &std::collections::HashSet<String>,
    seen: &mut std::collections::HashSet<String>,
    out: &mut Vec<ShapeInst>,
) {
    for group in &wire.chain {
        for ep in &group.endpoints {
            // Multi-segment paths are dot-path navigations; only auto-create
            // for unqualified single-segment endpoints whose root is unknown.
            if ep.path.len() != 1 {
                continue;
            }
            let id = &ep.path[0];
            if declared.contains(id) || seen.contains(id) {
                continue;
            }
            seen.insert(id.clone());
            out.push(auto_created_inst(id, ep.span));
        }
    }
}

fn auto_created_inst(id: &str, span: Span) -> ShapeInst {
    ShapeInst {
        id: Some(id.to_string()),
        ty: TypeRef {
            name: "rect".to_string(),
            span,
        },
        label: Some(id.to_string()),
        href: None,
        items: Vec::new(),
        body: None,
        span,
    }
}

// ─────────────────────────── Reserved names ───────────────────────────

pub(super) fn is_reserved(name: &str) -> bool {
    matches!(
        name,
        // Layout values
        "row" | "column" | "grid"
        | "start" | "center" | "end" | "stretch" | "between" | "around" | "evenly"
        // Anchors
        | "top" | "bottom" | "left" | "right"
        | "top-left" | "top-right" | "bottom-left" | "bottom-right"
        | "out-top" | "out-bottom" | "out-left" | "out-right"
        | "out-top-left" | "out-top-right" | "out-bottom-left" | "out-bottom-right"
        | "mid"
        // Endpoint sides (short)
        | "t" | "b" | "l" | "r"
        // Primitives
        | "rect" | "oval" | "line" | "path" | "poly" | "text"
        | "hex" | "slant" | "cyl" | "diamond" | "cloud" | "icon" | "image"
        // Templates
        | "group" | "badge" | "button" | "card" | "note"
        | "table" | "cell"
        // Defs-only specials
        | "scene" | "wire"
        // Constants
        | "true" | "false" | "none" | "auto"
        // Functions
        | "var" | "rgb" | "rgba" | "hsl"
    )
}

// ─────────────────────────── Attr resolution ───────────────────────────

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

// ─────────────────────────── Markers ───────────────────────────

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

fn default_markers_for_shape(_kind: ShapeKind) -> Markers {
    // No primitive carries default markers anymore; an "arrow" is just a
    // `|line| marker-end:arrow`. Wires get their defaults from the operator
    // (see `op_markers`).
    Markers::default()
}

fn op_markers(op: WireOp) -> Markers {
    Markers {
        start: MarkerKind::from_marker(op.start),
        end: MarkerKind::from_marker(op.end),
    }
}

// ─────────────────────────── Scene tree resolution ───────────────────────────

/// One internal wire (from a shape def body) lifted up to the program level
/// after instantiation, with its endpoint paths prefixed by the instance path.
struct LiftedWire {
    wire: WireDecl,
    /// Dot-path of the host instance (e.g. ["garden"]) — gets prefixed onto
    /// every endpoint path inside the wire at resolution time.
    prefix: Vec<String>,
}

#[allow(clippy::too_many_arguments)]
fn resolve_inst(
    inst: &ShapeInst,
    shapes: &shapes::ShapesTable,
    styles_table: &styles::StyleTable,
    vars: &VarTable,
    id_seen: &mut HashMap<String, Span>,
    path_prefix: &[String],
    lifted: &mut Vec<LiftedWire>,
) -> Result<ResolvedInst, Error> {
    let resolved_shape = shapes.resolve(&inst.ty.name, inst.ty.span)?;

    let applied_styles: Vec<String> = inst
        .items
        .iter()
        .filter_map(|i| match i {
            AttrItem::Style(s) => Some(s.name.clone()),
            AttrItem::Attr(_) => None,
        })
        .collect();

    // ID uniqueness + reserved-name check. Only check root-level ids globally;
    // shape-body instantiation may legitimately have the same local id across
    // multiple insts.
    if let Some(id) = &inst.id {
        if is_reserved(id) {
            return Err(Error::at(inst.span, format!("'{}' is reserved", id)));
        }
        if path_prefix.is_empty() && id_seen.contains_key(id) {
            return Err(Error::at(inst.span, format!("duplicate scene id '{}'", id)));
        }
        if path_prefix.is_empty() {
            id_seen.insert(id.clone(), inst.span);
        }
    }

    let inline = resolve_attrs(&inst.items, styles_table, vars)?;
    let mut ordered = resolved_shape.attrs.clone();
    ordered.extend(inline);

    let defaults = default_markers_for_shape(resolved_shape.kind);
    let markers = resolve_markers(&ordered, defaults.start, defaults.end)?;
    let attrs = collapse(&ordered);

    // Compute the dot-path of this inst for nested children.
    let mut child_prefix = path_prefix.to_vec();
    if let Some(id) = &inst.id {
        child_prefix.push(id.clone());
    }

    // Body assembly: shape-def intrinsic children, then label sugar (non-text),
    // then explicit body items from the source.
    let mut body_items: Vec<BodyItem> = resolved_shape.body_items.clone();
    let own_label = if resolved_shape.kind == ShapeKind::Text {
        inst.label.clone()
    } else {
        if let Some(label) = &inst.label {
            body_items.push(BodyItem::Inst(label_sugar_text(label, inst.span)));
        }
        None
    };
    if let Some(b) = &inst.body {
        body_items.extend(b.iter().cloned());
    }

    let mut children = Vec::new();
    for item in &body_items {
        match item {
            BodyItem::Inst(child) => {
                children.push(resolve_inst(
                    child,
                    shapes,
                    styles_table,
                    vars,
                    id_seen,
                    &child_prefix,
                    lifted,
                )?);
            }
            BodyItem::Wire(wire) => {
                lifted.push(LiftedWire {
                    wire: wire.clone(),
                    prefix: child_prefix.clone(),
                });
            }
        }
    }

    Ok(ResolvedInst {
        id: inst.id.clone(),
        shape: resolved_shape.kind,
        type_chain: resolved_shape.type_chain,
        applied_styles,
        label: own_label,
        href: inst.href.clone(),
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
        href: None,
        items: Vec::new(),
        body: None,
        span,
    }
}

// ─────────────────────────── Path index ───────────────────────────

/// Maps fully-qualified dot-paths to their place in the scene tree.
struct PathIndex {
    paths: Vec<String>,
}

impl PathIndex {
    fn contains(&self, path: &str) -> bool {
        self.paths.iter().any(|p| p == path)
    }

    /// SPEC §10 suffix-match: find the unique scene-tree path whose tail
    /// segments equal `query`'s segments. Returns the full canonical path.
    fn resolve(&self, query: &[String]) -> Result<String, EndpointMatch> {
        let qjoined = query.join(".");
        // Exact full path match wins immediately.
        if self.contains(&qjoined) {
            return Ok(qjoined);
        }
        let mut hits: Vec<&String> = Vec::new();
        for p in &self.paths {
            if path_ends_with(p, query) {
                hits.push(p);
            }
        }
        match hits.len() {
            0 => Err(EndpointMatch::NotFound),
            1 => Ok(hits[0].clone()),
            _ => Err(EndpointMatch::Ambiguous(
                hits.into_iter().cloned().collect(),
            )),
        }
    }
}

enum EndpointMatch {
    NotFound,
    Ambiguous(Vec<String>),
}

fn path_ends_with(path: &str, query: &[String]) -> bool {
    let segs: Vec<&str> = path.split('.').collect();
    if query.len() > segs.len() {
        return false;
    }
    let tail = &segs[segs.len() - query.len()..];
    tail.iter().zip(query.iter()).all(|(a, b)| *a == b)
}

fn build_path_index(nodes: &[ResolvedInst]) -> PathIndex {
    let mut paths = Vec::new();
    for n in nodes {
        walk_paths(n, &mut Vec::new(), &mut paths);
    }
    PathIndex { paths }
}

fn walk_paths(n: &ResolvedInst, stack: &mut Vec<String>, out: &mut Vec<String>) {
    if let Some(id) = &n.id {
        stack.push(id.clone());
        out.push(stack.join("."));
    }
    for c in &n.children {
        walk_paths(c, stack, out);
    }
    if n.id.is_some() {
        stack.pop();
    }
}

// ─────────────────────────── Wires ───────────────────────────

fn resolve_wire(
    w: &WireDecl,
    styles_table: &styles::StyleTable,
    vars: &VarTable,
    paths: &PathIndex,
    path_prefix: &[String],
    wires_defaults: &[ResolvedAttr],
) -> Result<Vec<ResolvedWire>, Error> {
    let inline = resolve_attrs(&w.items, styles_table, vars)?;
    // SPEC §13 application order: `|wire|` defaults are lowest specificity,
    // styles and per-wire attrs override (the latter are already merged into
    // `inline` left-to-right by `resolve_attrs`).
    let mut ordered: Vec<ResolvedAttr> = Vec::with_capacity(wires_defaults.len() + inline.len());
    ordered.extend(wires_defaults.iter().cloned());
    ordered.extend(inline);

    let op_marks = op_markers(w.op);
    let markers = resolve_markers(&ordered, op_marks.start, op_marks.end)?;
    let mut attrs = collapse(&ordered);

    // Synthesize stroke-style for line variants per SPEC §10.
    inject_line_style(&mut attrs, w.op.line);

    // Text children: label sugar + explicit body texts.
    let mut texts: Vec<ResolvedText> = Vec::new();
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
            let t_attrs = resolve_attrs(&t.items, styles_table, vars)?;
            let mut at = WireAt::Mid;
            let mut t_map = AttrMap::new();
            for item in &t_attrs {
                if item.name == "at" {
                    if let Some(v) = &item.value {
                        at = WireAt::parse(v).ok_or_else(|| {
                            Error::at(
                                item.span,
                                "|text| anchor on a wire must be start/mid/end or 0..1",
                            )
                        })?;
                    }
                } else {
                    let value = item.value.clone().unwrap_or_else(|| {
                        bare_attr_default(&item.name).unwrap_or(ResolvedValue::Ident("true".into()))
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

    // Cartesian fan expansion: each group's endpoints fan out independently.
    // For chain [{a}, {b,c}, {d}] with op `->`, we get a→b→d, a→c→d (each as
    // its own wire). Per spec §10 wire fan grammar.
    let expanded = expand_chain(&w.chain);

    let mut out = Vec::with_capacity(expanded.len());
    for chain_path in expanded {
        let mut endpoints = Vec::with_capacity(chain_path.len());
        for ep in chain_path {
            let qualified: Vec<String> = if path_prefix.is_empty() {
                ep.path.clone()
            } else {
                // For internal wires lifted from a shape body, prefix the
                // endpoint with the host inst's id-path before resolution.
                let mut p = path_prefix.to_vec();
                p.extend(ep.path.iter().cloned());
                p
            };
            let resolved_path = match paths.resolve(&qualified) {
                Ok(p) => p,
                Err(EndpointMatch::NotFound) => {
                    return Err(Error::at(
                        ep.span,
                        format!("wire endpoint '{}' not found", qualified.join(".")),
                    ));
                }
                Err(EndpointMatch::Ambiguous(hits)) => {
                    return Err(Error::at(
                        ep.span,
                        format!(
                            "endpoint '{}' is ambiguous (matches: {}); qualify with full path",
                            qualified.join("."),
                            hits.join(", ")
                        ),
                    ));
                }
            };
            endpoints.push(ResolvedEndpoint {
                path: resolved_path,
                side: ep.side,
                span: ep.span,
            });
        }
        out.push(ResolvedWire {
            endpoints,
            line: w.op.line,
            attrs: attrs.clone(),
            markers: markers.clone(),
            texts: texts.iter().map(clone_text).collect(),
            span: w.span,
        });
    }
    Ok(out)
}

fn clone_text(t: &ResolvedText) -> ResolvedText {
    ResolvedText {
        text: t.text.clone(),
        at: t.at.clone(),
        attrs: t.attrs.clone(),
        span: t.span,
    }
}

fn inject_line_style(attrs: &mut AttrMap, line: LineStyle) {
    let style = match line {
        LineStyle::Solid => return,
        LineStyle::Dashed => "dashed",
        LineStyle::Dotted => "dotted",
        // double / wavy aren't first-class in the renderer yet — treat as solid
        // visually but tag them so render can branch later.
        LineStyle::Double => "double",
        LineStyle::Wavy => "wavy",
    };
    // Don't override an explicit stroke-style attr.
    if attrs.get("stroke-style").is_none() {
        attrs.insert("stroke-style", ResolvedValue::Ident(style.into()));
    }
}

/// Take a wire chain and expand the cartesian fan-out across endpoint groups.
/// Result: each entry is one fully-flattened endpoint sequence (one wire).
fn expand_chain(chain: &[EndpointGroup]) -> Vec<Vec<WireEndpoint>> {
    let mut acc: Vec<Vec<WireEndpoint>> = vec![Vec::new()];
    for group in chain {
        let mut next: Vec<Vec<WireEndpoint>> =
            Vec::with_capacity(acc.len() * group.endpoints.len());
        for trail in &acc {
            for ep in &group.endpoints {
                let mut t = trail.clone();
                t.push(ep.clone());
                next.push(t);
            }
        }
        acc = next;
    }
    acc
}

// `_` to satisfy unused warning when this is only referenced indirectly.
#[allow(dead_code)]
fn _coerce_unused(_: &Side, _: &WireMarker) {}

// ─────────────────────────── Tests ───────────────────────────

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
        let p = resolve_str(
            "cat |rect| \"Cat\"\n\
             dog |rect| \"Dog\"\n\
             cat -> dog marker:arrow marker-end:dot\n",
        );
        let w = &p.wires[0];
        assert_eq!(w.markers.start, MarkerKind::Arrow);
        assert_eq!(w.markers.end, MarkerKind::Dot);
    }

    #[test]
    fn wire_op_default_markers() {
        let p = resolve_str(
            "cat |rect| \"Cat\"\n\
             dog |rect| \"Dog\"\n\
             cat <-> dog\n",
        );
        let w = &p.wires[0];
        assert_eq!(w.markers.start, MarkerKind::Arrow);
        assert_eq!(w.markers.end, MarkerKind::Arrow);
    }

    #[test]
    fn defaults_override_layout_var_keeps_kind_and_bakes_value() {
        let p = resolve_str("{ --gap:30 }\nx |rect|\n");
        let entry = p.vars.get("gap").expect("gap present");
        assert_eq!(entry.kind, VarKind::Layout);
        match &entry.value {
            ResolvedValue::Number(n) => assert_eq!(*n, 30.0),
            other => panic!("expected Number(30), got {:?}", other),
        }
    }

    #[test]
    fn label_sugar_creates_text_child_on_non_text_shape() {
        let p = resolve_str("cat |rect| \"hello\"\n");
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
        let p = resolve_str("cat |text| \"hello\"\n");
        let t = &p.scene.nodes[0];
        assert_eq!(t.shape, ShapeKind::Text);
        assert_eq!(t.label.as_deref(), Some("hello"));
        assert!(t.children.is_empty());
    }

    #[test]
    fn shape_inheritance_resolves_to_primitive_kind() {
        let p = resolve_str("{ |treat:rect| radius:5 }\ncat |treat| \"Cat\"\n");
        let n = &p.scene.nodes[0];
        assert_eq!(n.shape, ShapeKind::Rect);
        assert!(n.attrs.get("radius").is_some());
    }

    #[test]
    fn wire_auto_creates_undeclared_endpoints() {
        let p = resolve_str("cat -> dog\n");
        // Both `cat` and `dog` auto-created as rects.
        assert_eq!(p.scene.nodes.len(), 2);
        let ids: Vec<&str> = p
            .scene
            .nodes
            .iter()
            .filter_map(|n| n.id.as_deref())
            .collect();
        assert!(ids.contains(&"cat"));
        assert!(ids.contains(&"dog"));
    }

    #[test]
    fn wire_fan_expands_cartesian() {
        let p = resolve_str("cat & fox -> bird & mouse\n");
        assert_eq!(p.wires.len(), 4);
    }
}
