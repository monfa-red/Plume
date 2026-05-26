use std::ffi::OsStr;
use std::path::PathBuf;

/// Every sample must lex, parse, and resolve cleanly. Layout/render coverage is
/// sprint-specific; this gate is just on Sprint 2's resolver.
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
fn err_block_order_shapes_after_scene() {
    assert_resolve_error(
        "scene { :rect \"x\" }\nshapes { my :rect }\n",
        "'shapes' must appear before",
    );
}

#[test]
fn err_duplicate_block() {
    assert_resolve_error(
        "scene { :rect \"a\" }\nscene { :rect \"b\" }\n",
        "duplicate 'scene' block",
    );
}

#[test]
fn err_missing_scene_block() {
    assert_resolve_error(
        "styles { bold weight=bold }\n",
        "missing required 'scene' block",
    );
}

#[test]
fn err_duplicate_scene_id() {
    assert_resolve_error(
        "scene { a :rect \"1\"\n a :rect \"2\" }\n",
        "duplicate scene id 'a'",
    );
}

#[test]
fn err_unknown_shape_type() {
    assert_resolve_error("scene { :nosuch \"x\" }\n", "unknown type ':nosuch'");
}

#[test]
fn err_unknown_style() {
    assert_resolve_error("scene { :rect \"x\" .nope }\n", "unknown style '.nope'");
}

#[test]
fn err_style_cycle() {
    assert_resolve_error(
        "styles { a .b\n b .a }\nscene { :rect \"x\" }\n",
        "cycle in style",
    );
}

#[test]
fn err_shape_cycle() {
    assert_resolve_error("shapes { loop :loop }\nscene { :rect \"x\" }\n", "cycle in");
}

#[test]
fn err_shape_name_collides_with_primitive() {
    assert_resolve_error(
        "shapes { rect :oval }\nscene { :rect \"x\" }\n",
        "'rect' is reserved",
    );
}

#[test]
fn err_shape_name_collides_with_template() {
    assert_resolve_error(
        "shapes { card :rect }\nscene { :rect \"x\" }\n",
        "'card' is reserved",
    );
}

#[test]
fn err_wire_unknown_endpoint() {
    assert_resolve_error(
        "scene { a :rect \"A\" }\nwires { a -> ghost }\n",
        "wire references undefined id 'ghost'",
    );
}

#[test]
fn err_reserved_scene_id() {
    assert_resolve_error("scene { rect :rect \"x\" }\n", "'rect' is reserved");
}

#[test]
fn err_reserved_style_name() {
    assert_resolve_error(
        "styles { card weight=bold }\nscene { :rect \"x\" }\n",
        "'card' is reserved",
    );
}
