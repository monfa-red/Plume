//! A flat index of the placed scene: every id'd node's absolute bounding box,
//! keyed by its dot-path from the scene root. Both the router (to locate
//! endpoints) and the validator (to check attachment) build their own — neither
//! depends on the other.

use super::geometry::Rect;
use crate::layout::ir::PlacedNode;
use std::collections::BTreeMap;

pub struct SceneIndex {
    rects: BTreeMap<String, Rect>,
}

impl SceneIndex {
    pub fn build(nodes: &[PlacedNode]) -> Self {
        let mut rects = BTreeMap::new();
        for n in nodes {
            walk(n, &[], 0.0, 0.0, &mut rects);
        }
        Self { rects }
    }

    pub fn rect(&self, path: &str) -> Option<Rect> {
        self.rects.get(path).copied()
    }
}

/// Accumulate origins down the tree. Only id'd nodes contribute a path segment
/// (anonymous primitives don't, matching how resolve builds endpoint paths),
/// but every node's `cx`/`cy` offsets its descendants.
fn walk(n: &PlacedNode, prefix: &[String], ox: f64, oy: f64, out: &mut BTreeMap<String, Rect>) {
    let (nox, noy) = (ox + n.cx, oy + n.cy);
    let mut path = prefix.to_vec();
    if let Some(id) = &n.id {
        path.push(id.clone());
        out.insert(
            path.join("."),
            Rect {
                min_x: n.bbox.min_x + nox,
                min_y: n.bbox.min_y + noy,
                max_x: n.bbox.max_x + nox,
                max_y: n.bbox.max_y + noy,
            },
        );
    }
    for c in &n.children {
        walk(c, &path, nox, noy, out);
    }
}
