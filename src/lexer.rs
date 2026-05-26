use crate::error::Error;
use crate::span::Span;

#[derive(Debug, Clone, PartialEq)]
pub enum TokKind {
    Ident(String),
    String(String),
    Number(f64),
    Hex(String),       // hex digits without leading '#'
    RawCssVar(String), // CSS var name without leading '--'

    Colon,
    Dot,
    Equals,
    Semi,
    Comma,
    LBrace,
    RBrace,
    LParen,
    RParen,
    LBracket,
    RBracket,

    Arrow,       // ->
    LArrow,      // <-
    Biarrow,     // <->
    ArrowDash,   // -->
    LArrowDash,  // <--
    BiarrowDash, // <-->
    ArrowDot,    // -.->
    LArrowDot,   // <-.-
    BiarrowDot,  // <-.->

    Newline,
}

#[derive(Debug, Clone)]
pub struct Token {
    pub kind: TokKind,
    pub span: Span,
}

pub fn lex(src: &str) -> Result<Vec<Token>, Error> {
    let mut lexer = Lexer {
        src,
        bytes: src.as_bytes(),
        i: 0,
        paren_depth: 0,
        tokens: Vec::new(),
    };
    lexer.run()?;
    Ok(lexer.tokens)
}

struct Lexer<'a> {
    src: &'a str,
    bytes: &'a [u8],
    i: usize,
    paren_depth: usize,
    tokens: Vec<Token>,
}

impl<'a> Lexer<'a> {
    fn run(&mut self) -> Result<(), Error> {
        while self.i < self.bytes.len() {
            let c = self.bytes[self.i];

            match c {
                b' ' | b'\t' | b'\r' => self.i += 1,
                b'\n' => self.handle_newline(),
                b'/' if self.peek(1) == Some(b'/') => self.skip_line_comment(),
                b'{' => self.push_punct(TokKind::LBrace, 1),
                b'}' => self.push_punct(TokKind::RBrace, 1),
                b'(' => {
                    self.paren_depth += 1;
                    self.push_punct(TokKind::LParen, 1);
                }
                b')' => {
                    self.paren_depth = self.paren_depth.saturating_sub(1);
                    self.push_punct(TokKind::RParen, 1);
                }
                b'[' => {
                    self.paren_depth += 1;
                    self.push_punct(TokKind::LBracket, 1);
                }
                b']' => {
                    self.paren_depth = self.paren_depth.saturating_sub(1);
                    self.push_punct(TokKind::RBracket, 1);
                }
                b':' => self.push_punct(TokKind::Colon, 1),
                b'=' => self.push_punct(TokKind::Equals, 1),
                b';' => self.push_punct(TokKind::Semi, 1),
                b',' => self.push_punct(TokKind::Comma, 1),
                b'"' => self.lex_string()?,
                b'#' => self.lex_hex()?,
                b'.' => {
                    if self.peek(1).is_some_and(|c| c.is_ascii_digit()) {
                        self.lex_number()?;
                    } else {
                        self.push_punct(TokKind::Dot, 1);
                    }
                }
                b'-' | b'<' => self.lex_dash_or_arrow()?,
                b'+' => self.lex_number()?,
                d if d.is_ascii_digit() => self.lex_number()?,
                c if is_ident_start(c) => self.lex_ident(),
                _ => {
                    return Err(Error::at(
                        Span::new(self.i, self.i + 1),
                        format!("unexpected character {:?}", c as char),
                    ));
                }
            }
        }
        Ok(())
    }

    fn peek(&self, n: usize) -> Option<u8> {
        self.bytes.get(self.i + n).copied()
    }

    fn push_punct(&mut self, kind: TokKind, len: usize) {
        let span = Span::new(self.i, self.i + len);
        self.tokens.push(Token { kind, span });
        self.i += len;
    }

    fn handle_newline(&mut self) {
        let start = self.i;
        self.i += 1;
        // Collapse following whitespace/newlines into one logical break.
        while self.i < self.bytes.len() {
            let c = self.bytes[self.i];
            if c == b' ' || c == b'\t' || c == b'\r' || c == b'\n' {
                self.i += 1;
            } else {
                break;
            }
        }
        if self.paren_depth == 0 {
            self.tokens.push(Token {
                kind: TokKind::Newline,
                span: Span::new(start, start + 1),
            });
        }
    }

    fn skip_line_comment(&mut self) {
        while self.i < self.bytes.len() && self.bytes[self.i] != b'\n' {
            self.i += 1;
        }
    }

    fn lex_string(&mut self) -> Result<(), Error> {
        let start = self.i;
        self.i += 1; // opening quote
        let mut value = String::new();

        while self.i < self.bytes.len() {
            let b = self.bytes[self.i];
            if b == b'"' {
                self.i += 1;
                self.tokens.push(Token {
                    kind: TokKind::String(value),
                    span: Span::new(start, self.i),
                });
                return Ok(());
            }
            if b == b'\\' {
                let esc_start = self.i;
                self.i += 1;
                let next = self.bytes.get(self.i).copied().ok_or_else(|| {
                    Error::at(Span::new(esc_start, self.i), "unterminated escape sequence")
                })?;
                match next {
                    b'"' => value.push('"'),
                    b'\\' => value.push('\\'),
                    b'n' => value.push('\n'),
                    b't' => value.push('\t'),
                    other => {
                        return Err(Error::at(
                            Span::new(esc_start, self.i + 1),
                            format!("invalid escape sequence '\\{}'", other as char),
                        ));
                    }
                }
                self.i += 1;
                continue;
            }
            let ch = self.src[self.i..].chars().next().expect("non-empty utf-8");
            value.push(ch);
            self.i += ch.len_utf8();
        }

        Err(Error::at(
            Span::new(start, self.i),
            "unterminated string literal",
        ))
    }

