// AST node fields are progressively consumed by resolve/layout/render across
// sprints. Allow dead_code at the module level so CI's `-D warnings` doesn't
// gate progress on partial pipeline coverage.
#![allow(dead_code)]

use crate::span::Span;

#[derive(Debug)]
pub struct File {
    pub blocks: Vec<Block>,
}

#[derive(Debug)]
pub enum Block {
    Defaults(DefaultsBlock),
    Styles(StylesBlock),
    Shapes(ShapesBlock),
    Scene(SceneBlock),
    Wires(WiresBlock),
}

#[derive(Debug)]
pub struct DefaultsBlock {
    pub entries: Vec<DefaultEntry>,
    pub span: Span,
}

#[derive(Debug)]
pub struct DefaultEntry {
    pub name: String,
    pub value: Value,
    pub span: Span,
}

#[derive(Debug)]
pub struct StylesBlock {
    pub styles: Vec<StyleDef>,
    pub span: Span,
}

#[derive(Debug)]
pub struct StyleDef {
    pub name: String,
    pub items: Vec<AttrItem>,
    pub span: Span,
}

#[derive(Debug)]
pub struct ShapesBlock {
    pub shapes: Vec<ShapeDef>,
    pub span: Span,
}

#[derive(Debug)]
pub struct ShapeDef {
    pub name: String,
    pub base: Option<TypeRef>,
    pub items: Vec<AttrItem>,
    pub body: Option<Vec<ShapeInst>>,
    pub span: Span,
}

#[derive(Debug)]
pub struct SceneBlock {
    pub items: Vec<AttrItem>,
    pub body: Vec<ShapeInst>,
    pub span: Span,
}

#[derive(Debug)]
pub struct WiresBlock {
    pub items: Vec<AttrItem>,
    pub wires: Vec<WireDecl>,
    pub span: Span,
}

/// Node or primitive instance. `id` is `Some` for node decls in a scene,
/// `None` for anonymous primitives in a shape body or scene.
#[derive(Debug)]
pub struct ShapeInst {
    pub id: Option<String>,
    pub ty: TypeRef,
    pub label: Option<String>,
    pub items: Vec<AttrItem>,
    pub body: Option<Vec<ShapeInst>>,
    pub span: Span,
}

#[derive(Debug)]
pub struct WireDecl {
    pub endpoints: Vec<WireEndpoint>,
    pub op: WireOp,
    pub label: Option<String>,
    pub items: Vec<AttrItem>,
    pub body: Option<Vec<TextDecl>>,
    pub span: Span,
}

#[derive(Debug)]
pub struct WireEndpoint {
    pub id: String,
    pub anchor: Option<AnchorName>,
    pub span: Span,
}

#[derive(Debug)]
pub struct TextDecl {
    pub text: String,
    pub items: Vec<AttrItem>,
    pub span: Span,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WireOp {
    Arrow,
    LArrow,
    Biarrow,
    ArrowDash,
    LArrowDash,
    BiarrowDash,
    ArrowDot,
    LArrowDot,
    BiarrowDot,
}

impl WireOp {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Arrow => "->",
            Self::LArrow => "<-",
            Self::Biarrow => "<->",
            Self::ArrowDash => "-->",
            Self::LArrowDash => "<--",
            Self::BiarrowDash => "<-->",
            Self::ArrowDot => "-.->",
            Self::LArrowDot => "<-.-",
            Self::BiarrowDot => "<-.->",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AnchorName {
    Top,
    Bottom,
    Left,
    Right,
    TopLeft,
    TopRight,
    BottomLeft,
    BottomRight,
}

impl AnchorName {
    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "top" => Self::Top,
            "bottom" => Self::Bottom,
            "left" => Self::Left,
            "right" => Self::Right,
            "top-left" => Self::TopLeft,
            "top-right" => Self::TopRight,
            "bottom-left" => Self::BottomLeft,
            "bottom-right" => Self::BottomRight,
            _ => return None,
        })
    }
}

#[derive(Debug, Clone)]
pub struct TypeRef {
    pub name: String,
    pub span: Span,
}

#[derive(Debug)]
pub enum AttrItem {
    Attr(Attr),
    Style(StyleRef),
}

#[derive(Debug)]
pub struct Attr {
    pub name: String,
    pub value: Option<Value>, // None = bare attr
    pub span: Span,
}

#[derive(Debug)]
pub struct StyleRef {
    pub name: String,
    pub span: Span,
}

#[derive(Debug)]
pub enum Value {
    Number(f64),
    String(String),
    Hex(String),
    Ident(String),
    Tuple(Vec<Value>),
    List(Vec<Value>),
    Call(FnCall),
    RawCssVar(String), // only valid inside var()
}

#[derive(Debug)]
pub struct FnCall {
    pub name: String,
    pub args: Vec<Value>,
    pub span: Span,
}
