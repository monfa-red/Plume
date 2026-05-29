use plume::RenderOptions;

fn render_live(src: &str) -> String {
    plume::compile_str(src).expect("compile")
}

fn render_baked(src: &str) -> String {
    plume::compile_str_with(
        src,
        &RenderOptions {
            bake_vars: true,
            ..Default::default()
        },
    )
    .expect("compile")
}

#[test]
fn live_mode_emits_var_refs_for_visual_attrs() {
    let svg = render_live("|rect| \"hi\"\n");
    assert!(svg.contains("var(--plume-fill)"), "{}", svg);
    assert!(svg.contains("var(--plume-stroke)"), "{}", svg);
    assert!(
        svg.contains("@layer plume.defaults"),
        "default style block should be present in live mode"
    );
}

#[test]
fn bake_mode_resolves_var_refs_to_literals() {
    let svg = render_baked("|rect| \"hi\"\n");
    assert!(svg.contains("fill=\"white\""), "{}", svg);
    assert!(svg.contains("stroke=\"#444\""), "{}", svg);
    // Text fill is `currentColor` (SVG-native cascade), and the scene root
    // sets `color` to the baked `--text-color` (= --fg = #222).
    assert!(svg.contains("fill=\"currentColor\""), "{}", svg);
    assert!(svg.contains("color=\"#222\""), "{}", svg);
    assert!(
        !svg.contains("@layer plume.defaults"),
        "bake mode should omit the defaults style block"
    );
}

#[test]
fn defaults_override_baked_into_output() {
    let svg = render_baked("{ --accent:#ff00aa }\ncat |rect| \"Cat\" fill:--accent\n");
    assert!(svg.contains("fill=\"#ff00aa\""), "{}", svg);
}

#[test]
fn auto_classes_include_primitive_and_styles() {
    let svg = render_live(
        "{ .bold weight:bold\n  .thin stroke:#444 }\n\
         cat |rect| \"Cat\" .bold .thin\n",
    );
    assert!(svg.contains("plume-shape-rect"), "{}", svg);
    assert!(svg.contains("plume-style-bold"), "{}", svg);
    assert!(svg.contains("plume-style-thin"), "{}", svg);
}

#[test]
fn auto_classes_include_user_shape_chain() {
    let svg = render_live(
        "{ |treat:rect| radius:5 }\n\
         cat |treat| \"Cat\"\n",
    );
    assert!(svg.contains("plume-shape-treat"), "{}", svg);
    assert!(svg.contains("plume-shape-rect"), "{}", svg);
    assert!(svg.contains(r#"data-id="cat""#), "{}", svg);
}

#[test]
fn hex_emits_polygon() {
    let svg = render_live("|hex| size:(60, 60)\n");
    assert!(svg.contains("<polygon"), "{}", svg);
}

#[test]
fn diamond_emits_polygon() {
    let svg = render_live("|diamond| size:(60, 60)\n");
    assert!(svg.contains("<polygon"), "{}", svg);
}

#[test]
fn slant_emits_polygon_with_skew() {
    let svg = render_live("|slant| size:(80, 40) skew:20\n");
    assert!(svg.contains("<polygon"), "{}", svg);
}

#[test]
fn oval_emits_ellipse() {
    let svg = render_live("|oval| size:(80, 40)\n");
    assert!(svg.contains("<ellipse"), "{}", svg);
}

#[test]
fn cyl_emits_ellipse_and_path() {
    let svg = render_live("|cyl| size:(60, 80)\n");
    assert!(svg.contains("<ellipse"), "{}", svg);
    assert!(svg.contains("<path"), "{}", svg);
}

#[test]
fn cloud_emits_path() {
    let svg = render_live("|cloud| size:(100, 60)\n");
    assert!(svg.contains("<path"), "{}", svg);
}

#[test]
fn poly_emits_polygon_with_user_points() {
    let svg = render_live("|poly| points:[(0,0),(20,0),(10,20)]\n");
    assert!(svg.contains("<polygon"), "{}", svg);
}

#[test]
fn full_spec_example_renders_in_both_modes() {
    let src = std::fs::read_to_string("samples/full_example.plume").expect("read");
    let live = plume::compile_str(&src).expect("live compile");
    let baked = plume::compile_str_with(
        &src,
        &RenderOptions {
            bake_vars: true,
            ..Default::default()
        },
    )
    .expect("baked compile");
    assert!(live.contains("var(--plume-"));
    assert!(!baked.contains("@layer plume.defaults"));
    assert!(live.starts_with("<svg"));
    assert!(baked.starts_with("<svg"));
    assert!(live.contains(r#"<g class="plume-wires"/>"#));
    assert!(baked.contains(r#"<g class="plume-wires"/>"#));
}
