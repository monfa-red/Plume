use crate::ast::*;
use crate::error::Error;
use crate::lexer::{TokKind, Token};
use crate::span::Span;

pub fn parse(tokens: &[Token]) -> Result<File, Error> {
    let mut p = Parser {
        toks: tokens,
        pos: 0,
    };
    let mut blocks = Vec::new();
    p.skip_newlines();
    while p.peek().is_some() {
        blocks.push(p.parse_block()?);
        p.skip_newlines();
    }
    Ok(File { blocks })
}

/// Parse a single Plume value from a complete token stream. Used by the theme
/// loader to interpret `--plume-NAME: VALUE;` declarations as Plume values
/// (numbers, hex, tuples, calls, …) rather than opaque CSS strings.
pub fn parse_value_only(tokens: &[Token]) -> Result<Value, Error> {
    let mut p = Parser {
        toks: tokens,
        pos: 0,
    };
    p.skip_newlines();
    let v = p.parse_value()?;
    p.skip_newlines();
    if p.peek().is_some() {
        return Err(Error::at(p.next_span(), "trailing tokens after value"));
    }
    Ok(v)
}

struct Parser<'a> {
    toks: &'a [Token],
    pos: usize,
}

impl<'a> Parser<'a> {
    // ───────────────────────── Cursor helpers ─────────────────────────

    fn peek(&self) -> Option<&Token> {
        self.toks.get(self.pos)
    }

    fn peek_kind(&self) -> Option<&TokKind> {
        self.peek().map(|t| &t.kind)
    }

    fn next_span(&self) -> Span {
        self.peek()
            .map(|t| t.span)
            .unwrap_or_else(|| self.last_span())
    }

    fn last_span(&self) -> Span {
        self.toks
            .get(self.pos.saturating_sub(1))
            .map(|t| t.span)
            .unwrap_or_default()
    }

    fn skip_newlines(&mut self) {
        while matches!(self.peek_kind(), Some(TokKind::Newline)) {
            self.pos += 1;
        }
    }

    fn consume_terminator(&mut self) -> Result<(), Error> {
        match self.peek_kind() {
            Some(TokKind::Newline) | Some(TokKind::Semi) => {
                self.pos += 1;
                self.skip_newlines();
                Ok(())
            }
            Some(TokKind::RBrace) | None => Ok(()),
            Some(other) => Err(Error::at(
                self.next_span(),
                format!("expected newline, ';' or '}}', found {}", tok_desc(other)),
            )),
        }
    }

    fn expect_kind(&mut self, pred: impl Fn(&TokKind) -> bool, what: &str) -> Result<Span, Error> {
        match self.peek() {
            Some(t) if pred(&t.kind) => {
                let span = t.span;
                self.pos += 1;
                Ok(span)
            }
            Some(t) => Err(Error::at(
                t.span,
                format!("expected {}, found {}", what, tok_desc(&t.kind)),
            )),
            None => Err(Error::at(
                self.last_span(),
                format!("expected {}, found end of file", what),
            )),
        }
    }

    fn expect_lbrace(&mut self) -> Result<Span, Error> {
        self.expect_kind(|k| matches!(k, TokKind::LBrace), "'{'")
    }
    fn expect_rbrace(&mut self) -> Result<Span, Error> {
        self.expect_kind(|k| matches!(k, TokKind::RBrace), "'}'")
    }
    fn expect_lparen(&mut self) -> Result<Span, Error> {
        self.expect_kind(|k| matches!(k, TokKind::LParen), "'('")
    }
    fn expect_rparen(&mut self) -> Result<Span, Error> {
        self.expect_kind(|k| matches!(k, TokKind::RParen), "')'")
    }
    fn expect_lbracket(&mut self) -> Result<Span, Error> {
        self.expect_kind(|k| matches!(k, TokKind::LBracket), "'['")
    }
    fn expect_rbracket(&mut self) -> Result<Span, Error> {
        self.expect_kind(|k| matches!(k, TokKind::RBracket), "']'")
    }
    fn expect_colon(&mut self) -> Result<Span, Error> {
        self.expect_kind(|k| matches!(k, TokKind::Colon), "':'")
    }
    fn expect_equals(&mut self) -> Result<Span, Error> {
        self.expect_kind(|k| matches!(k, TokKind::Equals), "'='")
    }
    fn expect_dot(&mut self) -> Result<Span, Error> {
        self.expect_kind(|k| matches!(k, TokKind::Dot), "'.'")
    }

