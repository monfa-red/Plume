//! Wire routing.
//!
//! Walks `program.wires` (resolved IR) and produces orthogonal polyline paths
//! between scene nodes. Edge selection prefers a bracketed anchor; otherwise
//! picks the bbox edge nearest the other endpoint, with the SPEC §9 tie-break
//! (right > bottom > left > top). L-bend for perpendicular axes, Z-bend for
//! same-axis pairs.
//!
//! Chains (`a -> b -> c -> d`) split into per-segment sub-wires so the
//! rendered SVG draws one continuous-looking line through each waypoint while
//! keeping markers on the outer ends only.

use super::ir::{PlacedNode, RoutedText, RoutedWire};
use crate::ast::AnchorName;
use crate::error::Error;
use crate::resolve::{
    MarkerKind, Markers, Program, ResolvedEndpoint, ResolvedText, ResolvedWire, WireAt,
};
use crate::span::Span;

pub fn route_wires(
    program: &Program,
    scene_nodes: &[PlacedNode],
) -> Result<Vec<RoutedWire>, Error> {
    let mut out = Vec::new();
    for wire in &program.wires {
        out.extend(route_one(wire, scene_nodes)?);
    }
    Ok(out)
}

fn route_one(wire: &ResolvedWire, scene_nodes: &[PlacedNode]) -> Result<Vec<RoutedWire>, Error> {
    // Resolve every endpoint's absolute bbox up front.
    let bboxes: Vec<AbsBbox> = wire
        .endpoints
        .iter()
        .map(|ep| find_bbox(scene_nodes, &ep.id, ep.span))
        .collect::<Result<_, _>>()?;

    let from_id = wire.endpoints.first().unwrap().id.clone();
    let to_id = wire.endpoints.last().unwrap().id.clone();

    let mut sub_wires = Vec::new();
    let n = wire.endpoints.len();
    for i in 0..(n - 1) {
        let src = &bboxes[i];
        let tgt = &bboxes[i + 1];
        let src_anchor = wire.endpoints[i].anchor;
        let tgt_anchor = wire.endpoints[i + 1].anchor;

        if std::ptr::eq(src, tgt) || ptrs_equal_id(&wire.endpoints[i], &wire.endpoints[i + 1]) {
            return Err(Error::at(
                wire.span,
                "self-loops are not yet routed (Sprint 4 limitation)",
            ));
        }

        let path = route_segment(src, src_anchor, tgt, tgt_anchor);
        let is_first = i == 0;
        let is_last = i == n - 2;
        let segment_markers = Markers {
            start: if is_first {
                wire.markers.start
            } else {
                MarkerKind::None
            },
            end: if is_last {
                wire.markers.end
            } else {
                MarkerKind::None
            },
        };

        let texts = if is_first {
            place_texts(&wire.texts, &path)
        } else {
            Vec::new()
        };

        sub_wires.push(RoutedWire {
            path,
            markers: segment_markers,
            attrs: wire.attrs.clone(),
            texts,
            data_from: from_id.clone(),
            data_to: to_id.clone(),
        });
    }
    Ok(sub_wires)
}

fn ptrs_equal_id(a: &ResolvedEndpoint, b: &ResolvedEndpoint) -> bool {
    a.id == b.id
}

// ───────────────────────── Segment path ─────────────────────────

fn route_segment(
    src: &AbsBbox,
    src_anchor: Option<AnchorName>,
    tgt: &AbsBbox,
    tgt_anchor: Option<AnchorName>,
) -> Vec<(f64, f64)> {
    let src_edge = src_anchor
        .map(anchor_to_edge)
        .unwrap_or_else(|| nearest_edge(src, (tgt.cx(), tgt.cy())));
    let tgt_edge = tgt_anchor
        .map(anchor_to_edge)
        .unwrap_or_else(|| nearest_edge(tgt, (src.cx(), src.cy())));

    let s = edge_midpoint(src, src_edge);
    let t = edge_midpoint(tgt, tgt_edge);

    if edge_axis(src_edge) == edge_axis(tgt_edge) {
        // Same axis — straight line if collinear, else Z-bend.
        if edge_axis(src_edge) == Axis::Horizontal {
            if (s.1 - t.1).abs() < 0.5 {
                vec![s, t]
            } else {
                let mid_x = (s.0 + t.0) / 2.0;
                vec![s, (mid_x, s.1), (mid_x, t.1), t]
            }
        } else if (s.0 - t.0).abs() < 0.5 {
            vec![s, t]
        } else {
            let mid_y = (s.1 + t.1) / 2.0;
            vec![s, (s.0, mid_y), (t.0, mid_y), t]
        }
    } else {
        // Perpendicular axes — L-bend with one elbow.
        let bend = if edge_axis(src_edge) == Axis::Horizontal {
            (t.0, s.1)
        } else {
            (s.0, t.1)
        };
        vec![s, bend, t]
    }
}

