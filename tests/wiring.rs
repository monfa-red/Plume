//! Phase 0 acceptance (PLAN.md): the dumb router satisfies WIRING.md's per-wire
//! invariants on every sample, and the whole compile is deterministic. Cross-wire
//! crossings (A3) are reported but expected until the multi-wire phases, so they
//! are not gated here.

use std::path::PathBuf;

fn sample_paths() -> Vec<PathBuf> {
    let mut paths: Vec<PathBuf> = std::fs::read_dir("samples")
        .expect("read samples/")
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x == "plume"))
        .collect();
    paths.sort();
    paths
}

#[test]
fn dumb_router_holds_per_wire_invariants() {
    for path in sample_paths() {
        let src = std::fs::read_to_string(&path).unwrap();
        let violations = plume::validate_str(&src).expect("validate");
        // A1/A2/A5 are guaranteed by construction; A3 (PerpCrossing) is a
        // multi-wire property and not yet gated.
        let blocking: Vec<&plume::Violation> = violations
            .iter()
            .filter(|v| !matches!(v.rule, plume::Rule::PerpCrossing))
            .collect();
        assert!(
            blocking.is_empty(),
            "{}: per-wire invariant violations: {:?}",
            path.display(),
            blocking
        );
    }
}

#[test]
fn compile_is_byte_identical_across_runs() {
    let opts = plume::Options {
        bake_vars: true,
        ..Default::default()
    };
    for path in sample_paths() {
        let src = std::fs::read_to_string(&path).unwrap();
        let a = plume::compile_str_with(&src, &opts).expect("compile a");
        let b = plume::compile_str_with(&src, &opts).expect("compile b");
        assert_eq!(a, b, "{}: non-deterministic output", path.display());
    }
}
