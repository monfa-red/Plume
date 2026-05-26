use clap::Parser;
use std::io::Read;
use std::path::PathBuf;
use std::process::ExitCode;

#[derive(Parser)]
#[command(name = "plume", version, about, long_about = None)]
struct Cli {
    /// Input .plume file (use '-' for stdin)
    input: String,
    /// Output path (default: stdout)
    #[arg(short = 'o', long = "output")]
    output: Option<PathBuf>,
    /// Emit `var()` values inline as their resolved literal — necessary for
    /// renderers without CSS-variable support (resvg, librsvg, raster
    /// converters). Omits the default-vars `<style>` block.
    #[arg(long = "bake-vars")]
    bake_vars: bool,
}

fn main() -> ExitCode {
    let cli = Cli::parse();

    let (filename, source) = match cli.input.as_str() {
        "-" => {
            let mut buf = String::new();
            if std::io::stdin().read_to_string(&mut buf).is_err() {
                eprintln!("error: failed to read stdin");
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

    let opts = plume::RenderOptions {
        bake_vars: cli.bake_vars,
    };

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
