use crate::error::Error;
use crate::resolve::{Node, Program, Shape};

#[derive(Debug)]
pub struct LaidOut {
    pub viewbox: ViewBox,
    pub nodes: Vec<PlacedNode>,
}

#[derive(Debug)]
pub struct ViewBox {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

#[derive(Debug)]
pub struct PlacedNode {
    pub shape: Shape,
    pub label: Option<String>,
    pub cx: f64,
    pub cy: f64,
    pub w: f64,
    pub h: f64,
}

const RECT_W: f64 = 100.0;
const RECT_H: f64 = 40.0;
const CANVAS_PAD: f64 = 20.0;

pub fn layout(program: &Program) -> Result<LaidOut, Error> {
    let placed: Vec<PlacedNode> = program.nodes.iter().map(place_node).collect();

    let viewbox = compute_viewbox(&placed);
    Ok(LaidOut {
        viewbox,
        nodes: placed,
    })
}

fn place_node(n: &Node) -> PlacedNode {
    let (w, h) = match n.shape {
        Shape::Rect => (RECT_W, RECT_H),
    };
    PlacedNode {
        shape: n.shape,
        label: n.label.clone(),
        cx: 0.0,
        cy: 0.0,
        w,
        h,
    }
}

fn compute_viewbox(nodes: &[PlacedNode]) -> ViewBox {
    if nodes.is_empty() {
        return ViewBox {
            x: -CANVAS_PAD,
            y: -CANVAS_PAD,
            w: 2.0 * CANVAS_PAD,
            h: 2.0 * CANVAS_PAD,
        };
    }

    let mut minx = f64::INFINITY;
    let mut miny = f64::INFINITY;
    let mut maxx = f64::NEG_INFINITY;
    let mut maxy = f64::NEG_INFINITY;

    for n in nodes {
        minx = minx.min(n.cx - n.w / 2.0);
        miny = miny.min(n.cy - n.h / 2.0);
        maxx = maxx.max(n.cx + n.w / 2.0);
        maxy = maxy.max(n.cy + n.h / 2.0);
    }

    ViewBox {
        x: minx - CANVAS_PAD,
        y: miny - CANVAS_PAD,
        w: (maxx - minx) + 2.0 * CANVAS_PAD,
        h: (maxy - miny) + 2.0 * CANVAS_PAD,
    }
}
