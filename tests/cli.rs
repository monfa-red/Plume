//! End-to-end tests for the public Options surface — exercised through the
//! library API (which is what the CLI calls) rather than spawning the binary.

use plume::{Options, OutputFormat};

#[test]
fn html_format_wraps_svg_in_html_doc() {
    let html = plume::compile_str_with(
        "|rect| \"x\"\n",
        &Options {
            format: OutputFormat::Html,
            bake_vars: true,
            ..Default::default()
        },
    )
    .expect("compile");
    assert!(html.starts_with("<!doctype html>"));
    assert!(html.contains("<svg "));
    assert!(html.contains("</body>"));
    assert!(html.ends_with("</html>\n"));
}

#[test]
fn no_defaults_omits_style_block_but_keeps_var_refs() {
    let svg = plume::compile_str_with(
        "|rect| \"x\"\n",
        &Options {
            no_defaults: true,
            ..Default::default()
        },
    )
    .expect("compile");
    assert!(!svg.contains("@layer plume.defaults"));
    assert!(svg.contains("var(--plume-fill)"));
}

#[test]
fn theme_overrides_visual_var_visible_in_baked_output() {
    let svg = plume::compile_str_with(
        "|rect| \"x\" fill:--accent\n",
        &Options {
            theme_css: Some("--plume-accent: hotpink;".to_string()),
            bake_vars: true,
            ..Default::default()
        },
    )
    .expect("compile");
    assert!(svg.contains(r#"fill="hotpink""#), "{}", svg);
}

#[test]
fn theme_layout_var_bakes_into_layout_math() {
    let src = "{ |scene| layout:row }\n|rect| size:(40, 40)\n|rect| size:(40, 40)\n";
    let default = plume::compile_str(src).expect("default compile");
    let themed = plume::compile_str_with(
        src,
        &Options {
            theme_css: Some("--plume-gap: 60;".to_string()),
            ..Default::default()
        },
    )
    .expect("themed compile");
    let default_w = extract_viewbox_w(&default);
    let themed_w = extract_viewbox_w(&themed);
    assert!(
        (themed_w - default_w - 40.0).abs() < 0.5,
        "expected +40px viewbox width with gap=60 theme; default={} themed={}",
        default_w,
        themed_w,
    );
}

#[test]
fn theme_visual_var_does_not_change_layout_baking() {
    let src = "{ |scene| layout:row }\n|rect| size:(40, 40)\n|rect| size:(40, 40)\n";
    let default = plume::compile_str(src).expect("default compile");
    let themed = plume::compile_str_with(
        src,
        &Options {
            theme_css: Some("--plume-accent: red;".to_string()),
            ..Default::default()
        },
    )
    .expect("themed compile");
    assert_eq!(extract_viewbox_w(&default), extract_viewbox_w(&themed));
}

#[test]
fn check_with_succeeds_on_valid_input() {
    let opts = Options::default();
    assert!(plume::check_with("|rect| \"x\"\n", &opts).is_ok());
}

#[test]
fn check_with_propagates_resolve_errors() {
    let opts = Options::default();
    let err = plume::check_with("|nosuch| \"x\"\n", &opts).expect_err("expected error");
    assert!(
        err.to_string().contains("unknown type '|nosuch|'"),
        "got: {}",
        err
    );
}

fn extract_viewbox_w(svg: &str) -> f64 {
    let vb = svg
        .lines()
        .next()
        .unwrap()
        .split("viewBox=\"")
        .nth(1)
        .unwrap()
        .split('"')
        .next()
        .unwrap();
    vb.split_whitespace().nth(2).unwrap().parse().unwrap()
}
