//! Canonical source formatter. Parses to AST, emits a normalized form.
//!
//! Rules (SPEC §14 `plume fmt`):
//! - 2-space indent.
//! - One declaration per line; sibling declarations inside the same block get
//!   their id / type / label / attr columns aligned.
//! - Comments and blank-line groupings between siblings are preserved (at most
//!   one blank line collapsed from any longer run).
//! - Canonical value formatting (`(1, 2)` not `( 1 ,2 )`, etc.).
//! - Idempotent: `fmt(fmt(x)) == fmt(x)`.

use crate::ast::{
    AttrItem, Block, DefaultsBlock, File, SceneBlock, ShapeDef, ShapeInst, ShapesBlock,
    StylesBlock, Value, WireDecl, WireOp, WiresBlock,
};
use crate::error::Error;
use crate::lexer;
use crate::parser;
use crate::span::Span;

const INDENT: &str = "  ";

pub fn format(src: &str) -> Result<String, Error> {
    let tokens = lexer::lex(src)?;
    let file = parser::parse(&tokens)?;
    let trivia = scan_trivia(src);
    let mut out = String::new();
    let mut emitter = Emitter {
        src,
        trivia: &trivia,
        cursor: 0,
        out: &mut out,
    };
    emitter.emit_file(&file);
    Ok(out)
}

// ─────────────────────────── Trivia (comments + blank lines) ───────────────────────────

#[derive(Debug, Clone)]
enum Trivia {
    Comment(String),
    BlankLine,
}

#[derive(Debug, Clone)]
struct TriviaToken {
    /// Position in source bytes where this trivia starts.
    pos: usize,
    kind: Trivia,
}

fn scan_trivia(src: &str) -> Vec<TriviaToken> {
    let mut out = Vec::new();
    let bytes = src.as_bytes();
    let mut i = 0;
    let mut at_line_start = true;
    let mut blank_run = 0usize; // count of consecutive newlines at line start

    while i < bytes.len() {
        let c = bytes[i];
        match c {
            b' ' | b'\t' | b'\r' => i += 1,
            b'\n' => {
                if at_line_start {
                    blank_run += 1;
                    if blank_run == 2 {
                        // First blank line in a run.
                        out.push(TriviaToken {
                            pos: i,
                            kind: Trivia::BlankLine,
                        });
                    }
                } else {
                    blank_run = 1;
                }
                at_line_start = true;
                i += 1;
            }
            b'/' if bytes.get(i + 1) == Some(&b'/') => {
                let start = i;
                i += 2;
                while i < bytes.len() && bytes[i] != b'\n' {
                    i += 1;
                }
                let text = src[start..i].trim_end().to_string();
                out.push(TriviaToken {
                    pos: start,
                    kind: Trivia::Comment(text),
                });
                at_line_start = false;
                blank_run = 0;
            }
            _ => {
                // Some non-trivia character — skip until end of line for our purposes.
                at_line_start = false;
                blank_run = 0;
                // Advance past non-trivia. We need to honor strings so // inside
                // a string isn't picked up. For simplicity, use the same single-
                // char advancement; strings are rare to contain `//` and the
                // lexer-level token spans give us safety from re-scanning.
                if c == b'"' {
                    i += 1;
                    while i < bytes.len() {
                        let cc = bytes[i];
                        if cc == b'\\' && i + 1 < bytes.len() {
                            i += 2;
                            continue;
                        }
                        i += 1;
                        if cc == b'"' {
                            break;
                        }
                    }
                } else {
                    i += 1;
                }
            }
        }
    }
    out
}

// ─────────────────────────── Emitter ───────────────────────────

struct Emitter<'a> {
    src: &'a str,
    trivia: &'a [TriviaToken],
    cursor: usize,
    out: &'a mut String,
}

