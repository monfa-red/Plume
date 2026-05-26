use crate::ast::{Block, File, ShapeInst};
use crate::error::Error;

#[derive(Debug)]
pub struct Program {
    pub nodes: Vec<Node>,
}

#[derive(Debug)]
pub struct Node {
    pub shape: Shape,
    pub label: Option<String>,
}

#[derive(Debug, Clone, Copy)]
pub enum Shape {
    Rect,
}

/// Sprint 1 resolver: handles the Sprint 0 hello case (anonymous `:rect`
/// primitives in `scene`, with an optional label). Other blocks and shapes
/// surface a "not yet implemented" error so the compile pipeline stays honest.
pub fn resolve(file: File) -> Result<Program, Error> {
    let mut nodes = Vec::new();

    for block in file.blocks {
        match block {
            Block::Scene(scene) => {
                for inst in scene.body {
                    nodes.push(resolve_inst(&inst)?);
                }
            }
            Block::Defaults(b) => {
                return Err(Error::at(b.span, "defaults block: not yet implemented"));
            }
            Block::Styles(b) => {
                return Err(Error::at(b.span, "styles block: not yet implemented"));
            }
            Block::Shapes(b) => {
                return Err(Error::at(b.span, "shapes block: not yet implemented"));
            }
            Block::Wires(b) => {
                return Err(Error::at(b.span, "wires block: not yet implemented"));
            }
        }
    }

    Ok(Program { nodes })
}

fn resolve_inst(inst: &ShapeInst) -> Result<Node, Error> {
    if inst.id.is_some() {
        return Err(Error::at(
            inst.span,
            "named scene nodes: not yet implemented",
        ));
    }
    if inst.body.is_some() {
        return Err(Error::at(inst.span, "node bodies: not yet implemented"));
    }
    if !inst.items.is_empty() {
        return Err(Error::at(inst.span, "node attrs: not yet implemented"));
    }

    let shape = match inst.ty.name.as_str() {
        "rect" => Shape::Rect,
        other => {
            return Err(Error::at(
                inst.ty.span,
                format!("type ':{}' not yet implemented", other),
            ));
        }
    };

    Ok(Node {
        shape,
        label: inst.label.clone(),
    })
}