    fn expect_ident(&mut self) -> Result<(String, Span), Error> {
        match self.peek() {
            Some(Token {
                kind: TokKind::Ident(name),
                span,
            }) => {
                let out = (name.clone(), *span);
                self.pos += 1;
                Ok(out)
            }
            Some(t) => Err(Error::at(
                t.span,
                format!("expected identifier, found {}", tok_desc(&t.kind)),
            )),
            None => Err(Error::at(
                self.last_span(),
                "expected identifier, found end of file",
            )),
        }
    }

    fn expect_string(&mut self) -> Result<String, Error> {
        match self.peek() {
            Some(Token {
                kind: TokKind::String(s),
                ..
            }) => {
                let out = s.clone();
                self.pos += 1;
                Ok(out)
            }
            Some(t) => Err(Error::at(
                t.span,
                format!("expected string, found {}", tok_desc(&t.kind)),
            )),
            None => Err(Error::at(
                self.last_span(),
                "expected string, found end of file",
            )),
        }
    }

    fn eat_string(&mut self) -> Option<String> {
        if let Some(Token {
            kind: TokKind::String(s),
            ..
        }) = self.peek()
        {
            let out = s.clone();
            self.pos += 1;
            Some(out)
        } else {
            None
        }
    }

    // ───────────────────────── Values ─────────────────────────

    fn parse_value(&mut self) -> Result<Value, Error> {
        let tok = match self.peek() {
            Some(t) => t,
            None => {
                return Err(Error::at(
                    self.last_span(),
                    "expected value, found end of file",
                ));
            }
        };
        match &tok.kind {
            TokKind::Number(n) => {
                let v = *n;
                self.pos += 1;
                Ok(Value::Number(v))
            }
            TokKind::String(s) => {
                let v = s.clone();
                self.pos += 1;
                Ok(Value::String(v))
            }
            TokKind::Hex(h) => {
                let v = h.clone();
                self.pos += 1;
                Ok(Value::Hex(v))
            }
            TokKind::Ident(_) => {
                let (name, name_span) = self.expect_ident()?;
                if matches!(self.peek_kind(), Some(TokKind::LParen)) {
                    Ok(Value::Call(self.parse_call(name, name_span)?))
                } else {
                    Ok(Value::Ident(name))
                }
            }
            TokKind::LParen => self.parse_tuple_value(),
            TokKind::LBracket => self.parse_list_value(),
            TokKind::RawCssVar(_) => Err(Error::at(
                tok.span,
                "raw CSS variable ('--name') only valid as an argument to var()",
            )),
            other => Err(Error::at(
                tok.span,
                format!("expected value, found {}", tok_desc(other)),
            )),
        }
    }

    fn parse_call(&mut self, name: String, name_span: Span) -> Result<FnCall, Error> {
        self.expect_lparen()?;
        let mut args = Vec::new();
        if !matches!(self.peek_kind(), Some(TokKind::RParen)) {
            args.push(self.parse_call_arg(&name)?);
            while matches!(self.peek_kind(), Some(TokKind::Comma)) {
                self.pos += 1;
                args.push(self.parse_call_arg(&name)?);
            }
        }
        let end = self.expect_rparen()?;
        Ok(FnCall {
            name,
            args,
            span: Span::new(name_span.start, end.end),
        })
    }

    fn parse_call_arg(&mut self, fn_name: &str) -> Result<Value, Error> {
        // Inside var(), allow `--name` raw CSS var.
        if let Some(Token {
            kind: TokKind::RawCssVar(s),
            span,
        }) = self.peek()
        {
            if fn_name != "var" {
                return Err(Error::at(
                    *span,
                    format!(
                        "raw CSS variable only valid inside var(), not {}()",
                        fn_name
                    ),
                ));
            }
            let v = s.clone();
            self.pos += 1;
            return Ok(Value::RawCssVar(v));
        }
        self.parse_value()
    }

    fn parse_tuple_value(&mut self) -> Result<Value, Error> {
        self.expect_lparen()?;
        let mut items = Vec::new();
        if !matches!(self.peek_kind(), Some(TokKind::RParen)) {
            items.push(self.parse_value()?);
            while matches!(self.peek_kind(), Some(TokKind::Comma)) {
                self.pos += 1;
                items.push(self.parse_value()?);
            }
        }
        self.expect_rparen()?;
        Ok(Value::Tuple(items))
    }

    fn parse_list_value(&mut self) -> Result<Value, Error> {
        self.expect_lbracket()?;
        let mut items = Vec::new();
        if !matches!(self.peek_kind(), Some(TokKind::RBracket)) {
            items.push(self.parse_value()?);
            while matches!(self.peek_kind(), Some(TokKind::Comma)) {
                self.pos += 1;
                items.push(self.parse_value()?);
            }
        }
        self.expect_rbracket()?;
        Ok(Value::List(items))
    }