impl<'a> Emitter<'a> {
    fn emit_file(&mut self, file: &File) {
        for (i, block) in file.blocks.iter().enumerate() {
            self.emit_trivia_before(block_span(block).start, /*indent*/ 0);
            if i > 0 && !self.out.ends_with("\n\n") {
                // Ensure exactly one blank line between blocks unless explicit trivia already provided.
                if self.out.ends_with('\n') {
                    self.out.push('\n');
                } else {
                    self.out.push_str("\n\n");
                }
            }
            self.emit_block(block);
            self.cursor = block_span(block).end;
        }
        // Trailing comments at end of file.
        self.emit_trivia_before(self.src.len(), 0);
        if !self.out.ends_with('\n') {
            self.out.push('\n');
        }
    }

    fn emit_block(&mut self, block: &Block) {
        match block {
            Block::Defaults(d) => self.emit_defaults(d),
            Block::Styles(s) => self.emit_styles(s),
            Block::Shapes(s) => self.emit_shapes(s),
            Block::Scene(s) => self.emit_scene(s),
            Block::Wires(w) => self.emit_wires(w),
        }
    }

    fn emit_defaults(&mut self, d: &DefaultsBlock) {
        self.out.push_str("defaults {\n");
        for entry in &d.entries {
            self.emit_trivia_before(entry.span.start, 1);
            self.indent(1);
            self.out.push_str(&entry.name);
            self.out.push('=');
            self.emit_value(&entry.value);
            self.out.push('\n');
            self.cursor = entry.span.end;
        }
        self.emit_trivia_before(d.span.end, 1);
        self.out.push_str("}\n");
    }

    fn emit_styles(&mut self, s: &StylesBlock) {
        self.out.push_str("styles ");
        if self.maybe_collapse_empty_body(&s.styles, s.span.end, 0) {
            return;
        }
        self.out.push_str("{\n");
        let widths = compute_style_widths(&s.styles, self.trivia);
        for (i, style) in s.styles.iter().enumerate() {
            self.emit_trivia_before(style.span.start, 1);
            self.indent(1);
            self.out.push_str(&style.name);
            pad(self.out, widths[i].saturating_sub(style.name.len()));
            self.emit_attr_items(&style.items);
            self.out.push('\n');
            self.cursor = style.span.end;
        }
        self.emit_trivia_before(s.span.end, 1);
        self.out.push_str("}\n");
    }

    fn emit_shapes(&mut self, s: &ShapesBlock) {
        self.out.push_str("shapes ");
        if self.maybe_collapse_empty_body(&s.shapes, s.span.end, 0) {
            return;
        }
        self.out.push_str("{\n");
        let widths = compute_shape_widths(&s.shapes, self.trivia);
        for (i, shape) in s.shapes.iter().enumerate() {
            self.emit_trivia_before(shape.span.start, 1);
            self.emit_shape_def(shape, 1, widths[i]);
            self.cursor = shape.span.end;
        }
        self.emit_trivia_before(s.span.end, 1);
        self.out.push_str("}\n");
    }

    fn emit_shape_def(&mut self, sd: &ShapeDef, depth: usize, w: ShapeWidths) {
        self.indent(depth);
        self.out.push_str(&sd.name);
        let has_more = sd.base.is_some() || !sd.items.is_empty() || sd.body.is_some();
        if has_more {
            pad(self.out, w.name.saturating_sub(sd.name.len()));
        }
        if let Some(base) = &sd.base {
            self.out.push_str(" :");
            self.out.push_str(&base.name);
            // pad type column only if attrs or body follow
            if !sd.items.is_empty() || sd.body.is_some() {
                let base_len = base.name.len() + 1; // +1 for ':'
                pad(self.out, w.ty.saturating_sub(base_len));
            }
        } else if w.ty > 0 && (!sd.items.is_empty() || sd.body.is_some()) {
            // No base, but other defs in group have one — reserve column.
            self.out.push(' ');
            pad(self.out, w.ty);
        }
        self.emit_attr_items(&sd.items);
        self.emit_body(&sd.body, sd.span.end, depth);
        self.out.push('\n');
    }

