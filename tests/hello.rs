#[test]
fn hello_sample_compiles_to_expected_svg() {
    let src = std::fs::read_to_string("samples/hello.plume").expect("read samples/hello.plume");
    let svg = plume::compile_str(&src).expect("compile hello.plume");
    insta::assert_snapshot!(svg);
}
