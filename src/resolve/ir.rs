// Resolved IR types — fields are consumed progressively by layout/render across
// sprints, so allow dead_code at the module level to keep CI's `-D warnings`
// happy while the pipeline fills in.
#![allow(dead_code)]

use crate::ast::{LineStyle, Side};
use crate::span::Span;
use std::collections::{BTreeMap, HashMap};

/// Fully resolved program — output of phase 2.
pub struct Program {
    pub vars: VarTable,
    pub scene: ResolvedScene,
    pub wires: Vec<ResolvedWire>,
}

/// Scene block: container attrs + body of instances.
pub struct ResolvedScene {
    pub attrs: AttrMap,
    pub nodes: Vec<ResolvedInst>,
}

/// A resolved node or primitive instance. `id` is `Some` iff the source used a
/// named scene node (`cat |treat| …`); anonymous primitives have `id == None`.
pub struct ResolvedInst {
    pub id: Option<String>,
    pub shape: ShapeKind,
    /// User-shape and template names walked from the inst's declared type back
    /// to its primitive (e.g. for `cat |treat|` where `treat:rect`, this is
    /// `["treat"]` — the primitive `rect` is in `shape`).
    pub type_chain: Vec<String>,
    /// Style class names applied to this inst, in source (left-to-right) order.
    pub applied_styles: Vec<String>,
    /// For `Text` shape: the text content. For other shapes: always `None` —
    /// label sugar on non-text shapes produces a `Text` child instead.
    pub label: Option<String>,
    /// Optional href — wraps the rendered shape in `<a href>` for links.
    pub href: Option<String>,
    pub attrs: AttrMap,
    pub markers: Markers,
    pub children: Vec<ResolvedInst>,
    pub span: Span,
}

/// One of the 13 built-in primitives. All user shapes resolve to one of these.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShapeKind {
    Rect,
    Oval,
    Hex,
    Slant,
    Cyl,
    Diamond,
    Cloud,
    Poly,
    Path,
    Text,
    Line,
    Icon,
    Image,
}

impl ShapeKind {
    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "rect" => Self::Rect,
            "oval" => Self::Oval,
            "hex" => Self::Hex,
            "slant" => Self::Slant,
            "cyl" => Self::Cyl,
            "diamond" => Self::Diamond,
            "cloud" => Self::Cloud,
            "poly" => Self::Poly,
            "path" => Self::Path,
            "text" => Self::Text,
            "line" => Self::Line,
            "icon" => Self::Icon,
            "image" => Self::Image,
            _ => return None,
        })
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Rect => "rect",
            Self::Oval => "oval",
            Self::Hex => "hex",
            Self::Slant => "slant",
            Self::Cyl => "cyl",
            Self::Diamond => "diamond",
            Self::Cloud => "cloud",
            Self::Poly => "poly",
            Self::Path => "path",
            Self::Text => "text",
            Self::Line => "line",
            Self::Icon => "icon",
            Self::Image => "image",
        }
    }
}

/// One ordered attr from a style or inline merge — `value` is `None` for bare
/// attrs. Used as intermediate storage before §13 specificity collapse.
#[derive(Clone, Debug)]
pub struct ResolvedAttr {
    pub name: String,
    pub value: Option<ResolvedValue>,
    pub span: Span,
}

/// Final attribute values after §13 specificity merging. Marker attrs are
/// extracted into `Markers` and not stored here.
#[derive(Default, Clone, Debug)]
pub struct AttrMap {
    pub map: BTreeMap<String, ResolvedValue>,
}

impl AttrMap {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(&mut self, name: impl Into<String>, value: ResolvedValue) {
        self.map.insert(name.into(), value);
    }

    pub fn get(&self, name: &str) -> Option<&ResolvedValue> {
        self.map.get(name)
    }
}

#[derive(Clone, Debug)]
pub enum ResolvedValue {
    Number(f64),
    String(String),
    Hex(String),
    Ident(String),
    Tuple(Vec<ResolvedValue>),
    List(Vec<ResolvedValue>),
    Call(ResolvedCall),
    LiveVar {
        name: String,
        raw: bool,
        baked: Option<Box<ResolvedValue>>,
    },
}

#[derive(Clone, Debug)]
pub struct ResolvedCall {
    pub name: String,
    pub args: Vec<ResolvedValue>,
}

/// CSS variable defaults table. Entries are keyed by name without the
/// `--plume-` prefix.
#[derive(Clone, Debug, Default)]
pub struct VarTable {
    pub entries: HashMap<String, VarEntry>,
}

#[derive(Clone, Debug)]
pub struct VarEntry {
    pub kind: VarKind,
    pub value: ResolvedValue,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VarKind {
    Layout,
    Visual,
}

impl VarTable {
    pub fn new() -> Self {
        Self {
            entries: HashMap::new(),
        }
    }

    pub fn get(&self, name: &str) -> Option<&VarEntry> {
        self.entries.get(name)
    }

    pub fn set(&mut self, name: impl Into<String>, kind: VarKind, value: ResolvedValue) {
        self.entries.insert(name.into(), VarEntry { kind, value });
    }

    pub fn kind_of(&self, name: &str) -> Option<VarKind> {
        self.entries.get(name).map(|e| e.kind)
    }
}

#[derive(Clone, Debug, Default)]
pub struct Markers {
    pub start: MarkerKind,
    pub end: MarkerKind,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum MarkerKind {
    #[default]
    None,
    Arrow,
    Dot,
    Diamond,
    Crow,
}

impl MarkerKind {
    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "none" => Self::None,
            "arrow" => Self::Arrow,
            "dot" => Self::Dot,
            "diamond" => Self::Diamond,
            "crow" => Self::Crow,
            _ => return None,
        })
    }

    pub fn from_marker(m: crate::ast::WireMarker) -> Self {
        match m {
            crate::ast::WireMarker::None => Self::None,
            crate::ast::WireMarker::Arrow => Self::Arrow,
            crate::ast::WireMarker::Crow => Self::Crow,
            crate::ast::WireMarker::Dot => Self::Dot,
            crate::ast::WireMarker::Diamond => Self::Diamond,
        }
    }
}

pub struct ResolvedWire {
    pub endpoints: Vec<ResolvedEndpoint>,
    pub line: LineStyle,
    pub attrs: AttrMap,
    pub markers: Markers,
    pub texts: Vec<ResolvedText>,
    pub span: Span,
}

pub struct ResolvedEndpoint {
    /// Fully-qualified dot-path from scene root (e.g. `garden.outlet`).
    pub path: String,
    pub side: Option<Side>,
    pub span: Span,
}

pub struct ResolvedText {
    pub text: String,
    pub at: WireAt,
    pub attrs: AttrMap,
    pub span: Span,
}

#[derive(Clone, Debug)]
pub enum WireAt {
    Start,
    Mid,
    End,
    Fraction(f64),
}

impl WireAt {
    pub fn default_for_label() -> Self {
        Self::Mid
    }

    pub fn parse(value: &ResolvedValue) -> Option<Self> {
        match value {
            ResolvedValue::Ident(s) => match s.as_str() {
                "start" => Some(Self::Start),
                "mid" => Some(Self::Mid),
                "end" => Some(Self::End),
                _ => None,
            },
            ResolvedValue::Number(n) => {
                if (0.0..=1.0).contains(n) {
                    Some(Self::Fraction(*n))
                } else {
                    None
                }
            }
            _ => None,
        }
    }
}
