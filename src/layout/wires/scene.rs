//! Scene index: a flat-but-tree-aware lookup from a fully-qualified shape
//! path (the dot-path the resolver canonicalises) to its absolute bbox and
//! its chain of named ancestors. The ancestor chain is how we decide which
//! shapes count as obstacles for a given wire: a container that holds the
//! source or target of the wire is *passable* (the wire enters it), every
//! other shape is a *hard obstacle*.

use super::geometry::AbsBbox;
use crate::layout::ir::PlacedNode;
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
}

#[derive(Clone)]
pub struct ShapeRef {
    pub bbox: AbsBbox,
}

impl SceneIndex {
    pub fn build(scene_nodes: &[PlacedNode]) -> Self {
        let mut idx = SceneIndex {
            nodes: Vec::new(),
            by_path: HashMap::new(),
        };
        for node in scene_nodes {
            idx.walk(node, 0.0, 0.0, &[], &mut Vec::new());
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
            });
            self.by_path.insert(full_path, i);
            next_ancestors.push(i);
        }
        for child in &node.children {
            self.walk(child, abs_cx, abs_cy, &next_ancestors, path_stack);
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

    /// Obstacles for a wire between `src_id` and `tgt_id`. Each shape is an
    /// obstacle UNLESS it is an endpoint or an ancestor of an endpoint, in
    /// which case the path is allowed to cross its boundary.
    pub fn obstacles_for(&self, src_id: &str, tgt_id: &str, gap: f64) -> Vec<AbsBbox> {
        let src_i = self.by_path.get(src_id).copied();
        let tgt_i = self.by_path.get(tgt_id).copied();
        let mut passable: Vec<usize> = Vec::new();
        if let Some(i) = src_i {
            passable.push(i);
            passable.extend(self.nodes[i].ancestors.iter().copied());
        }
        if let Some(i) = tgt_i {
            passable.push(i);
            passable.extend(self.nodes[i].ancestors.iter().copied());
        }

        let mut out = Vec::new();
        for (i, n) in self.nodes.iter().enumerate() {
            if passable.contains(&i) {
                continue;
            }
            // A container only contributes its own frame if all its named
            // ancestors are passable (otherwise its outer container would
            // already cover it).
            if !n.ancestors.iter().all(|a| passable.contains(a)) {
                continue;
            }
            // Skip degenerate (zero-sized) container labels.
            if !n.is_leaf && n.bbox.w == 0.0 && n.bbox.h == 0.0 {
                continue;
            }
            out.push(n.bbox.inflate(gap));
        }
        out
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
