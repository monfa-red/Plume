//! Per-shape bbox computation. Closed shapes with text-only children auto-size
//! to the text plus padding (or text-pad if no padding is set). Container
//! shapes get their bbox from already-laid-out children plus padding.

use super::ir::Bbox;
use super::text;
use super::values::{as_pair, expand_box_value, layout_var};
use crate::error::Error;
use crate::resolve::{AttrMap, ResolvedInst, ResolvedValue, ShapeKind, VarTable};
use crate::span::Span;

#[derive(Default, Clone, Copy)]
pub struct PaddingBox {
    pub top: f64,
    pub right: f64,
    pub bottom: f64,
    pub left: f64,
}

impl PaddingBox {
    pub fn uniform(n: f64) -> Self {
        Self {
            top: n,
            right: n,
            bottom: n,
            left: n,
        }
    }
}

/// Bbox for a leaf primitive (no children — purely shape-driven dimensions).
pub fn leaf_bbox(inst: &ResolvedInst, vars: &VarTable) -> Result<Bbox, Error> {
    let bbox = geom_bbox(inst, vars)?;
    Ok(bbox.inflate(stroke_half(inst, vars)))
}

/// Bbox for a closed shape that has been auto-sized to its content (text or
/// nested children) plus padding.
pub fn auto_sized_bbox(
    inst: &ResolvedInst,
    content_bbox: Bbox,
    vars: &VarTable,
    use_text_pad: bool,
) -> Result<Bbox, Error> {
    let pad = if use_text_pad && !has_padding_attr(&inst.attrs) {
        PaddingBox::uniform(layout_var(vars, "text-pad").unwrap_or(16.0))
    } else {
        padding(&inst.attrs, vars, inst.span)?
    };
    let w = content_bbox.w() + pad.left + pad.right;
    let h = content_bbox.h() + pad.top + pad.bottom;
    let bbox = Bbox::centered(w, h);
    Ok(bbox.inflate(stroke_half(inst, vars)))
}

fn has_padding_attr(attrs: &AttrMap) -> bool {
    attrs.get("padding").is_some()
}

pub fn padding(attrs: &AttrMap, vars: &VarTable, span: Span) -> Result<PaddingBox, Error> {
    if let Some(v) = attrs.get("padding") {
        let (t, r, b, l) = expand_box_value(v, span)?;
        Ok(PaddingBox {
            top: t,
            right: r,
            bottom: b,
            left: l,
        })
    } else {
        Ok(PaddingBox::uniform(
            layout_var(vars, "padding").unwrap_or(0.0),
        ))
    }
}

pub fn gap(attrs: &AttrMap, vars: &VarTable, span: Span) -> Result<(f64, f64), Error> {
    // gap → (y_between_rows, x_between_cols). Scalar collapses to both equal;
    // (y, x) takes the form directly.
    if let Some(v) = attrs.get("gap") {
        let nums = super::values::as_number_tuple(v, span)?;
        Ok(match nums.len() {
            1 => (nums[0], nums[0]),
            2 => (nums[0], nums[1]),
            n => {
                return Err(Error::at(
                    span,
                    format!("'gap' expects 1 or 2 values, got {}", n),
                ));
            }
        })
    } else {
        let g = layout_var(vars, "gap").unwrap_or(20.0);
        Ok((g, g))
    }
}

// ───────────────────────── Internal bbox computation ─────────────────────────

