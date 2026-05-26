use crate::span::Span;

#[derive(Debug)]
pub struct File {
    pub scene: Option<Scene>,
}

#[derive(Debug)]
pub struct Scene {
    pub items: Vec<Decl>,
}

#[derive(Debug)]
pub struct Decl {
    pub kind: DeclKind,
    pub span: Span,
}

#[derive(Debug)]
pub enum DeclKind {
    Primitive { ty: String, label: Option<String> },
}
