use plume::RenderOptions;

fn render_live(src: &str) -> String {
    plume::compile_str(src).expect("compile")
}

fn render_baked(src: &str) -> String {
    plume::compile_str_with(src, &RenderOptions { bake_vars: true }).expect("compile")
}

#[test]
fn live_mode_emits_var_refs_for_visual_attrs() {
    let svg = render_live("scene { :rect \"hi\" }\n");
    assert!(svg.contains("var(--plume-fill)"), "{}", svg);
    assert!(svg.contains("var(--plume-stroke)"), "{}", svg);
    assert!(
        svg.contains("@layer plume.defaults"),
        "default style block should be present in live mode"
    );
}

#[test]
fn bake_mode_resolves_var_refs_to_literals() {
    let svg = render_baked("scene { :rect \"hi\" }\n");
    assert!(svg.contains("fill=\"white\""), "{}", svg);
    assert!(svg.contains("stroke=\"#444\""), "{}", svg);
    // Recursive resolution: text-color → fg → #222
    assert!(svg.contains("fill=\"#222\""), "{}", svg);
    // No defaults block needed in bake mode.
    assert!(
        !svg.contains("@layer plume.defaults"),
        "bake mode should omit the defaults style block"
    );
}

#[test]
fn defaults_override_baked_into_output() {
    // Override accent in defaults — bake mode should pick up the new value.
    let svg = render_baked(
        "defaults { accent=#ff00aa }\n\
         scene { :rect \"x\" fill=var(accent) }\n",
    );
    assert!(svg.contains("fill=\"#ff00aa\""), "{}", svg);
}

#[test]
fn auto_classes_include_primitive_and_styles() {
    let svg = render_live(
        "styles { bold weight=bold\n thin stroke=#444 }\n\
         scene { :rect \"x\" .bold .thin }\n",
    );
    assert!(svg.contains("plume-shape-rect"), "{}", svg);
    assert!(svg.contains("plume-style-bold"), "{}", svg);
    assert!(svg.contains("plume-style-thin"), "{}", svg);
}

#[test]
fn auto_classes_include_user_shape_chain() {
    let svg = render_live(
        "shapes { psu :rect radius=5 }\n\
         scene { drive :psu \"PSU\" }\n",
    );
    // SPEC §13: user shape → primitive chain, both emitted.
    assert!(svg.contains("plume-shape-psu"), "{}", svg);
    assert!(svg.contains("plume-shape-rect"), "{}", svg);
    assert!(svg.contains(r#"data-id="drive""#), "{}", svg);
}

#[test]
fn hex_emits_polygon() {
    let svg = render_live("scene { :hex w=60 h=60 }\n");
    assert!(svg.contains("<polygon"), "{}", svg);
}

#[test]
fn diamond_emits_polygon() {
    let svg = render_live("scene { :diamond w=60 h=60 }\n");
    assert!(svg.contains("<polygon"), "{}", svg);
}

#[test]
fn slant_emits_polygon_with_skew() {
    let svg = render_live("scene { :slant w=80 h=40 skew=20 }\n");
    assert!(svg.contains("<polygon"), "{}", svg);
}

#[test]
fn oval_emits_ellipse() {
    let svg = render_live("scene { :oval rx=40 ry=20 }\n");
    assert!(svg.contains("<ellipse"), "{}", svg);
}

#[test]
fn cyl_emits_ellipse_and_path() {
    let svg = render_live("scene { :cyl w=60 h=80 }\n");
    assert!(svg.contains("<ellipse"), "{}", svg);
    assert!(svg.contains("<path"), "{}", svg);
}

#[test]
fn cloud_emits_path() {
    let svg = render_live("scene { :cloud w=100 h=60 }\n");
    assert!(svg.contains("<path"), "{}", svg);
}

#[test]
fn poly_emits_polygon_with_user_points() {
    let svg = render_live("scene { :poly points=[(0,0),(20,0),(10,20)] }\n");
    assert!(svg.contains("<polygon"), "{}", svg);
}

#[test]
fn full_spec_example_renders_in_both_modes() {
    let src = std::fs::read_to_string("samples/full_example.plume").expect("read");
    let live = plume::compile_str(&src).expect("live compile");
    let baked =
        plume::compile_str_with(&src, &RenderOptions { bake_vars: true }).expect("baked compile");
    assert!(live.contains("var(--plume-"));
    assert!(!baked.contains("@layer plume.defaults"));
    // Both should be plausible SVG documents.
    assert!(live.starts_with("<svg"));
    assert!(baked.starts_with("<svg"));
    assert!(live.contains("plume-wire"));
    assert!(baked.contains("plume-wire"));
}