    // ───────────────────────── Attr items ─────────────────────────

    fn parse_attr_items(&mut self) -> Result<Vec<AttrItem>, Error> {
        let mut items = Vec::new();
        loop {
            match self.peek_kind() {
                Some(TokKind::Dot) => items.push(AttrItem::Style(self.parse_style_ref()?)),
                Some(TokKind::Ident(_)) => items.push(AttrItem::Attr(self.parse_attr()?)),
                _ => break,
            }
        }
        Ok(items)
    }

    fn parse_style_ref(&mut self) -> Result<StyleRef, Error> {
        let start = self.expect_dot()?;
        let (name, end) = self.expect_ident()?;
        Ok(StyleRef {
            name,
            span: Span::new(start.start, end.end),
        })
    }

    fn parse_attr(&mut self) -> Result<Attr, Error> {
        let (name, name_span) = self.expect_ident()?;
        let value = if matches!(self.peek_kind(), Some(TokKind::Equals)) {
            self.pos += 1;
            Some(self.parse_value()?)
        } else {
            None
        };
        let end = self.last_span();
        Ok(Attr {
            name,
            value,
            span: Span::new(name_span.start, end.end),
        })
    }

    fn parse_type_ref(&mut self) -> Result<TypeRef, Error> {
        let start = self.expect_colon()?;
        let (name, end) = self.expect_ident()?;
        Ok(TypeRef {
            name,
            span: Span::new(start.start, end.end),
        })
    }

    // ───────────────────────── Top-level block ─────────────────────────

    fn parse_block(&mut self) -> Result<Block, Error> {
        let (name, name_span) = self.expect_ident()?;
        match name.as_str() {
            "defaults" => {
                self.expect_lbrace()?;
                let entries = self.parse_defaults_body()?;
                let end = self.expect_rbrace()?;
                Ok(Block::Defaults(DefaultsBlock {
                    entries,
                    span: Span::new(name_span.start, end.end),
                }))
            }
            "styles" => {
                self.expect_lbrace()?;
                let styles = self.parse_styles_body()?;
                let end = self.expect_rbrace()?;
                Ok(Block::Styles(StylesBlock {
                    styles,
                    span: Span::new(name_span.start, end.end),
                }))
            }
            "shapes" => {
                self.expect_lbrace()?;
                let shapes = self.parse_shapes_body()?;
                let end = self.expect_rbrace()?;
                Ok(Block::Shapes(ShapesBlock {
                    shapes,
                    span: Span::new(name_span.start, end.end),
                }))
            }
            "scene" => {
                let items = self.parse_attr_items()?;
                self.expect_lbrace()?;
                let body = self.parse_inst_body_items()?;
                let end = self.expect_rbrace()?;
                Ok(Block::Scene(SceneBlock {
                    items,
                    body,
                    span: Span::new(name_span.start, end.end),
                }))
            }
            "wires" => {
                let items = self.parse_attr_items()?;
                self.expect_lbrace()?;
                let wires = self.parse_wires_body()?;
                let end = self.expect_rbrace()?;
                Ok(Block::Wires(WiresBlock {
                    items,
                    wires,
                    span: Span::new(name_span.start, end.end),
                }))
            }
            other => Err(Error::at(
                name_span,
                format!(
                    "unknown top-level block '{}' (expected defaults, styles, shapes, scene, wires)",
                    other
                ),
            )),
        }
    }

    // ───────────────────────── Block bodies ─────────────────────────

    fn parse_defaults_body(&mut self) -> Result<Vec<DefaultEntry>, Error> {
        let mut entries = Vec::new();
        self.skip_newlines();
        while !matches!(self.peek_kind(), Some(TokKind::RBrace) | None) {
            entries.push(self.parse_default_entry()?);
            self.consume_terminator()?;
        }
        Ok(entries)
    }

    fn parse_default_entry(&mut self) -> Result<DefaultEntry, Error> {
        let (name, name_span) = self.expect_ident()?;
        self.expect_equals()?;
        let value = self.parse_value()?;
        let end = self.last_span();
        Ok(DefaultEntry {
            name,
            value,
            span: Span::new(name_span.start, end.end),
        })
    }

