//! Conformance suite — every `samples/*.plume` file is compiled with
//! `--bake-vars` and its SVG output snapshotted via `insta`. Changes that
//! shift any sample's output surface as a snapshot diff, surfacing
//! regressions across all SPEC features at once.
//!
//! Bake mode is the default snapshot because it produces hermetic output:
//! no `var(...)` indirection, every literal frozen. Live-mode snapshots
//! are covered by the dedicated tests in `tests/rendering.rs`.

use plume::{Options, OutputFormat};

#[test]
fn snapshot_baked_svg_for_every_sample() {
    let opts = Options {
        bake_vars: true,
        format: OutputFormat::Svg,
        ..Default::default()
    };

    let samples_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("samples");
    insta::glob!(samples_dir, "*.plume", |path| {
        let src = std::fs::read_to_string(path)
            .unwrap_or_else(|e| panic!("read {}: {}", path.display(), e));
        let svg = plume::compile_str_with(&src, &opts)
            .unwrap_or_else(|e| panic!("{}: compile failed: {}", path.display(), e));
        insta::assert_snapshot!(svg);
    });
}