fn geom_bbox(inst: &ResolvedInst, vars: &VarTable) -> Result<Bbox, Error> {
    let attrs = &inst.attrs;
    match inst.shape {
        ShapeKind::Rect
        | ShapeKind::Slant
        | ShapeKind::Cyl
        | ShapeKind::Diamond
        | ShapeKind::Cloud
        | ShapeKind::Hex => {
            let (w, h) = closed_shape_dims(inst, vars)?;
            Ok(Bbox::centered(w, h))
        }
        ShapeKind::Oval => {
            // :circle template sugars `r=N` → rx=ry=N. We accept either form.
            let r = attr_num(attrs, "r");
            let rx = r
                .or_else(|| attr_num(attrs, "rx"))
                .or_else(|| layout_var(vars, "oval-rx"))
                .unwrap_or(30.0);
            let ry = r
                .or_else(|| attr_num(attrs, "ry"))
                .or_else(|| layout_var(vars, "oval-ry"))
                .unwrap_or(20.0);
            Ok(Bbox::centered(rx * 2.0, ry * 2.0))
        }
        ShapeKind::Text => {
            let size = attr_num(attrs, "size")
                .or_else(|| layout_var(vars, "text-size"))
                .unwrap_or(13.0);
            let label = inst.label.as_deref().unwrap_or("");
            let w = text::approx_width(label, size);
            let h = text::approx_height(label, size);
            Ok(Bbox::centered(w, h))
        }
        ShapeKind::Line | ShapeKind::Arrow => {
            let from = attr_pair(attrs, "from", inst.span)?.ok_or_else(|| {
                Error::at(
                    inst.span,
                    format!("':{}' requires 'from'", inst.shape.as_str()),
                )
            })?;
            let to = attr_pair(attrs, "to", inst.span)?.ok_or_else(|| {
                Error::at(
                    inst.span,
                    format!("':{}' requires 'to'", inst.shape.as_str()),
                )
            })?;
            Ok(Bbox {
                min_x: from.0.min(to.0),
                min_y: from.1.min(to.1),
                max_x: from.0.max(to.0),
                max_y: from.1.max(to.1),
            })
        }
        ShapeKind::Icon => {
            let size = attr_num(attrs, "size")
                .or_else(|| layout_var(vars, "icon-size"))
                .unwrap_or(24.0);
            Ok(Bbox::centered(size, size))
        }
        ShapeKind::Image => {
            let w = attr_num(attrs, "w")
                .ok_or_else(|| Error::at(inst.span, "':image' requires 'w'"))?;
            let h = attr_num(attrs, "h")
                .ok_or_else(|| Error::at(inst.span, "':image' requires 'h'"))?;
            Ok(Bbox::centered(w, h))
        }
        ShapeKind::Poly => {
            let points = attr_points(attrs, "points", inst.span)?
                .ok_or_else(|| Error::at(inst.span, "':poly' requires 'points'"))?;
            if points.len() < 3 {
                return Err(Error::at(inst.span, "':poly' requires at least 3 points"));
            }
            let mut bb = Bbox {
                min_x: f64::INFINITY,
                min_y: f64::INFINITY,
                max_x: f64::NEG_INFINITY,
                max_y: f64::NEG_INFINITY,
            };
            for (x, y) in &points {
                bb.min_x = bb.min_x.min(*x);
                bb.min_y = bb.min_y.min(*y);
                bb.max_x = bb.max_x.max(*x);
                bb.max_y = bb.max_y.max(*y);
            }
            Ok(bb)
        }
        ShapeKind::Path => {
            // Native top-left coords (§6 rule 5). Real bbox needs SVG path
            // parsing; Sprint 3 returns a zero bbox and Sprint 5 will fill in.
            Ok(Bbox::empty())
        }
    }
}

fn closed_shape_dims(inst: &ResolvedInst, vars: &VarTable) -> Result<(f64, f64), Error> {
    let attrs = &inst.attrs;
    let w = attr_num(attrs, "w");
    let h = attr_num(attrs, "h");
    let (default_w, default_h) = match inst.shape {
        ShapeKind::Rect | ShapeKind::Slant => (
            layout_var(vars, "rect-w").unwrap_or(100.0),
            layout_var(vars, "rect-h").unwrap_or(40.0),
        ),
        ShapeKind::Hex | ShapeKind::Cyl | ShapeKind::Diamond | ShapeKind::Cloud => (60.0, 60.0),
        _ => (0.0, 0.0),
    };
    Ok((w.unwrap_or(default_w), h.unwrap_or(default_h)))
}

fn stroke_half(inst: &ResolvedInst, vars: &VarTable) -> f64 {
    let t = attr_num(&inst.attrs, "thickness")
        .or_else(|| layout_var(vars, "thickness"))
        .unwrap_or(1.0);
    t / 2.0
}

// ───────────────────────── Attr extraction helpers ─────────────────────────

fn attr_num(attrs: &AttrMap, name: &str) -> Option<f64> {
    attrs.get(name).and_then(extract_num)
}

fn extract_num(v: &ResolvedValue) -> Option<f64> {
    match v {
        ResolvedValue::Number(n) => Some(*n),
        ResolvedValue::LiveVar { baked: Some(b), .. } => extract_num(b),
        _ => None,
    }
}

fn attr_pair(attrs: &AttrMap, name: &str, span: Span) -> Result<Option<(f64, f64)>, Error> {
    match attrs.get(name) {
        Some(v) => Ok(Some(as_pair(v, span)?)),
        None => Ok(None),
    }
}

fn attr_points(attrs: &AttrMap, name: &str, span: Span) -> Result<Option<Vec<(f64, f64)>>, Error> {
    match attrs.get(name) {
        Some(ResolvedValue::List(items)) => {
            let mut out = Vec::with_capacity(items.len());
            for item in items {
                out.push(as_pair(item, span)?);
            }
            Ok(Some(out))
        }
        Some(_) => Err(Error::at(
            span,
            format!("'{}' expects a list of (x,y) tuples", name),
        )),
        None => Ok(None),
    }
}