    fn parse_styles_body(&mut self) -> Result<Vec<StyleDef>, Error> {
        let mut styles = Vec::new();
        self.skip_newlines();
        while !matches!(self.peek_kind(), Some(TokKind::RBrace) | None) {
            styles.push(self.parse_style_def()?);
            self.consume_terminator()?;
        }
        Ok(styles)
    }

    fn parse_style_def(&mut self) -> Result<StyleDef, Error> {
        let (name, name_span) = self.expect_ident()?;
        let items = self.parse_attr_items()?;
        let end = self.last_span();
        Ok(StyleDef {
            name,
            items,
            span: Span::new(name_span.start, end.end),
        })
    }

    fn parse_shapes_body(&mut self) -> Result<Vec<ShapeDef>, Error> {
        let mut shapes = Vec::new();
        self.skip_newlines();
        while !matches!(self.peek_kind(), Some(TokKind::RBrace) | None) {
            shapes.push(self.parse_shape_def()?);
            self.consume_terminator()?;
        }
        Ok(shapes)
    }

    fn parse_shape_def(&mut self) -> Result<ShapeDef, Error> {
        let (name, name_span) = self.expect_ident()?;
        let base = if matches!(self.peek_kind(), Some(TokKind::Colon)) {
            Some(self.parse_type_ref()?)
        } else {
            None
        };
        let items = self.parse_attr_items()?;
        let body = if matches!(self.peek_kind(), Some(TokKind::LBrace)) {
            Some(self.parse_inst_body()?)
        } else {
            None
        };
        if base.is_none() && body.is_none() {
            return Err(Error::at(
                name_span,
                format!("shape '{}' requires :base or a body", name),
            ));
        }
        let end = self.last_span();
        Ok(ShapeDef {
            name,
            base,
            items,
            body,
            span: Span::new(name_span.start, end.end),
        })
    }

    fn parse_inst_body(&mut self) -> Result<Vec<ShapeInst>, Error> {
        self.expect_lbrace()?;
        let items = self.parse_inst_body_items()?;
        self.expect_rbrace()?;
        Ok(items)
    }

    fn parse_inst_body_items(&mut self) -> Result<Vec<ShapeInst>, Error> {
        let mut items = Vec::new();
        self.skip_newlines();
        while !matches!(self.peek_kind(), Some(TokKind::RBrace) | None) {
            items.push(self.parse_shape_inst()?);
            self.consume_terminator()?;
        }
        Ok(items)
    }

    fn parse_shape_inst(&mut self) -> Result<ShapeInst, Error> {
        let start = self.next_span();
        let id = if matches!(self.peek_kind(), Some(TokKind::Ident(_))) {
            Some(self.expect_ident()?.0)
        } else {
            None
        };
        let ty = self.parse_type_ref()?;
        let label = self.eat_string();
        let items = self.parse_attr_items()?;
        let body = if matches!(self.peek_kind(), Some(TokKind::LBrace)) {
            Some(self.parse_inst_body()?)
        } else {
            None
        };
        let end = self.last_span();
        Ok(ShapeInst {
            id,
            ty,
            label,
            items,
            body,
            span: Span::new(start.start, end.end),
        })
    }

    // ───────────────────────── Wires ─────────────────────────

    fn parse_wires_body(&mut self) -> Result<Vec<WireDecl>, Error> {
        let mut wires = Vec::new();
        self.skip_newlines();
        while !matches!(self.peek_kind(), Some(TokKind::RBrace) | None) {
            wires.push(self.parse_wire_decl()?);
            self.consume_terminator()?;
        }
        Ok(wires)
    }

    fn parse_wire_decl(&mut self) -> Result<WireDecl, Error> {
        let start = self.next_span();
        let mut endpoints = vec![self.parse_wire_endpoint()?];
        let op = self.parse_wire_op()?;
        endpoints.push(self.parse_wire_endpoint()?);

        loop {
            match self.try_peek_wire_op() {
                Some(next_op) if next_op == op => {
                    self.pos += 1;
                    endpoints.push(self.parse_wire_endpoint()?);
                }
                Some(next_op) => {
                    return Err(Error::at(
                        self.next_span(),
                        format!(
                            "wire chain mixes operators '{}' and '{}'",
                            op.as_str(),
                            next_op.as_str()
                        ),
                    ));
                }
                None => break,
            }
        }

        let label = self.eat_string();
        let items = self.parse_attr_items()?;
        let body = if matches!(self.peek_kind(), Some(TokKind::LBrace)) {
            Some(self.parse_wire_text_body()?)
        } else {
            None
        };
        let end = self.last_span();
        Ok(WireDecl {
            endpoints,
            op,
            label,
            items,
            body,
            span: Span::new(start.start, end.end),
        })
    }

