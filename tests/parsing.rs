use std::ffi::OsStr;
use std::path::PathBuf;

/// Every `samples/*.plume` file must lex + parse without error.
/// Resolve / layout / render correctness is enforced by sprint-specific tests.
#[test]
fn all_samples_parse() {
    let samples_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("samples");
    let mut failures = Vec::new();

    for entry in std::fs::read_dir(&samples_dir).expect("read samples dir") {
        let path = entry.expect("readdir entry").path();
        if path.extension() != Some(OsStr::new("plume")) {
            continue;
        }
        let src = std::fs::read_to_string(&path).expect("read sample");
        if let Err(e) = plume::check_parse(&src) {
            let name = path.file_name().unwrap().to_string_lossy().into_owned();
            failures.push(format!("{}: {}", name, e));
        }
    }

    assert!(
        failures.is_empty(),
        "the following samples failed to parse:\n  {}",
        failures.join("\n  ")
    );
}

// ─────────────────────────── Invalid-input cases ───────────────────────────

#[track_caller]
fn assert_parse_error(src: &str, expect_msg_substr: &str) {
    let err = plume::check_parse(src).expect_err("expected parse error");
    let msg = err.to_string();
    assert!(
        msg.contains(expect_msg_substr),
        "expected error containing {:?}, got {:?}",
        expect_msg_substr,
        msg
    );
}

#[test]
fn err_unknown_top_level_block() {
    assert_parse_error("foo { }\n", "unknown top-level block 'foo'");
}

#[test]
fn err_wire_chain_mixes_operators() {
    assert_parse_error("wires { a -> b --> c }\n", "wire chain mixes operators");
}

#[test]
fn err_wire_endpoint_brackets_rejected() {
    // SPEC v2.1: bracket anchors on wire endpoints were dropped.
    assert_parse_error("wires { a[right] -> b }\n", "expected wire operator");
}

#[test]
fn err_unterminated_string() {
    assert_parse_error("scene { :rect \"oops }\n", "unterminated string");
}

#[test]
fn err_bad_escape_sequence() {
    assert_parse_error("scene { :rect \"\\x\" }\n", "invalid escape sequence");
}

#[test]
fn err_invalid_hex_color() {
    assert_parse_error(
        "defaults { c=#ff }\nscene { :rect \"x\" }\n",
        "invalid hex color",
    );
}

#[test]
fn err_wire_body_non_text() {
    assert_parse_error(
        "wires { a -> b { :rect \"oops\" } }\n",
        "wire body may only contain :text primitives",
    );
}

#[test]
fn err_shape_def_needs_base_or_body() {
    assert_parse_error("shapes { naked }\n", "requires :base or a body");
}

#[test]
fn plume_var_value_parses_anywhere() {
    // SPEC v1 §11.2: `--name` is a first-class value form, not function-scoped.
    plume::check_parse("defaults { gap=--my-gap }\n").expect("--gap parses");
    plume::check_parse("scene { :rect fill=--accent }\n").expect("--accent parses");
}

// ───────────────── Multi-line attr continuation (SPEC §2) ─────────────────

#[track_caller]
fn parses_same(single_line: &str, multi_line: &str) {
    plume::check_parse(single_line).expect("single-line parse");
    plume::check_parse(multi_line).expect("multi-line parse");
}

#[test]
fn continuation_key_value_after_newline() {
    parses_same(
        "scene { my :rect cell=(1, 1) size=(80, 40) }\n",
        "scene {\n  my :rect cell=(1, 1)\n     size=(80, 40)\n}\n",
    );
}

#[test]
fn continuation_dot_style_after_newline() {
    parses_same(
        "styles { thin stroke=#444 }\nscene { my :rect .thin }\n",
        "styles { thin stroke=#444 }\nscene {\n  my :rect\n     .thin\n}\n",
    );
}

#[test]
fn continuation_open_brace_on_next_line() {
    parses_same(
        "scene { my :rect cell=(2, 1) { } }\n",
        "scene {\n  my :rect cell=(2, 1)\n  {\n  }\n}\n",
    );
}

#[test]
fn newline_then_unrelated_ident_is_not_continuation() {
    // Ensure two siblings without continuation keywords still parse as separate statements.
    plume::check_parse("scene {\n  a :rect\n  b :rect\n}\n").expect("two separate statements");
}
