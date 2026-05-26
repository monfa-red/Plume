use clap::{error::ErrorKind, Parser};
use std::io::Read;
use std::path::PathBuf;
use std::process::ExitCode;

#[derive(Parser)]
#[command(
    name = "plume",
    version,
    about = "Compile Plume diagrams to SVG",
    long_about = None,
    disable_help_flag = false,
)]
struct Cli {
    /// Input .plume file (use '-' for stdin)
    input: String,

    /// Output path (default: stdout)
    #[arg(short = 'o', long = "output")]
    output: Option<PathBuf>,

    /// Output wrapper: `svg` (default) or `html`.
    #[arg(long = "format", default_value = "svg")]
    format: String,

    /// Force-embed the default `<style>` block (default outside preprocessor mode).
    #[arg(long = "standalone")]
    standalone: bool,

    /// Omit the default `<style>` block — host page supplies `--plume-*` vars.
    #[arg(long = "no-defaults", conflicts_with = "bake_vars")]
    no_defaults: bool,

    /// Emit `var()` values inline as their resolved literal. Necessary for
    /// renderers without CSS-variable support (resvg, librsvg, raster
    /// converters).
    #[arg(long = "bake-vars")]
    bake_vars: bool,

    /// Parse and validate only — no layout, no render.
    #[arg(long = "check")]
    check: bool,

    /// CSS file with `--plume-*` overrides. Applied before the `defaults {}`
    /// block; layout vars from the theme bake into the layout.
    #[arg(long = "theme", value_name = "FILE")]
    theme: Option<PathBuf>,
}

fn main() -> ExitCode {
    let cli = match Cli::try_parse() {
        Ok(c) => c,
        Err(e) => {
            // clap prints help/version to stdout and errors to stderr itself.
            let _ = e.print();
            return match e.kind() {
                ErrorKind::DisplayHelp | ErrorKind::DisplayVersion => ExitCode::SUCCESS,
                _ => ExitCode::from(3),
            };
        }
    };

    let format = match cli.format.as_str() {
        "svg" => plume::OutputFormat::Svg,
        "html" => plume::OutputFormat::Html,
        other => {
            eprintln!("error: invalid --format '{}' (expected svg|html)", other);
            return ExitCode::from(3);
        }
    };
    // `--standalone` is explicitly the default and is therefore a no-op flag.
    // Accept it for spec compliance.
    let _ = cli.standalone;

    let (filename, source) = match cli.input.as_str() {
        "-" => {
            let mut buf = String::new();
            if let Err(e) = std::io::stdin().read_to_string(&mut buf) {
                eprintln!("error: failed to read stdin: {}", e);
                return ExitCode::from(2);
            }
            ("<stdin>".to_string(), buf)
        }
        path => match std::fs::read_to_string(path) {
            Ok(s) => (path.to_string(), s),
            Err(e) => {
                eprintln!("error: {}: {}", path, e);
                return ExitCode::from(2);
            }
        },
    };

    let theme_css = match &cli.theme {
        Some(path) => match std::fs::read_to_string(path) {
            Ok(s) => Some(s),
            Err(e) => {
                eprintln!("error: {}: {}", path.display(), e);
                return ExitCode::from(2);
            }
        },
        None => None,
    };

    let opts = plume::Options {
        bake_vars: cli.bake_vars,
        no_defaults: cli.no_defaults,
        format,
        theme_css,
    };

    if cli.check {
        return match plume::check_with(&source, &opts) {
            Ok(_) => ExitCode::SUCCESS,
            Err(e) => {
                eprintln!("{}", e.display_with_source(&source, &filename));
                ExitCode::from(1)
            }
        };
    }

    match plume::compile_str_with(&source, &opts) {
        Ok(svg) => {
            if let Some(out_path) = cli.output {
                if let Err(e) = std::fs::write(&out_path, svg.as_bytes()) {
                    eprintln!("error: write {}: {}", out_path.display(), e);
                    return ExitCode::from(2);
                }
            } else {
                print!("{}", svg);
            }
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("{}", e.display_with_source(&source, &filename));
            ExitCode::from(1)
        }
    }
}
