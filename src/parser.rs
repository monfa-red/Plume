use crate::ast::{Decl, DeclKind, File, Scene};
use crate::error::Error;
use crate::lexer::{TokKind, Token};
use crate::span::Span;

struct Parser<'a> {
    toks: &'a [Token],
    pos: usize,
}

pub fn parse(tokens: &[Token]) -> Result<File, Error> {
    let mut p = Parser {
        toks: tokens,
        pos: 0,
    };
    let mut scene = None;

    while let Some(tok) = p.peek() {
        match &tok.kind {
            TokKind::Ident(name) if name == "scene" => {
                if scene.is_some() {
                    return Err(Error::at(tok.span, "duplicate 'scene' block"));
                }
                scene = Some(p.parse_scene()?);
            }
            TokKind::Ident(name) => {
                return Err(Error::at(
                    tok.span,
                    format!("unexpected '{}' at top level (expected 'scene')", name),
                ));
            }
            _ => {
                return Err(Error::at(
                    tok.span,
                    format!("expected 'scene', found {}", tok_desc(&tok.kind)),
                ));
            }
        }
    }

    Ok(File { scene })
}

impl<'a> Parser<'a> {
    fn peek(&self) -> Option<&Token> {
        self.toks.get(self.pos)
    }

    fn bump(&mut self) -> Option<&Token> {
        let t = self.toks.get(self.pos);
        if t.is_some() {
            self.pos += 1;
        }
        t
    }

    fn last_span(&self) -> Span {
        self.toks
            .get(self.pos.saturating_sub(1))
            .map(|t| t.span)
            .unwrap_or_default()
    }

    fn next_span(&self) -> Span {
        self.toks
            .get(self.pos)
            .map(|t| t.span)
            .unwrap_or_else(|| self.last_span())
    }

    fn expect_lbrace(&mut self) -> Result<Span, Error> {
        self.expect(|k| matches!(k, TokKind::LBrace), "{")
    }

    fn expect_rbrace(&mut self) -> Result<Span, Error> {
        self.expect(|k| matches!(k, TokKind::RBrace), "}")
    }

    fn expect_colon(&mut self) -> Result<Span, Error> {
        self.expect(|k| matches!(k, TokKind::Colon), ":")
    }

    fn expect(&mut self, pred: impl Fn(&TokKind) -> bool, what: &str) -> Result<Span, Error> {
        match self.peek() {
            Some(t) if pred(&t.kind) => {
                let span = t.span;
                self.pos += 1;
                Ok(span)
            }
            Some(t) => Err(Error::at(
                t.span,
                format!("expected '{}', found {}", what, tok_desc(&t.kind)),
            )),
            None => Err(Error::at(
                self.last_span(),
                format!("expected '{}', found end of file", what),
            )),
        }
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

    fn eat_string(&mut self) -> Option<String> {
        match self.peek() {
            Some(Token {
                kind: TokKind::String(s),
                ..
            }) => {
                let out = s.clone();
                self.pos += 1;
                Some(out)
            }
            _ => None,
        }
    }

    fn parse_scene(&mut self) -> Result<Scene, Error> {
        self.bump(); // 'scene'
        self.expect_lbrace()?;

        let mut items = Vec::new();
        loop {
            match self.peek() {
                Some(Token {
                    kind: TokKind::RBrace,
                    ..
                })
                | None => break,
                _ => items.push(self.parse_decl()?),
            }
        }

        self.expect_rbrace()?;
        Ok(Scene { items })
    }

    fn parse_decl(&mut self) -> Result<Decl, Error> {
        let start = self.next_span();
        self.expect_colon()?;
        let (ty, _) = self.expect_ident()?;
        let label = self.eat_string();
        let end = self.last_span();
        Ok(Decl {
            kind: DeclKind::Primitive { ty, label },
            span: Span::new(start.start, end.end),
        })
    }
}

fn tok_desc(k: &TokKind) -> String {
    match k {
        TokKind::Ident(s) => format!("identifier '{}'", s),
        TokKind::String(_) => "string".to_string(),
        TokKind::Colon => "':'".to_string(),
        TokKind::LBrace => "'{'".to_string(),
        TokKind::RBrace => "'}'".to_string(),
    }
}
