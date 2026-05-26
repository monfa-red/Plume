use super::ir::{ResolvedAttr, ShapeKind, VarTable};
use super::styles::StyleTable;
use super::vars::resolve_value;
use crate::ast::{AttrItem, ShapeDef, ShapeInst};
use crate::error::Error;
use crate::span::Span;
use std::collections::HashMap;

const MAX_INHERITANCE_DEPTH: usize = 16;

/// Built-in templates per SPEC §8. Each maps to a base primitive.
/// Default attrs from §8 are added in later sprints; Sprint 2 only needs the
/// base relationship for chain walking.
const TEMPLATES: &[(&str, &str)] = &[
    ("group", "rect"),
    ("circle", "oval"),
    ("badge", "rect"),
    ("button", "rect"),
    ("card", "rect"),
    ("note", "rect"),
    ("db", "cyl"),
    ("table", "group"),
    ("cell", "rect"),
    ("dim", "line"),
];

/// Result of resolving a `:type` reference. `kind` is the underlying primitive;
/// `attrs` and `body_items` accumulate from the inheritance chain.
#[derive(Clone)]
pub struct ResolvedShape {
    pub kind: ShapeKind,
    pub attrs: Vec<ResolvedAttr>,
    pub body_items: Vec<ShapeInst>,
}

pub struct ShapesTable {
    user: HashMap<String, ResolvedShapeDef>,
}

struct ResolvedShapeDef {
    base: Option<String>,
    attrs: Vec<ResolvedAttr>,
    body_items: Vec<ShapeInst>,
    span: Span,
}

impl ShapesTable {
    pub fn build(defs: &[ShapeDef], styles: &StyleTable, vars: &VarTable) -> Result<Self, Error> {
        let mut user: HashMap<String, ResolvedShapeDef> = HashMap::new();

        for def in defs {
            if super::is_reserved(&def.name) {
                return Err(Error::at(def.span, format!("'{}' is reserved", def.name)));
            }
            if ShapeKind::parse(&def.name).is_some() {
                return Err(Error::at(
                    def.span,
                    format!("'{}' shadows a built-in primitive", def.name),
                ));
            }
            if is_template_name(&def.name) {
                return Err(Error::at(
                    def.span,
                    format!("'{}' shadows a built-in template", def.name),
                ));
            }
            if user.contains_key(&def.name) {
                return Err(Error::at(
                    def.span,
                    format!("duplicate shape '{}'", def.name),
                ));
            }

            // Resolve items into attrs (expanding styles).
            let mut attrs: Vec<ResolvedAttr> = Vec::new();
            for item in &def.items {
                match item {
                    AttrItem::Attr(a) => {
                        let value = match &a.value {
                            Some(v) => Some(resolve_value(v, vars)?),
                            None => None,
                        };
                        attrs.push(ResolvedAttr {
                            name: a.name.clone(),
                            value,
                            span: a.span,
                        });
                    }
                    AttrItem::Style(s) => {
                        let inner = styles.lookup(&s.name).ok_or_else(|| {
                            Error::at(s.span, format!("unknown style '.{}'", s.name))
                        })?;
                        attrs.extend(inner.iter().cloned());
                    }
                }
            }

            let base = def.base.as_ref().map(|t| t.name.clone());
            let body_items = def.body.clone().unwrap_or_default();

            user.insert(
                def.name.clone(),
                ResolvedShapeDef {
                    base,
                    attrs,
                    body_items,
                    span: def.span,
                },
            );
        }

        let table = Self { user };

        // Validate inheritance: every user shape's chain walks cleanly.
        for def in defs {
            let mut visiting: Vec<String> = Vec::new();
            table.walk_chain(&def.name, def.span, &mut visiting, 0)?;
        }

        Ok(table)
    }

    /// Resolve a `:type` reference into a primitive kind and inherited attrs.
    pub fn resolve(&self, name: &str, use_span: Span) -> Result<ResolvedShape, Error> {
        let mut visiting: Vec<String> = Vec::new();
        self.walk_chain(name, use_span, &mut visiting, 0)
    }

    fn walk_chain(
        &self,
        name: &str,
        use_span: Span,
        visiting: &mut Vec<String>,
        depth: usize,
    ) -> Result<ResolvedShape, Error> {
        if depth > MAX_INHERITANCE_DEPTH {
            return Err(Error::at(
                use_span,
                format!("'{}' exceeds max inheritance depth (16)", name),
            ));
        }
        if visiting.iter().any(|n| n == name) {
            let chain = format!("{} -> {}", visiting.join(" -> "), name);
            return Err(Error::at(use_span, format!("cycle in '{}'", chain)));
        }

        // Primitive — leaf, no further inheritance.
        if let Some(kind) = ShapeKind::parse(name) {
            return Ok(ResolvedShape {
                kind,
                attrs: Vec::new(),
                body_items: Vec::new(),
            });
        }

        // Template — built-in, walk its base.
        if let Some((_, base)) = TEMPLATES.iter().find(|(n, _)| *n == name) {
            visiting.push(name.to_string());
            let resolved = self.walk_chain(base, use_span, visiting, depth + 1)?;
            visiting.pop();
            return Ok(resolved);
        }

        // User shape — walk base (if any), then layer this shape's own attrs +
        // body items on top.
        let def = self
            .user
            .get(name)
            .ok_or_else(|| Error::at(use_span, format!("unknown type ':{}'", name)))?;

        visiting.push(name.to_string());
        let (kind, mut attrs, mut body_items) = match &def.base {
            Some(base_name) => {
                let base = self.walk_chain(base_name, def.span, visiting, depth + 1)?;
                (base.kind, base.attrs, base.body_items)
            }
            None => (ShapeKind::Rect, Vec::new(), Vec::new()),
        };
        visiting.pop();

        attrs.extend(def.attrs.iter().cloned());
        body_items.extend(def.body_items.iter().cloned());

        Ok(ResolvedShape {
            kind,
            attrs,
            body_items,
        })
    }
}

fn is_template_name(name: &str) -> bool {
    TEMPLATES.iter().any(|(n, _)| *n == name)
}
