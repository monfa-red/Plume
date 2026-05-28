//! Scene index: a flat-but-tree-aware lookup from a fully-qualified shape
//! path (the dot-path the resolver canonicalises) to its absolute bbox and
//! its chain of named ancestors. The ancestor chain is how we decide which
//! shapes count as obstacles for a given wire: a container that holds the
//! source or target of the wire is *passable* (the wire enters it), every
//! other shape is a *hard obstacle*.

use super::geometry::AbsBbox;
use crate::layout::ir::PlacedNode;
use crate::resolve::{AttrMap, ResolvedValue};
use std::collections::HashMap;

pub struct SceneIndex {
    nodes: Vec<IndexedNode>,
    by_path: HashMap<String, usize>,
}

struct IndexedNode {
    bbox: AbsBbox,
    /// Indices into `nodes` for every ancestor that has an id, root-first.
    ancestors: Vec<usize>,
    is_leaf: bool,
    /// Spacing this node enjoys from its layout siblings — driven by the
    /// parent's `gap` attr (or the scene's gap for top-level shapes).
    /// Wire clearance uses `max(wire_gap, clearance)` so a wire passing
    /// a shape never sits closer than the layout already spaces shapes.
    clearance: f64,
    /// Fully-qualified dot-path of this node (same key as `by_path`).
    path: String,
}

#[derive(Clone)]
pub struct ShapeRef {
    pub bbox: AbsBbox,
}

impl SceneIndex {
    pub fn build(scene_nodes: &[PlacedNode], scene_attrs: &AttrMap) -> Self {
        let mut idx = SceneIndex {
            nodes: Vec::new(),
            by_path: HashMap::new(),
        };
        let scene_gap = explicit_gap(scene_attrs).unwrap_or(0.0);
        for node in scene_nodes {
            idx.walk(node, 0.0, 0.0, &[], &mut Vec::new(), scene_gap);
        }
        idx
    }

    fn walk(
        &mut self,
        node: &PlacedNode,
        parent_cx: f64,
        parent_cy: f64,
        ancestors: &[usize],
        path_stack: &mut Vec<String>,
        clearance: f64,
    ) {
        let abs_cx = parent_cx + node.cx;
        let abs_cy = parent_cy + node.cy;
        let bbox = AbsBbox {
            x: abs_cx + node.bbox.min_x,
            y: abs_cy + node.bbox.min_y,
            w: node.bbox.w(),
            h: node.bbox.h(),
        };
        let mut next_ancestors = ancestors.to_vec();
        let mut pushed_path = false;
        if let Some(id) = &node.id {
            let i = self.nodes.len();
            path_stack.push(id.clone());
            let full_path = path_stack.join(".");
            pushed_path = true;
            self.nodes.push(IndexedNode {
                bbox,
                ancestors: ancestors.to_vec(),
                is_leaf: node.children.is_empty(),
                clearance,
                path: full_path.clone(),
            });
            self.by_path.insert(full_path, i);
            next_ancestors.push(i);
        }
        // Children inherit this node's `gap` attr as their clearance
        // (controls space between siblings in this container). Without
        // an explicit `gap`, fall through to the same clearance we
        // already have — a deeply-nested unspecified container shouldn't
        // tighten clearance below the scene-wide default.
        let child_clearance = explicit_gap(&node.attrs).unwrap_or(clearance);
        for child in &node.children {
            self.walk(
                child,
                abs_cx,
                abs_cy,
                &next_ancestors,
                path_stack,
                child_clearance,
            );
        }
        if pushed_path {
            path_stack.pop();
        }
    }

    pub fn lookup(&self, path: &str) -> Option<ShapeRef> {
        let i = *self.by_path.get(path)?;
        Some(ShapeRef {
            bbox: self.nodes[i].bbox,
        })
    }

    /// The clearance this shape inherits from its parent — wire routing
    /// uses `max(wire_gap, clearance)` for any wire passing this shape.
    pub fn clearance(&self, path: &str) -> Option<f64> {
        self.by_path
            .get(path)
            .copied()
            .map(|i| self.nodes[i].clearance)
    }

    /// Indices of nodes a wire between `src_id` and `tgt_id` may cross: the
    /// endpoints and all their named ancestors.
    fn passable_set(&self, src_id: &str, tgt_id: &str) -> Vec<usize> {
        let mut passable: Vec<usize> = Vec::new();
        for id in [src_id, tgt_id] {
            if let Some(&i) = self.by_path.get(id) {
                passable.push(i);
                passable.extend(self.nodes[i].ancestors.iter().copied());
            }
        }
        passable
    }

    /// Like `obstacles_for` but returns each obstacle's *path* and its
    /// *un-inflated* bbox. The validator inflates by the oracle clearance
    /// itself, so this stays free of any clearance policy.
    pub fn raw_obstacles(&self, src_id: &str, tgt_id: &str) -> Vec<(String, AbsBbox)> {
        let passable = self.passable_set(src_id, tgt_id);
        let mut out = Vec::new();
        for (i, n) in self.nodes.iter().enumerate() {
            if passable.contains(&i) {
                continue;
            }
            if !n.ancestors.iter().all(|a| passable.contains(a)) {
                continue;
            }
            if !n.is_leaf && n.bbox.w == 0.0 && n.bbox.h == 0.0 {
                continue;
            }
            out.push((n.path.clone(), n.bbox));
        }
        out
    }

    /// Every shape's `(path, bbox)` — used to seed the global visibility
    /// lattice with every shape's clearance edges. Skips zero-size groups
    /// (same rule as `raw_obstacles`).
    pub fn all_boxes(&self) -> Vec<(String, AbsBbox)> {
        self.nodes
            .iter()
            .filter(|n| n.is_leaf || n.bbox.w != 0.0 || n.bbox.h != 0.0)
            .map(|n| (n.path.clone(), n.bbox))
            .collect()
    }

    /// World bounds spanning every node. Used by the perimeter-route fallback
    /// to know where the "outside" is.
    pub fn bounds(&self, pad: f64) -> AbsBbox {
        if self.nodes.is_empty() {
            return AbsBbox {
                x: -50.0,
                y: -50.0,
                w: 100.0,
                h: 100.0,
            };
        }
        let mut min_x = f64::INFINITY;
        let mut min_y = f64::INFINITY;
        let mut max_x = f64::NEG_INFINITY;
        let mut max_y = f64::NEG_INFINITY;
        for n in &self.nodes {
            min_x = min_x.min(n.bbox.x);
            min_y = min_y.min(n.bbox.y);
            max_x = max_x.max(n.bbox.x + n.bbox.w);
            max_y = max_y.max(n.bbox.y + n.bbox.h);
        }
        AbsBbox {
            x: min_x - pad * 2.0,
            y: min_y - pad * 2.0,
            w: max_x - min_x + pad * 4.0,
            h: max_y - min_y + pad * 4.0,
        }
    }
}

/// `gap` attribute as a single scalar — either the value itself or the
/// larger of `(y_gap, x_gap)` when it's a tuple, so the wire clearance
/// covers the worst-case axis. Returns `None` if no `gap` attr was set.
fn explicit_gap(attrs: &AttrMap) -> Option<f64> {
    let v = attrs.get("gap")?;
    match v {
        ResolvedValue::Number(n) => Some(*n),
        ResolvedValue::Tuple(parts) => {
            let mut best: Option<f64> = None;
            for p in parts {
                if let ResolvedValue::Number(n) = p {
                    best = Some(best.map_or(*n, |b: f64| b.max(*n)));
                }
            }
            best
        }
        _ => None,
    }
}