    fn emit_scene(&mut self, s: &SceneBlock) {
        self.out.push_str("scene");
        self.emit_attr_items(&s.items);
        self.out.push(' ');
        if self.maybe_collapse_empty_body(&s.body, s.span.end, 0) {
            return;
        }
        self.out.push_str("{\n");
        let widths = compute_node_widths(&s.body, self.trivia);
        for (i, inst) in s.body.iter().enumerate() {
            self.emit_trivia_before(inst.span.start, 1);
            self.emit_shape_inst(inst, 1, widths[i]);
            self.cursor = inst.span.end;
        }
        self.emit_trivia_before(s.span.end, 1);
        self.out.push_str("}\n");
    }

    fn emit_wires(&mut self, w: &WiresBlock) {
        self.out.push_str("wires");
        self.emit_attr_items(&w.items);
        self.out.push(' ');
        if self.maybe_collapse_empty_body(&w.wires, w.span.end, 0) {
            return;
        }
        self.out.push_str("{\n");
        for wire in &w.wires {
            self.emit_trivia_before(wire.span.start, 1);
            self.emit_wire(wire, 1);
            self.cursor = wire.span.end;
        }
        self.emit_trivia_before(w.span.end, 1);
        self.out.push_str("}\n");
    }

    fn emit_shape_inst(&mut self, inst: &ShapeInst, depth: usize, w: NodeWidths) {
        self.indent(depth);
        let has_label = inst.label.is_some();
        let has_attrs = !inst.items.is_empty();
        let has_body = inst.body.is_some();
        let anything_after_type = has_label || has_attrs || has_body;

        if w.id > 0 {
            match &inst.id {
                Some(id) => {
                    self.out.push_str(id);
                    pad(self.out, w.id.saturating_sub(id.len()));
                }
                None => pad(self.out, w.id),
            }
            self.out.push(' ');
        } else if let Some(id) = &inst.id {
            self.out.push_str(id);
            self.out.push(' ');
        }

        self.out.push(':');
        self.out.push_str(&inst.ty.name);
        if anything_after_type {
            let ty_len = inst.ty.name.len() + 1;
            pad(self.out, w.ty.saturating_sub(ty_len));
        }

        if let Some(label) = &inst.label {
            self.out.push(' ');
            self.emit_string(label);
        }
        self.emit_attr_items(&inst.items);
        self.emit_body(&inst.body, inst.span.end, depth);
        self.out.push('\n');
    }

    fn emit_body(&mut self, body: &Option<Vec<ShapeInst>>, end: usize, depth: usize) {
        let body = match body {
            Some(b) => b,
            None => return,
        };
        if body.is_empty() && !self.has_comment_in(self.cursor, end) {
            self.out.push_str(" {}");
            self.cursor = end;
            return;
        }
        self.out.push_str(" {\n");
        let widths = compute_node_widths(body, self.trivia);
        for (i, c) in body.iter().enumerate() {
            self.emit_trivia_before(c.span.start, depth + 1);
            self.emit_shape_inst(c, depth + 1, widths[i]);
            self.cursor = c.span.end;
        }
        self.emit_trivia_before(end, depth + 1);
        self.indent(depth);
        self.out.push('}');
    }

    fn maybe_collapse_empty_body<T>(&mut self, items: &[T], end: usize, _depth: usize) -> bool {
        if items.is_empty() && !self.has_comment_in(self.cursor, end) {
            self.out.push_str("{}\n");
            self.cursor = end;
            return true;
        }
        false
    }

    fn has_comment_in(&self, start: usize, end: usize) -> bool {
        self.trivia
            .iter()
            .any(|t| matches!(t.kind, Trivia::Comment(_)) && t.pos >= start && t.pos < end)
    }

