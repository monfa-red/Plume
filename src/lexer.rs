use crate::error::Error;
use crate::span::Span;

#[derive(Debug, Clone, PartialEq)]
pub enum TokKind {
    Ident(String),
    String(String),
    Colon,
    LBrace,
    RBrace,
}

#[derive(Debug, Clone)]
pub struct Token {
    pub kind: TokKind,
    pub span: Span,
}

pub fn lex(src: &str) -> Result<Vec<Token>, Error> {
    let bytes = src.as_bytes();
    let mut i = 0;
    let mut toks = Vec::new();

    while i < bytes.len() {
        let c = bytes[i];

        if c == b' ' || c == b'\t' || c == b'\n' || c == b'\r' {
            i += 1;
            continue;
        }

        if c == b'/' && i + 1 < bytes.len() && bytes[i + 1] == b'/' {
            while i < bytes.len() && bytes[i] != b'\n' {
                i += 1;
            }
            continue;
        }

        match c {
            b'{' => {
                toks.push(Token {
                    kind: TokKind::LBrace,
                    span: Span::new(i, i + 1),
                });
                i += 1;
            }
            b'}' => {
                toks.push(Token {
                    kind: TokKind::RBrace,
                    span: Span::new(i, i + 1),
                });
                i += 1;
            }
            b':' => {
                toks.push(Token {
                    kind: TokKind::Colon,
                    span: Span::new(i, i + 1),
                });
                i += 1;
            }
            b'"' => {
                let (tok, next) = lex_string(src, i)?;
                toks.push(tok);
                i = next;
            }
            _ if is_ident_start(c) => {
                let start = i;
                while i < bytes.len() && is_ident_continue(bytes[i]) {
                    i += 1;
                }
                let name = src[start..i].to_string();
                toks.push(Token {
                    kind: TokKind::Ident(name),
                    span: Span::new(start, i),
                });
            }
            _ => {
                return Err(Error::at(
                    Span::new(i, i + 1),
                    format!("unexpected character {:?}", c as char),
                ));
            }
        }
    }

    Ok(toks)
}

fn lex_string(src: &str, start: usize) -> Result<(Token, usize), Error> {
    debug_assert_eq!(src.as_bytes()[start], b'"');
    let mut i = start + 1;
    let mut value = String::new();
    let bytes = src.as_bytes();

    while i < bytes.len() {
        let b = bytes[i];
        if b == b'"' {
            return Ok((
                Token {
                    kind: TokKind::String(value),
                    span: Span::new(start, i + 1),
                },
                i + 1,
            ));
        }
        if b == b'\\' {
            if i + 1 >= bytes.len() {
                break;
            }
            match bytes[i + 1] {
                b'"' => value.push('"'),
                b'\\' => value.push('\\'),
                b'n' => value.push('\n'),
                b't' => value.push('\t'),
                other => {
                    return Err(Error::at(
                        Span::new(i, i + 2),
                        format!("invalid escape sequence '\\{}'", other as char),
                    ));
                }
            }
            i += 2;
            continue;
        }
        let ch = src[i..].chars().next().expect("non-empty");
        value.push(ch);
        i += ch.len_utf8();
    }

    Err(Error::at(
        Span::new(start, i),
        "unterminated string literal",
    ))
}

fn is_ident_start(c: u8) -> bool {
    c.is_ascii_alphabetic() || c == b'_'
}

fn is_ident_continue(c: u8) -> bool {
    c.is_ascii_alphanumeric() || c == b'_' || c == b'-'
}
