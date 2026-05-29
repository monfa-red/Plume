//! A flat index of the placed scene: every id'd node's absolute bounding box,
//! keyed by its dot-path from the scene root. Both the router (to locate
//! endpoints) and the validator (to check attachment) build their own — neither
//! depends on the other.

use super::geometry::Rect;
use crate::layout::ir::PlacedNode;
use crate::resolve::ShapeKind;
use std::collections::BTreeMap;

pub struct SceneIndex {
    nodes: BTreeMap<String, NodeInfo>,
}

#[derive(Clone, Copy)]
struct NodeInfo {
    rect: Rect,
    shape: ShapeKind,
}

impl SceneIndex {
    pub fn build(nodes: &[PlacedNode]) -> Self {
        let mut map = BTreeMap::new();
        for n in nodes {
            walk(n, &[], 0.0, 0.0, &mut map);
        }
        Self { nodes: map }
    }

    /// Absolute bbox of the node at `path`.
    pub fn rect(&self, path: &str) -> Option<Rect> {
        self.nodes.get(path).map(|n| n.rect)
    }

    /// Primitive kind of the node at `path` — lets the validator reject a wire
    /// that targets a text node (A4).
    pub fn shape(&self, path: &str) -> Option<ShapeKind> {
        self.nodes.get(path).map(|n| n.shape)
    }
}

/// Accumulate origins down the tree. Only id'd nodes contribute a path segment
/// (anonymous primitives don't, matching how resolve builds endpoint paths),
/// but every node's `cx`/`cy` offsets its descendants.
fn walk(n: &PlacedNode, prefix: &[String], ox: f64, oy: f64, out: &mut BTreeMap<String, NodeInfo>) {
    let (nox, noy) = (ox + n.cx, oy + n.cy);
    let mut path = prefix.to_vec();
    if let Some(id) = &n.id {
        path.push(id.clone());
        out.insert(
            path.join("."),
            NodeInfo {
                rect: Rect {
                    min_x: n.bbox.min_x + nox,
                    min_y: n.bbox.min_y + noy,
                    max_x: n.bbox.max_x + nox,
                    max_y: n.bbox.max_y + noy,
                },
                shape: n.shape,
            },
        );
    }
    for c in &n.children {
        walk(c, &path, nox, noy, out);
    }
}

/// The solid obstacles a single wire must avoid (WIRING §Definitions).
///
/// A wire's own two `endpoints` and every container that *contains* one of them
/// are **passable** — the wire may cross their boundaries. Every other shape is a
/// **solid** obstacle: its whole bbox blocks the wire, and the router won't thread
/// between a solid group's children, so a non-passable subtree collapses to one
/// rect. Text nodes are never obstacles. A passable container still contributes
/// its *non-endpoint* children as obstacles.
pub fn obstacles_for(nodes: &[PlacedNode], endpoints: [&str; 2]) -> Vec<Rect> {
    let mut out = Vec::new();
    gather(nodes, &[], 0.0, 0.0, &endpoints, &mut out);
    out
}

/// Walk one sibling list at parent origin `(ox, oy)`. Returns whether any sibling
/// subtree is passable (contains an endpoint); pushes the solid obstacles found.
fn gather(
    siblings: &[PlacedNode],
    prefix: &[String],
    ox: f64,
    oy: f64,
    endpoints: &[&str; 2],
    out: &mut Vec<Rect>,
) -> bool {
    let mut any_passable = false;
    for c in siblings {
        if c.shape == ShapeKind::Text {
            continue; // text rides along — never an obstacle, never an endpoint
        }
        let mut path = prefix.to_vec();
        if let Some(id) = &c.id {
            path.push(id.clone());
        }
        let is_endpoint = c.id.is_some() && {
            let p = path.join(".");
            p == endpoints[0] || p == endpoints[1]
        };

        let (cox, coy) = (ox + c.cx, oy + c.cy);
        let mut inner = Vec::new();
        let child_passable = gather(&c.children, &path, cox, coy, endpoints, &mut inner);

        if is_endpoint || child_passable {
            out.extend(inner); // a passable region exposes its inner obstacles
            any_passable = true;
        } else {
            out.push(abs_rect(c, ox, oy)); // whole subtree collapses to one solid rect
        }
    }
    any_passable
}

/// A node's absolute bbox, given its parent's accumulated origin `(ox, oy)`.
fn abs_rect(n: &PlacedNode, ox: f64, oy: f64) -> Rect {
    let (x, y) = (ox + n.cx, oy + n.cy);
    Rect {
        min_x: n.bbox.min_x + x,
        min_y: n.bbox.min_y + y,
        max_x: n.bbox.max_x + x,
        max_y: n.bbox.max_y + y,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scene(src: &str) -> Vec<PlacedNode> {
        let tokens = crate::lexer::lex(src).expect("lex");
        let file = crate::parser::parse(&tokens).expect("parse");
        let program = crate::resolve::resolve(file).expect("resolve");
        crate::layout::layout(&program).expect("layout").nodes
    }

    fn contains(r: &Rect, p: (f64, f64)) -> bool {
        p.0 >= r.min_x && p.0 <= r.max_x && p.1 >= r.min_y && p.1 <= r.max_y
    }

    // a, other at top level; g is a group wrapping inner. Every shape carries a
    // text label, which must be ignored.
    const SRC: &str = "{ |scene| layout:row gap:40 }\n\
                       a     |rect|  \"A\" size:(40,40)\n\
                       other |rect|  \"O\" size:(40,40)\n\
                       g     |group| \"G\" layout:column gap:10 {\n\
                       \x20 inner |rect| \"I\" size:(30,30)\n\
                       }\n";

    #[test]
    fn endpoint_and_ancestor_passable_others_solid() {
        let nodes = scene(SRC);
        let idx = SceneIndex::build(&nodes);
        let other_c = idx.rect("other").unwrap().center();
        let inner_c = idx.rect("g.inner").unwrap().center();

        let obs = obstacles_for(&nodes, ["a", "g.inner"]);

        assert!(
            obs.iter().any(|r| contains(r, other_c)),
            "other is a solid obstacle"
        );
        assert!(
            !obs.iter().any(|r| contains(r, inner_c)),
            "the endpoint inner and its ancestor g are passable"
        );
        assert_eq!(
            obs.len(),
            1,
            "only other — text labels and passable nodes excluded"
        );
    }

    #[test]
    fn non_endpoint_children_of_an_endpoint_group_are_obstacles() {
        let nodes = scene(SRC);
        let idx = SceneIndex::build(&nodes);
        let other_c = idx.rect("other").unwrap().center();
        let inner_c = idx.rect("g.inner").unwrap().center();

        // Wiring to the group g: inner is now a non-endpoint child of an endpoint.
        let obs = obstacles_for(&nodes, ["a", "g"]);

        assert!(obs.iter().any(|r| contains(r, other_c)));
        assert!(
            obs.iter().any(|r| contains(r, inner_c)),
            "a non-endpoint child of an endpoint container is a solid obstacle"
        );
        assert_eq!(obs.len(), 2);
    }
}
