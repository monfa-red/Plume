use crate::ast::{DeclKind, File};
use crate::error::Error;
use crate::span::Span;

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

pub fn resolve(file: File) -> Result<Program, Error> {
    let mut nodes = Vec::new();

    if let Some(scene) = file.scene {
        for decl in scene.items {
            match decl.kind {
                DeclKind::Primitive { ty, label } => {
                    let shape = resolve_shape(&ty, decl.span)?;
                    nodes.push(Node { shape, label });
                }
            }
        }
    }

    Ok(Program { nodes })
}

fn resolve_shape(name: &str, span: Span) -> Result<Shape, Error> {
    match name {
        "rect" => Ok(Shape::Rect),
        other => Err(Error::at(span, format!("unknown type ':{}'", other))),
    }
}