    fn lex_hex(&mut self) -> Result<(), Error> {
        let start = self.i;
        self.i += 1; // '#'
        let digits_start = self.i;
        while self.i < self.bytes.len() && self.bytes[self.i].is_ascii_hexdigit() {
            self.i += 1;
        }
        let len = self.i - digits_start;
        if !matches!(len, 3 | 6 | 8) {
            return Err(Error::at(
                Span::new(start, self.i),
                format!("invalid hex color '{}'", &self.src[start..self.i]),
            ));
        }
        let digits = self.src[digits_start..self.i].to_string();
        self.tokens.push(Token {
            kind: TokKind::Hex(digits),
            span: Span::new(start, self.i),
        });
        Ok(())
    }

    fn lex_dash_or_arrow(&mut self) -> Result<(), Error> {
        if let Some((kind, len)) = self.try_wire_op() {
            let start = self.i;
            self.tokens.push(Token {
                kind,
                span: Span::new(start, start + len),
            });
            self.i += len;
            return Ok(());
        }

        // `--` followed by ident start → RawCssVar
        if self.bytes[self.i] == b'-'
            && self.peek(1) == Some(b'-')
            && self.peek(2).is_some_and(is_ident_start)
        {
            return self.lex_raw_css_var();
        }

        // Signed number: '-' followed by digit, or '-.<digit>'
        if self.bytes[self.i] == b'-' {
            let next = self.peek(1);
            if next.is_some_and(|c| c.is_ascii_digit()) {
                return self.lex_number();
            }
            if next == Some(b'.') && self.peek(2).is_some_and(|c| c.is_ascii_digit()) {
                return self.lex_number();
            }
        }

        Err(Error::at(
            Span::new(self.i, self.i + 1),
            format!("unexpected character {:?}", self.bytes[self.i] as char),
        ))
    }

    fn try_wire_op(&self) -> Option<(TokKind, usize)> {
        let rest = &self.src[self.i..];
        // Longest-match first.
        for (pat, kind) in WIRE_OPS {
            if rest.starts_with(pat) {
                return Some((kind.clone(), pat.len()));
            }
        }
        None
    }

    fn lex_raw_css_var(&mut self) -> Result<(), Error> {
        let start = self.i;
        self.i += 2; // skip '--'
        let name_start = self.i;
        while self.i < self.bytes.len() && is_ident_continue(self.bytes[self.i]) {
            self.i += 1;
        }
        let name = self.src[name_start..self.i].to_string();
        self.tokens.push(Token {
            kind: TokKind::RawCssVar(name),
            span: Span::new(start, self.i),
        });
        Ok(())
    }

    fn lex_number(&mut self) -> Result<(), Error> {
        let start = self.i;

        if matches!(self.bytes[self.i], b'+' | b'-') {
            self.i += 1;
        }

        let mut saw_digit = false;
        while self.i < self.bytes.len() && self.bytes[self.i].is_ascii_digit() {
            self.i += 1;
            saw_digit = true;
        }
        if self.i < self.bytes.len()
            && self.bytes[self.i] == b'.'
            && self.peek(1).is_some_and(|c| c.is_ascii_digit())
        {
            self.i += 1; // '.'
            while self.i < self.bytes.len() && self.bytes[self.i].is_ascii_digit() {
                self.i += 1;
                saw_digit = true;
            }
        }

        if !saw_digit {
            return Err(Error::at(
                Span::new(start, self.i),
                "invalid number literal",
            ));
        }

        let text = &self.src[start..self.i];
        let value: f64 = text.parse().map_err(|_| {
            Error::at(
                Span::new(start, self.i),
                format!("invalid number literal '{}'", text),
            )
        })?;
        self.tokens.push(Token {
            kind: TokKind::Number(value),
            span: Span::new(start, self.i),
        });
        Ok(())
    }

    fn lex_ident(&mut self) {
        let start = self.i;
        while self.i < self.bytes.len() && is_ident_continue(self.bytes[self.i]) {
            self.i += 1;
        }
        let name = self.src[start..self.i].to_string();
        self.tokens.push(Token {
            kind: TokKind::Ident(name),
            span: Span::new(start, self.i),
        });
    }
}

static WIRE_OPS: &[(&str, TokKind)] = &[
    ("<-.->", TokKind::BiarrowDot),
    ("<-.-", TokKind::LArrowDot),
    ("-.->", TokKind::ArrowDot),
    ("<-->", TokKind::BiarrowDash),
    ("-->", TokKind::ArrowDash),
    ("<--", TokKind::LArrowDash),
    ("<->", TokKind::Biarrow),
    ("->", TokKind::Arrow),
    ("<-", TokKind::LArrow),
];

fn is_ident_start(c: u8) -> bool {
    c.is_ascii_alphabetic() || c == b'_'
}

fn is_ident_continue(c: u8) -> bool {
    c.is_ascii_alphanumeric() || c == b'_' || c == b'-'
}