    fn parse_wire_endpoint(&mut self) -> Result<WireEndpoint, Error> {
        let (id, id_span) = self.expect_ident()?;
        let anchor = if matches!(self.peek_kind(), Some(TokKind::LBracket)) {
            self.pos += 1;
            let (name, name_span) = self.expect_ident()?;
            let parsed = AnchorName::parse(&name).ok_or_else(|| {
                Error::at(
                    name_span,
                    format!(
                        "wire endpoint anchor '{}' must be top/bottom/left/right or a corner",
                        name
                    ),
                )
            })?;
            self.expect_rbracket()?;
            Some(parsed)
        } else {
            None
        };
        let end = self.last_span();
        Ok(WireEndpoint {
            id,
            anchor,
            span: Span::new(id_span.start, end.end),
        })
    }

    fn try_peek_wire_op(&self) -> Option<WireOp> {
        match self.peek_kind() {
            Some(TokKind::Arrow) => Some(WireOp::Arrow),
            Some(TokKind::LArrow) => Some(WireOp::LArrow),
            Some(TokKind::Biarrow) => Some(WireOp::Biarrow),
            Some(TokKind::ArrowDash) => Some(WireOp::ArrowDash),
            Some(TokKind::LArrowDash) => Some(WireOp::LArrowDash),
            Some(TokKind::BiarrowDash) => Some(WireOp::BiarrowDash),
            Some(TokKind::ArrowDot) => Some(WireOp::ArrowDot),
            Some(TokKind::LArrowDot) => Some(WireOp::LArrowDot),
            Some(TokKind::BiarrowDot) => Some(WireOp::BiarrowDot),
            _ => None,
        }
    }

    fn parse_wire_op(&mut self) -> Result<WireOp, Error> {
        match self.try_peek_wire_op() {
            Some(op) => {
                self.pos += 1;
                Ok(op)
            }
            None => Err(Error::at(
                self.next_span(),
                format!(
                    "expected wire operator, found {}",
                    self.peek_kind().map_or("end of file".to_string(), tok_desc)
                ),
            )),
        }
    }

    fn parse_wire_text_body(&mut self) -> Result<Vec<TextDecl>, Error> {
        self.expect_lbrace()?;
        self.skip_newlines();
        let mut texts = Vec::new();
        while !matches!(self.peek_kind(), Some(TokKind::RBrace) | None) {
            let start = self.next_span();
            self.expect_colon()?;
            let (kw, kw_span) = self.expect_ident()?;
            if kw != "text" {
                return Err(Error::at(
                    kw_span,
                    "wire body may only contain :text primitives",
                ));
            }
            let text = self.expect_string()?;
            let items = self.parse_attr_items()?;
            let end = self.last_span();
            texts.push(TextDecl {
                text,
                items,
                span: Span::new(start.start, end.end),
            });
            self.consume_terminator()?;
        }
        self.expect_rbrace()?;
        Ok(texts)
    }
}

fn tok_desc(k: &TokKind) -> String {
    match k {
        TokKind::Ident(s) => format!("identifier '{}'", s),
        TokKind::String(_) => "string".to_string(),
        TokKind::Number(_) => "number".to_string(),
        TokKind::Hex(_) => "hex color".to_string(),
        TokKind::RawCssVar(s) => format!("'--{}'", s),
        TokKind::Colon => "':'".to_string(),
        TokKind::Dot => "'.'".to_string(),
        TokKind::Equals => "'='".to_string(),
        TokKind::Semi => "';'".to_string(),
        TokKind::Comma => "','".to_string(),
        TokKind::LBrace => "'{'".to_string(),
        TokKind::RBrace => "'}'".to_string(),
        TokKind::LParen => "'('".to_string(),
        TokKind::RParen => "')'".to_string(),
        TokKind::LBracket => "'['".to_string(),
        TokKind::RBracket => "']'".to_string(),
        TokKind::Arrow => "'->'".to_string(),
        TokKind::LArrow => "'<-'".to_string(),
        TokKind::Biarrow => "'<->'".to_string(),
        TokKind::ArrowDash => "'-->'".to_string(),
        TokKind::LArrowDash => "'<--'".to_string(),
        TokKind::BiarrowDash => "'<-->'".to_string(),
        TokKind::ArrowDot => "'-.->'".to_string(),
        TokKind::LArrowDot => "'<-.-'".to_string(),
        TokKind::BiarrowDot => "'<-.->'".to_string(),
        TokKind::Newline => "newline".to_string(),
    }
}
