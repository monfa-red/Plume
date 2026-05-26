use crate::error::Error;
use crate::resolve::{Program, ResolvedInst, ShapeKind};

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
    pub shape: ShapeKind,
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
    let placed: Vec<PlacedNode> = program
        .scene
        .nodes
        .iter()
        .map(place_top_level)
        .collect::<Result<_, _>>()?;

    Ok(LaidOut {
        viewbox: compute_viewbox(&placed),
        nodes: placed,
    })
}

fn place_top_level(inst: &ResolvedInst) -> Result<PlacedNode, Error> {
    let (w, h) = match inst.shape {
        ShapeKind::Rect => (RECT_W, RECT_H),
        other => {
            return Err(Error::at(
                inst.span,
                format!("layout for shape ':{}' not yet implemented", other.as_str()),
            ));
        }
    };

    Ok(PlacedNode {
        shape: inst.shape,
        label: extract_label(inst),
        cx: 0.0,
        cy: 0.0,
        w,
        h,
    })
}

/// Find the first Text child and return its content. Used to surface the
/// label-sugar text on Rect (and other non-text) parents until Sprint 3 builds
/// the full child-positioning layout.
fn extract_label(inst: &ResolvedInst) -> Option<String> {
    inst.children
        .iter()
        .find(|c| c.shape == ShapeKind::Text)
        .and_then(|c| c.label.clone())
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
