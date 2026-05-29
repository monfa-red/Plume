// Layout IR — populated bottom-up during phase 3. Fields are consumed by render
// (phase 5); allow dead_code at the module level while the pipeline fills in.
#![allow(dead_code)]

use crate::resolve::{AttrMap, Markers, ShapeKind, VarTable};
use crate::span::Span;

pub struct LaidOut {
    pub viewbox: ViewBox,
    pub scene_attrs: AttrMap,
    pub nodes: Vec<PlacedNode>,
    /// Resolved CSS variables — carried through to render so the `<style>`
    /// block and `--bake-vars` mode can both read them.
    pub vars: VarTable,
}

#[derive(Debug, Clone, Copy)]
pub struct ViewBox {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

#[derive(Clone)]
pub struct PlacedNode {
    pub id: Option<String>,
    pub shape: ShapeKind,
    pub type_chain: Vec<String>,
    pub applied_styles: Vec<String>,
    pub label: Option<String>,
    pub attrs: AttrMap,
    pub markers: Markers,
    /// Local origin position in parent coords.
    pub cx: f64,
    pub cy: f64,
    /// Bbox in local coords (relative to this node's own origin).
    pub bbox: Bbox,
    pub rotation: f64,
    pub children: Vec<PlacedNode>,
    pub span: Span,
}

#[derive(Debug, Clone, Copy)]
pub struct Bbox {
    pub min_x: f64,
    pub min_y: f64,
    pub max_x: f64,
    pub max_y: f64,
}

impl Bbox {
    pub fn empty() -> Self {
        Self {
            min_x: 0.0,
            min_y: 0.0,
            max_x: 0.0,
            max_y: 0.0,
        }
    }

    pub fn centered(w: f64, h: f64) -> Self {
        Self {
            min_x: -w / 2.0,
            min_y: -h / 2.0,
            max_x: w / 2.0,
            max_y: h / 2.0,
        }
    }

    pub fn w(&self) -> f64 {
        self.max_x - self.min_x
    }

    pub fn h(&self) -> f64 {
        self.max_y - self.min_y
    }

    /// Inflate by `pad` on every side.
    pub fn inflate(self, pad: f64) -> Self {
        Self {
            min_x: self.min_x - pad,
            min_y: self.min_y - pad,
            max_x: self.max_x + pad,
            max_y: self.max_y + pad,
        }
    }

    /// Union with another bbox already expressed in this frame.
    pub fn union(self, other: Bbox) -> Self {
        Self {
            min_x: self.min_x.min(other.min_x),
            min_y: self.min_y.min(other.min_y),
            max_x: self.max_x.max(other.max_x),
            max_y: self.max_y.max(other.max_y),
        }
    }

    /// Shift this bbox by (dx, dy). Useful when composing child bboxes into a
    /// parent's frame.
    pub fn shifted(self, dx: f64, dy: f64) -> Self {
        Self {
            min_x: self.min_x + dx,
            min_y: self.min_y + dy,
            max_x: self.max_x + dx,
            max_y: self.max_y + dy,
        }
    }
}
