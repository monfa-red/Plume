use std::ffi::OsStr;
use std::path::PathBuf;

/// Every sample must lex, parse, and resolve cleanly.
#[test]
fn all_samples_resolve() {
    let samples_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("samples");
    let mut failures = Vec::new();

    for entry in std::fs::read_dir(&samples_dir).expect("read samples dir") {
        let path = entry.expect("readdir entry").path();
        if path.extension() != Some(OsStr::new("plume")) {
            continue;
        }
        let src = std::fs::read_to_string(&path).expect("read sample");
        if let Err(e) = plume::check(&src) {
            let name = path.file_name().unwrap().to_string_lossy().into_owned();
            failures.push(format!("{}: {}", name, e));
        }
    }

    assert!(
        failures.is_empty(),
        "the following samples failed to resolve:\n  {}",
        failures.join("\n  ")
    );
}

// ─────────────────────────── Invalid-input cases ───────────────────────────

#[track_caller]
fn assert_resolve_error(src: &str, expect_msg_substr: &str) {
    let err = plume::check(src).expect_err("expected resolve error");
    let msg = err.to_string();
    assert!(
        msg.contains(expect_msg_substr),
        "expected error containing {:?}, got {:?}",
        expect_msg_substr,
        msg
    );
}

#[test]
fn err_duplicate_scene_id() {
    assert_resolve_error(
        "cat |rect| \"1\"\ncat |rect| \"2\"\n",
        "duplicate scene id 'cat'",
    );
}

#[test]
fn err_unknown_shape_type() {
    assert_resolve_error("cat |nosuch| \"x\"\n", "unknown type '|nosuch|'");
}

#[test]
fn err_unknown_style() {
    assert_resolve_error("cat |rect| \"x\" .nope\n", "unknown style '.nope'");
}

#[test]
fn err_style_cycle() {
    assert_resolve_error(
        "{ .alpha .beta\n  .beta .alpha }\ncat |rect|\n",
        "cycle in style",
    );
}

#[test]
fn err_shape_cycle() {
    assert_resolve_error("{ |looper:looper| }\ncat |rect|\n", "cycle in");
}

#[test]
fn err_shape_name_collides_with_primitive() {
    assert_resolve_error("{ |rect:oval| }\ncat |rect|\n", "'rect' is reserved");
}

#[test]
fn err_shape_name_collides_with_template() {
    assert_resolve_error("{ |card:rect| }\ncat |rect|\n", "'card' is reserved");
}

#[test]
fn err_reserved_scene_id() {
    assert_resolve_error("rect |rect| \"x\"\n", "'rect' is reserved");
}

#[test]
fn err_reserved_style_name() {
    assert_resolve_error("{ .card weight:bold }\ncat |rect|\n", "'card' is reserved");
}

#[test]
fn wire_endpoint_dotpath_navigates_into_groups() {
    plume::check("garden |group| { frog |rect| }\noutside |rect|\ngarden.frog -> outside\n")
        .expect("dot-path resolves");
}

#[test]
fn type_defaults_apply_to_every_instance() {
    // `|rect| radius:5` in defs gives every rect a default radius of 5.
    plume::check("{ |rect| radius:5 }\ncat |rect| \"Cat\"\n").expect("rect defaults");
}

#[test]
fn type_defaults_unknown_type_errors() {
    let err = plume::check("{ |frog| fill:green }\ncat |rect|\n").expect_err("unknown");
    assert!(err.to_string().contains("unknown type"), "got: {}", err);
}

#[test]
fn type_defaults_duplicate_errors() {
    let err = plume::check("{ |rect| radius:5\n  |rect| radius:9 }\ncat |rect|\n")
        .expect_err("duplicate");
    assert!(err.to_string().contains("duplicate"), "got: {}", err);
}

#[test]
fn wire_endpoint_ambiguous_path_errors() {
    let err = plume::check(
        "kitchen |group| { mouse |rect| }\ngarden |group| { mouse |rect| }\nmouse -> kitchen\n",
    )
    .expect_err("ambiguous");
    assert!(err.to_string().contains("ambiguous"), "got: {}", err);
}