    fn emit_wire(&mut self, w: &WireDecl, depth: usize) {
        self.indent(depth);
        for (i, ep) in w.endpoints.iter().enumerate() {
            if i > 0 {
                self.out.push(' ');
                self.out.push_str(wire_op_str(w.op));
                self.out.push(' ');
            }
            self.out.push_str(&ep.id);
            if let Some(a) = ep.anchor {
                self.out.push('[');
                self.out.push_str(anchor_str(a));
                self.out.push(']');
            }
        }
        if let Some(label) = &w.label {
            self.out.push(' ');
            self.emit_string(label);
        }
        self.emit_attr_items(&w.items);
        if let Some(body) = &w.body {
            self.out.push_str(" {\n");
            for t in body {
                self.emit_trivia_before(t.span.start, depth + 1);
                self.indent(depth + 1);
                self.out.push_str(":text ");
                self.emit_string(&t.text);
                self.emit_attr_items(&t.items);
                self.out.push('\n');
                self.cursor = t.span.end;
            }
            self.emit_trivia_before(w.span.end, depth + 1);
            self.indent(depth);
            self.out.push('}');
        }
        self.out.push('\n');
    }

    fn emit_attr_items(&mut self, items: &[AttrItem]) {
        for item in items {
            self.out.push(' ');
            match item {
                AttrItem::Style(s) => {
                    self.out.push('.');
                    self.out.push_str(&s.name);
                }
                AttrItem::Attr(a) => {
                    self.out.push_str(&a.name);
                    if let Some(v) = &a.value {
                        self.out.push('=');
                        self.emit_value(v);
                    }
                }
            }
        }
    }

    fn emit_value(&mut self, v: &Value) {
        match v {
            Value::Number(n) => {
                self.out.push_str(&format_number(*n));
            }
            Value::String(s) => self.emit_string(s),
            Value::Hex(h) => {
                self.out.push('#');
                self.out.push_str(h);
            }
            Value::Ident(s) => self.out.push_str(s),
            Value::Tuple(items) => {
                self.out.push('(');
                for (i, item) in items.iter().enumerate() {
                    if i > 0 {
                        self.out.push_str(", ");
                    }
                    self.emit_value(item);
                }
                self.out.push(')');
            }
            Value::List(items) => {
                self.out.push('[');
                for (i, item) in items.iter().enumerate() {
                    if i > 0 {
                        self.out.push_str(", ");
                    }
                    self.emit_value(item);
                }
                self.out.push(']');
            }
            Value::Call(c) => {
                self.out.push_str(&c.name);
                self.out.push('(');
                for (i, arg) in c.args.iter().enumerate() {
                    if i > 0 {
                        self.out.push_str(", ");
                    }
                    self.emit_value(arg);
                }
                self.out.push(')');
            }
            Value::RawCssVar(n) => {
                self.out.push_str("--");
                self.out.push_str(n);
            }
        }
    }

    fn emit_string(&mut self, s: &str) {
        self.out.push('"');
        for c in s.chars() {
            match c {
                '"' => self.out.push_str("\\\""),
                '\\' => self.out.push_str("\\\\"),
                '\n' => self.out.push_str("\\n"),
                '\t' => self.out.push_str("\\t"),
                _ => self.out.push(c),
            }
        }
        self.out.push('"');
    }

    fn indent(&mut self, depth: usize) {
        for _ in 0..depth {
            self.out.push_str(INDENT);
        }
    }

    /// Emit any comments and blank lines in [self.cursor, until). After this,
    /// self.cursor is left untouched — callers update it after emitting their
    /// own content.
    fn emit_trivia_before(&mut self, until: usize, depth: usize) {
        let mut last_was_blank = false;
        for t in self.trivia {
            if t.pos < self.cursor {
                continue;
            }
            if t.pos >= until {
                break;
            }
            match &t.kind {
                Trivia::Comment(text) => {
                    self.indent(depth);
                    self.out.push_str(text);
                    self.out.push('\n');
                    last_was_blank = false;
                }
                Trivia::BlankLine => {
                    if !last_was_blank && !self.out.ends_with("\n\n") {
                        self.out.push('\n');
                        last_was_blank = true;
                    }
                }
            }
        }
        self.cursor = until;
    }
}

// ─────────────────────────── Helpers ───────────────────────────

