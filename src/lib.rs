mod ast;
mod error;
mod layout;
mod lexer;
mod parser;
mod render;
mod resolve;
mod span;
mod theme;

pub use error::Error;

use std::path::Path;

pub fn compile_str(src: &str) -> Result<String, Error> {
    let tokens = lexer::lex(src)?;
    let file = parser::parse(&tokens)?;
    let program = resolve::resolve(file)?;
    let laid_out = layout::layout(&program)?;
    Ok(render::render(&laid_out))
}

pub fn compile_file(path: &Path) -> Result<String, Error> {
    let src = std::fs::read_to_string(path).map_err(|e| {
        Error::at(
            span::Span::empty(),
            format!("read {}: {}", path.display(), e),
        )
    })?;
    compile_str(&src)
}

/// Lex and parse only — verifies syntactic correctness without running
/// resolve/layout/render. Used by tests and (eventually) `plume --check-parse`.
pub fn check_parse(src: &str) -> Result<(), Error> {
    let tokens = lexer::lex(src)?;
    let _file = parser::parse(&tokens)?;
    Ok(())
}