// ───────────────────────── Edge selection ─────────────────────────

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Edge {
    Right,
    Bottom,
    Left,
    Top,
    TopLeft,
    TopRight,
    BottomLeft,
    BottomRight,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Axis {
    Horizontal,
    Vertical,
}

fn nearest_edge(my: &AbsBbox, other: (f64, f64)) -> Edge {
    let dx = other.0 - my.cx();
    let dy = other.1 - my.cy();
    let adx = dx.abs();
    let ady = dy.abs();
    if adx > ady {
        if dx >= 0.0 {
            Edge::Right
        } else {
            Edge::Left
        }
    } else if ady > adx {
        if dy >= 0.0 {
            Edge::Bottom
        } else {
            Edge::Top
        }
    } else {
        // Tied. Priority: right > bottom > left > top.
        if dx >= 0.0 {
            Edge::Right
        } else if dy >= 0.0 {
            Edge::Bottom
        } else {
            Edge::Left
        }
    }
}

fn anchor_to_edge(a: AnchorName) -> Edge {
    match a {
        AnchorName::Top => Edge::Top,
        AnchorName::Bottom => Edge::Bottom,
        AnchorName::Left => Edge::Left,
        AnchorName::Right => Edge::Right,
        AnchorName::TopLeft => Edge::TopLeft,
        AnchorName::TopRight => Edge::TopRight,
        AnchorName::BottomLeft => Edge::BottomLeft,
        AnchorName::BottomRight => Edge::BottomRight,
    }
}

fn edge_axis(e: Edge) -> Axis {
    match e {
        Edge::Right | Edge::Left | Edge::TopLeft | Edge::BottomLeft => Axis::Horizontal,
        Edge::Top | Edge::Bottom | Edge::TopRight | Edge::BottomRight => Axis::Vertical,
    }
}

fn edge_midpoint(bbox: &AbsBbox, e: Edge) -> (f64, f64) {
    match e {
        Edge::Right => (bbox.x + bbox.w, bbox.cy()),
        Edge::Left => (bbox.x, bbox.cy()),
        Edge::Top => (bbox.cx(), bbox.y),
        Edge::Bottom => (bbox.cx(), bbox.y + bbox.h),
        Edge::TopLeft => (bbox.x, bbox.y),
        Edge::TopRight => (bbox.x + bbox.w, bbox.y),
        Edge::BottomLeft => (bbox.x, bbox.y + bbox.h),
        Edge::BottomRight => (bbox.x + bbox.w, bbox.y + bbox.h),
    }
}

// ───────────────────────── Text placement ─────────────────────────

fn place_texts(texts: &[ResolvedText], path: &[(f64, f64)]) -> Vec<RoutedText> {
    let mut out = Vec::with_capacity(texts.len());
    for t in texts {
        let fraction = match &t.at {
            WireAt::Start => 0.0,
            WireAt::Mid => 0.5,
            WireAt::End => 1.0,
            WireAt::Fraction(f) => *f,
        };
        let (pos, tangent) = point_at_fraction(path, fraction);
        out.push(RoutedText {
            content: t.text.clone(),
            position: pos,
            tangent,
            attrs: t.attrs.clone(),
        });
    }
    out
}

fn point_at_fraction(path: &[(f64, f64)], f: f64) -> ((f64, f64), (f64, f64)) {
    if path.is_empty() {
        return ((0.0, 0.0), (1.0, 0.0));
    }
    if path.len() == 1 {
        return (path[0], (1.0, 0.0));
    }
    let total: f64 = path.windows(2).map(|w| dist(w[0], w[1])).sum();
    let target = total * f.clamp(0.0, 1.0);
    let mut acc = 0.0;
    for w in path.windows(2) {
        let seg = dist(w[0], w[1]);
        if acc + seg >= target {
            let local_f = if seg > 0.0 { (target - acc) / seg } else { 0.0 };
            let x = w[0].0 + (w[1].0 - w[0].0) * local_f;
            let y = w[0].1 + (w[1].1 - w[0].1) * local_f;
            let dx = (w[1].0 - w[0].0) / seg.max(1e-9);
            let dy = (w[1].1 - w[0].1) / seg.max(1e-9);
            return ((x, y), (dx, dy));
        }
        acc += seg;
    }
    let last = *path.last().unwrap();
    let prev = path[path.len() - 2];
    let dx = last.0 - prev.0;
    let dy = last.1 - prev.1;
    let len = (dx * dx + dy * dy).sqrt().max(1e-9);
    (last, (dx / len, dy / len))
}

fn dist(a: (f64, f64), b: (f64, f64)) -> f64 {
    ((b.0 - a.0).powi(2) + (b.1 - a.1).powi(2)).sqrt()
}

// ───────────────────────── Bbox lookup ─────────────────────────

#[derive(Clone, Copy, Debug)]
struct AbsBbox {
    /// Top-left in scene coords.
    x: f64,
    y: f64,
    w: f64,
    h: f64,
}

impl AbsBbox {
    fn cx(&self) -> f64 {
        self.x + self.w / 2.0
    }
    fn cy(&self) -> f64 {
        self.y + self.h / 2.0
    }
}

fn find_bbox(nodes: &[PlacedNode], id: &str, ref_span: Span) -> Result<AbsBbox, Error> {
    for node in nodes {
        if let Some(bb) = find_recurse(node, id, 0.0, 0.0) {
            return Ok(bb);
        }
    }
    Err(Error::at(
        ref_span,
        format!("wire references undefined id '{}'", id),
    ))
}

fn find_recurse(node: &PlacedNode, id: &str, parent_cx: f64, parent_cy: f64) -> Option<AbsBbox> {
    let abs_cx = parent_cx + node.cx;
    let abs_cy = parent_cy + node.cy;
    if let Some(node_id) = &node.id {
        if node_id == id {
            return Some(AbsBbox {
                x: abs_cx + node.bbox.min_x,
                y: abs_cy + node.bbox.min_y,
                w: node.bbox.w(),
                h: node.bbox.h(),
            });
        }
    }
    for child in &node.children {
        if let Some(bb) = find_recurse(child, id, abs_cx, abs_cy) {
            return Some(bb);
        }
    }
    None
}
