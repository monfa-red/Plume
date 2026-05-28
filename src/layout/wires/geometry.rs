//! Basic geometry types shared by every phase of routing.
//!
//! `Edge` names the four sides of an axis-aligned rectangle. `AbsBbox` is a
//! shape's bounding box in absolute scene coordinates. Helpers convert
//! between user-facing `Side` and the internal `Edge`, find edge midpoints,
//! and pick the nearest edge facing a target point.

use crate::ast::Side;

#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash, PartialOrd, Ord)]
pub enum Edge {
    Right,
    Bottom,
    Left,
    Top,
}

impl Edge {
    /// True for Left/Right (the segment leaving this edge is horizontal).
    pub fn is_horizontal_exit(self) -> bool {
        matches!(self, Edge::Left | Edge::Right)
    }

    pub fn opposite(self) -> Edge {
        match self {
            Edge::Right => Edge::Left,
            Edge::Left => Edge::Right,
            Edge::Top => Edge::Bottom,
            Edge::Bottom => Edge::Top,
        }
    }
}

pub fn side_to_edge(s: Side) -> Edge {
    match s {
        Side::Top => Edge::Top,
        Side::Bottom => Edge::Bottom,
        Side::Left => Edge::Left,
        Side::Right => Edge::Right,
    }
}

/// Pick the edge of `my` that most directly faces the point `other`.
/// Ties (perfectly aligned) prefer Right > Bottom > Left > Top — arbitrary
/// but deterministic so renders are reproducible.
pub fn nearest_edge(my: &AbsBbox, other: (f64, f64)) -> Edge {
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
    } else if dx >= 0.0 {
        Edge::Right
    } else if dy >= 0.0 {
        Edge::Bottom
    } else {
        Edge::Left
    }
}

#[derive(Clone, Copy, Debug)]
pub struct AbsBbox {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

impl AbsBbox {
    pub fn cx(&self) -> f64 {
        self.x + self.w / 2.0
    }
    pub fn cy(&self) -> f64 {
        self.y + self.h / 2.0
    }
    pub fn right(&self) -> f64 {
        self.x + self.w
    }
    pub fn bottom(&self) -> f64 {
        self.y + self.h
    }

    /// Grow this bbox by `pad` on every side. Used to compute the
    /// minimum-clearance footprint of an obstacle.
    pub fn inflate(&self, by: f64) -> AbsBbox {
        AbsBbox {
            x: self.x - by,
            y: self.y - by,
            w: self.w + 2.0 * by,
            h: self.h + 2.0 * by,
        }
    }
}

/// The exact world point at the midpoint of a shape's named edge.
pub fn edge_midpoint(bbox: &AbsBbox, e: Edge) -> (f64, f64) {
    match e {
        Edge::Right => (bbox.right(), bbox.cy()),
        Edge::Left => (bbox.x, bbox.cy()),
        Edge::Top => (bbox.cx(), bbox.y),
        Edge::Bottom => (bbox.cx(), bbox.bottom()),
    }
}

/// Apply a lane offset to a point sitting on a shape edge: shift it along
/// the edge by `lane` units, clamped so it stays at least `inset` from the
/// corners. Returns the displaced point.
pub fn shift_along_edge(pt: (f64, f64), edge: Edge, lane: f64, bbox: &AbsBbox) -> (f64, f64) {
    if lane.abs() < 0.01 {
        return pt;
    }
    let inset = 4.0;
    match edge {
        Edge::Top | Edge::Bottom => {
            let min_x = bbox.x + inset;
            let max_x = bbox.right() - inset;
            ((pt.0 + lane).clamp(min_x, max_x), pt.1)
        }
        Edge::Left | Edge::Right => {
            let min_y = bbox.y + inset;
            let max_y = bbox.bottom() - inset;
            (pt.0, (pt.1 + lane).clamp(min_y, max_y))
        }
    }
}
