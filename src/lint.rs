//! AST-level lint pass. Emits warnings for stylistic smells that aren't
//! parse/resolve errors — most notably inline visual attrs that belong in
//! `styles {}` (SPEC §15 visual-attr table).

use crate::ast::{AttrItem, Block, File, ShapeInst, TypeRef, WireDecl};
use crate::error::Diagnostic;

/// Attrs that are purely visual — appearance only, not what's drawn or where.
/// Inline use outside `styles {}` emits a warning.
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
    for block in &file.blocks {
        match block {
            Block::Scene(s) => {
                for inst in &s.body {
                    lint_inst(inst, &mut diags);
                }
            }
            Block::Wires(w) => {
                for wire in &w.wires {
                    lint_wire(wire, &mut diags);
                }
            }
            Block::Shapes(sh) => {
                // Shape *definitions* set defaults that act like styles —
                // visual attrs there are fine. But shape *bodies* contain
                // primitives that should follow scene rules.
                for shape in &sh.shapes {
                    if let Some(body) = &shape.body {
                        for inst in body {
                            lint_inst(inst, &mut diags);
                        }
                    }
                }
            }
            Block::Defaults(_) | Block::Styles(_) => {}
        }
    }
    diags
}

fn lint_inst(inst: &ShapeInst, diags: &mut Vec<Diagnostic>) {
    for item in &inst.items {
        if let AttrItem::Attr(a) = item {
            if is_visual(&a.name, &inst.ty) {
                diags.push(Diagnostic::warn(
                    a.span,
                    format!(
                        "visual attr '{}' inline; consider moving to styles {{}}",
                        a.name
                    ),
                ));
            }
        }
    }
    if let Some(body) = &inst.body {
        for child in body {
            lint_inst(child, diags);
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
                        "visual attr '{}' inline; consider moving to styles {{}}",
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