fn block_span(b: &Block) -> Span {
    match b {
        Block::Defaults(d) => d.span,
        Block::Styles(s) => s.span,
        Block::Shapes(s) => s.span,
        Block::Scene(s) => s.span,
        Block::Wires(w) => w.span,
    }
}

fn wire_op_str(op: WireOp) -> &'static str {
    op.as_str()
}

fn anchor_str(a: crate::ast::AnchorName) -> &'static str {
    use crate::ast::AnchorName::*;
    match a {
        Top => "top",
        Bottom => "bottom",
        Left => "left",
        Right => "right",
        TopLeft => "top-left",
        TopRight => "top-right",
        BottomLeft => "bottom-left",
        BottomRight => "bottom-right",
    }
}

// ─────────────────────────── Column alignment ───────────────────────────

#[derive(Default, Clone, Copy)]
struct NodeWidths {
    id: usize, // 0 if no ids in the group
    ty: usize, // includes leading ':'
}

#[derive(Default, Clone, Copy)]
struct ShapeWidths {
    name: usize,
    ty: usize, // includes leading ':' (0 if no shape in group has a base)
}

fn compute_node_widths(insts: &[ShapeInst], trivia: &[TriviaToken]) -> Vec<NodeWidths> {
    let groups = split_groups(insts, trivia, |i| i.span);
    let mut out = vec![NodeWidths::default(); insts.len()];
    for g in groups {
        let mut w = NodeWidths::default();
        for &i in &g {
            let inst = &insts[i];
            if let Some(id) = &inst.id {
                w.id = w.id.max(id.len());
            }
            w.ty = w.ty.max(inst.ty.name.len() + 1);
        }
        for i in g {
            out[i] = w;
        }
    }
    out
}

fn compute_style_widths(styles: &[crate::ast::StyleDef], trivia: &[TriviaToken]) -> Vec<usize> {
    let groups = split_groups(styles, trivia, |s| s.span);
    let mut out = vec![0usize; styles.len()];
    for g in groups {
        let mut w = 0usize;
        for &i in &g {
            w = w.max(styles[i].name.len());
        }
        for i in g {
            out[i] = w;
        }
    }
    out
}

fn compute_shape_widths(shapes: &[ShapeDef], trivia: &[TriviaToken]) -> Vec<ShapeWidths> {
    let groups = split_groups(shapes, trivia, |s| s.span);
    let mut out = vec![ShapeWidths::default(); shapes.len()];
    for g in groups {
        let mut w = ShapeWidths::default();
        for &i in &g {
            w.name = w.name.max(shapes[i].name.len());
            if let Some(b) = &shapes[i].base {
                w.ty = w.ty.max(b.name.len() + 1);
            }
        }
        for i in g {
            out[i] = w;
        }
    }
    out
}

/// Split a list of siblings into alignment groups separated by source-level
/// blank lines. Comments inside a contiguous run do NOT split the group.
fn split_groups<T>(
    items: &[T],
    trivia: &[TriviaToken],
    span_of: impl Fn(&T) -> Span,
) -> Vec<Vec<usize>> {
    if items.is_empty() {
        return Vec::new();
    }
    let mut groups: Vec<Vec<usize>> = vec![vec![0]];
    for i in 1..items.len() {
        let prev_end = span_of(&items[i - 1]).end;
        let curr_start = span_of(&items[i]).start;
        let has_blank = trivia.iter().any(|t| {
            matches!(t.kind, Trivia::BlankLine) && t.pos >= prev_end && t.pos < curr_start
        });
        if has_blank {
            groups.push(vec![i]);
        } else {
            groups.last_mut().unwrap().push(i);
        }
    }
    groups
}

fn pad(out: &mut String, n: usize) {
    for _ in 0..n {
        out.push(' ');
    }
}

fn format_number(n: f64) -> String {
    if n.fract() == 0.0 && n.is_finite() && n.abs() < 1e15 {
        format!("{}", n as i64)
    } else {
        let s = format!("{}", n);
        s
    }
}
