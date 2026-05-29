mod ast;
mod error;
mod fmt;
mod layout;
mod lexer;
mod lint;
mod parser;
mod render;
mod resolve;
mod serve;
mod span;
mod theme;

pub use error::{Diagnostic, Error, Level};
pub use fmt::format as format_source;
pub use layout::{Rule, Severity, Violation};
pub use serve::serve;
pub use theme::extract_plume_vars;

use std::path::Path;

/// Top-level compile options threaded through every phase. Build with
/// `Options::default()` and override fields with the struct-update syntax —
/// future versions may add knobs.
#[derive(Clone, Debug, Default)]
pub struct Options {
    /// Emit `var()` values inline as their resolved literal so renderers
    /// without CSS-variable support (resvg, librsvg, image converters) still
    /// display the diagram correctly. The defaults `<style>` block is skipped
    /// in this mode.
    pub bake_vars: bool,
    /// Omit the `<style>@layer plume.defaults { ... }</style>` block. The host
    /// page is expected to supply `--plume-*` custom properties.
    pub no_defaults: bool,
    /// Output wrapper format.
    pub format: OutputFormat,
    /// Raw CSS text whose `--plume-*` declarations override built-in defaults
    /// before the `defaults {}` block. `extract_plume_vars` does the parse.
    pub theme_css: Option<String>,
}

#[derive(Default, Clone, Copy, Debug, PartialEq, Eq)]
pub enum OutputFormat {
    #[default]
    Svg,
    Html,
}

/// Sprint 5 named the render-time options struct `RenderOptions`; keep that
/// spelling as an alias so existing call sites keep compiling.
pub type RenderOptions = Options;

pub fn compile_str(src: &str) -> Result<String, Error> {
    compile_str_with(src, &Options::default())
}

pub fn compile_str_with(src: &str, opts: &Options) -> Result<String, Error> {
    let program = resolve_pipeline(src, opts)?;
    let laid_out = layout::layout(&program)?;
    let svg = render::render(&laid_out, opts);
    Ok(match opts.format {
        OutputFormat::Svg => svg,
        OutputFormat::Html => wrap_html(&svg),
    })
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
/// resolve/layout/render.
pub fn check_parse(src: &str) -> Result<(), Error> {
    let tokens = lexer::lex(src)?;
    let _file = parser::parse(&tokens)?;
    Ok(())
}

/// Lex, parse, and run the lint pass. Returns warnings (no errors).
/// Parse errors are surfaced as `Err`; missing lints just return an empty Vec.
pub fn lint_str(src: &str) -> Result<Vec<Diagnostic>, Error> {
    let tokens = lexer::lex(src)?;
    let file = parser::parse(&tokens)?;
    Ok(lint::lint(&file))
}

/// Lex, parse, and resolve. Verifies semantic correctness without running
/// layout or render. The CLI's `--check` flag goes through here.
pub fn check(src: &str) -> Result<(), Error> {
    check_with(src, &Options::default())
}

pub fn check_with(src: &str, opts: &Options) -> Result<(), Error> {
    let _ = resolve_pipeline(src, opts)?;
    Ok(())
}

/// Lex, parse, resolve, lay out, route, then validate the routing against the
/// contract in WIRING.md. Returns the violations found (empty = clean). Parse
/// and resolve errors surface as `Err`.
pub fn validate_str(src: &str) -> Result<Vec<Violation>, Error> {
    let program = resolve_pipeline(src, &Options::default())?;
    let laid_out = layout::layout(&program)?;
    Ok(layout::validate_routing(&laid_out))
}

fn resolve_pipeline(src: &str, opts: &Options) -> Result<resolve::Program, Error> {
    let tokens = lexer::lex(src)?;
    let file = parser::parse(&tokens)?;
    let theme = match &opts.theme_css {
        Some(css) => theme::extract_plume_vars(css),
        None => Vec::new(),
    };
    resolve::resolve_with_theme(file, &theme)
}

fn wrap_html(svg: &str) -> String {
    format!(
        "<!doctype html>\n<html>\n<head>\n  <meta charset=\"utf-8\">\n  <title>plume</title>\n</head>\n<body>\n{}</body>\n</html>\n",
        svg
    )
}
