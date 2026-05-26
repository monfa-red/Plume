use crate::span::{line_col, Span};
use std::fmt;

#[derive(Debug)]
pub struct Error {
    pub message: String,
    pub span: Span,
}

impl Error {
    pub fn at(span: Span, message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            span,
        }
    }

    pub fn display_with_source<'a>(
        &'a self,
        source: &'a str,
        filename: &'a str,
    ) -> ErrorDisplay<'a> {
        ErrorDisplay {
            err: self,
            source,
            filename,
        }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "error: {}", self.message)
    }
}

impl std::error::Error for Error {}

pub struct ErrorDisplay<'a> {
    err: &'a Error,
    source: &'a str,
    filename: &'a str,
}

impl<'a> fmt::Display for ErrorDisplay<'a> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let (line, col) = line_col(self.source, self.err.span.start);
        write!(
            f,
            "{}:{}:{}: error: {}",
            self.filename, line, col, self.err.message
        )
    }
}

#[derive(Debug, Clone)]
pub struct Diagnostic {
    pub level: Level,
    pub message: String,
    pub span: Span,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Level {
    Warning,
}

impl Diagnostic {
    pub fn warn(span: Span, message: impl Into<String>) -> Self {
        Self {
            level: Level::Warning,
            message: message.into(),
            span,
        }
    }

    pub fn display_with_source<'a>(
        &'a self,
        source: &'a str,
        filename: &'a str,
    ) -> DiagnosticDisplay<'a> {
        DiagnosticDisplay {
            diag: self,
            source,
            filename,
        }
    }
}

pub struct DiagnosticDisplay<'a> {
    diag: &'a Diagnostic,
    source: &'a str,
    filename: &'a str,
}

impl<'a> fmt::Display for DiagnosticDisplay<'a> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let (line, col) = line_col(self.source, self.diag.span.start);
        let kind = match self.diag.level {
            Level::Warning => "warning",
        };
        write!(
            f,
            "{}:{}:{}: {}: {}",
            self.filename, line, col, kind, self.diag.message
        )
    }
}
