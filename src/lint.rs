//! AST-level lint pass. Emits warnings for stylistic smells that aren't
//! parse/resolve errors — most notably inline visual attrs that belong in a
//! `.style` def (SPEC section 16 visual-attr lint category).

use crate::ast::{AttrItem, BodyItem, DefsEntry, File, ShapeInst, Stmt, TypeRef, WireDecl};
use crate::error::Diagnostic;

/// Attrs that are purely visual — appearance only, not what's drawn or where.
/// Inline use outside a style def emits a warning.
const VISUAL_ATTRS: &[&str] = &[
    "fill",
    "stroke",
    "thickness",
    "stroke-style",
    "opacity",
    "radius",
    "double",
    "rotation",
    "shadow",
    "weight",
    "align",
    "fit",
    "variant",
];

/// `size` is visual on text nodes but structural on icons; check by type.
fn size_is_visual(ty: &TypeRef) -> bool {
    ty.name == "text"
}

pub fn lint(file: &File) -> Vec<Diagnostic> {
    let mut diags = Vec::new();
    for stmt in &file.stmts {
        match stmt {
            Stmt::Node(inst) => lint_inst(inst, &mut diags),
            Stmt::Wire(w) => lint_wire(w, &mut diags),
        }
    }
    // Shape defs in the defs block can contain bodies with primitives that
    // should follow scene rules.
    if let Some(defs) = &file.defs {
        for entry in &defs.entries {
            if let DefsEntry::ShapeDef(sd) = entry {
                if let Some(body) = &sd.body {
                    for item in body {
                        lint_body_item(item, &mut diags);
                    }
                }
            }
        }
    }
    diags
}

fn lint_body_item(item: &BodyItem, diags: &mut Vec<Diagnostic>) {
    match item {
        BodyItem::Inst(i) => lint_inst(i, diags),
        BodyItem::Wire(w) => lint_wire(w, diags),
    }
}

fn lint_inst(inst: &ShapeInst, diags: &mut Vec<Diagnostic>) {
    for item in &inst.items {
        if let AttrItem::Attr(a) = item {
            if is_visual(&a.name, &inst.ty) {
                diags.push(Diagnostic::warn(
                    a.span,
                    format!(
                        "visual attr '{}' inline; consider moving to a .style",
                        a.name
                    ),
                ));
            }
        }
    }
    if let Some(body) = &inst.body {
        for child in body {
            lint_body_item(child, diags);
        }
    }
}

fn lint_wire(wire: &WireDecl, diags: &mut Vec<Diagnostic>) {
    for item in &wire.items {
        if let AttrItem::Attr(a) = item {
            // Marker attrs are structural on wires; everything else in
            // VISUAL_ATTRS is style.
            if VISUAL_ATTRS.contains(&a.name.as_str()) {
                diags.push(Diagnostic::warn(
                    a.span,
                    format!(
                        "visual attr '{}' inline; consider moving to a .style",
                        a.name
                    ),
                ));
            }
        }
    }
}

fn is_visual(name: &str, ty: &TypeRef) -> bool {
    if VISUAL_ATTRS.contains(&name) {
        return true;
    }
    if name == "size" && size_is_visual(ty) {
        return true;
    }
    false
}
