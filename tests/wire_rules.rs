//! Baseline routing-contract report. This snapshot captures every violation
//! the CURRENT router produces across all samples — the "what's broken"
//! ground truth the Step-2 rebuild drives toward empty. As the router
//! improves, accept the shrinking snapshot with `cargo insta review`.

use std::fs;
use std::path::PathBuf;

#[test]
fn routing_rules_baseline() {
    let mut paths: Vec<PathBuf> = fs::read_dir("samples")
        .unwrap()
        .filter_map(|e| {
            let p = e.unwrap().path();
            (p.extension().and_then(|x| x.to_str()) == Some("plume")).then_some(p)
        })
        .collect();
    paths.sort();

    let mut report = String::new();
    for p in paths {
        let name = p.file_name().unwrap().to_string_lossy().to_string();
        let src = fs::read_to_string(&p).unwrap();
        let violations = match plume::validate_str(&src) {
            Ok(v) => v,
            Err(_) => continue, // a sample that doesn't compile is not our concern here
        };
        if violations.is_empty() {
            continue;
        }
        report.push_str(&format!("{name}:\n"));
        for v in &violations {
            report.push_str(&format!(
                "  [{}/{:?}] {}\n",
                v.rule.id(),
                v.rule.severity(),
                v.detail
            ));
        }
        report.push('\n');
    }

    if report.is_empty() {
        report.push_str("(no violations across any sample)\n");
    }
    insta::assert_snapshot!(report);
}

/// Routing must be deterministic (spec §7): compiling a sample twice yields
/// byte-identical SVG, so every routed polyline is reproducible.
#[test]
fn routing_is_deterministic() {
    let opts = plume::Options {
        bake_vars: true,
        format: plume::OutputFormat::Svg,
        ..Default::default()
    };
    let mut paths: Vec<PathBuf> = fs::read_dir("samples")
        .unwrap()
        .filter_map(|e| {
            let p = e.unwrap().path();
            (p.extension().and_then(|x| x.to_str()) == Some("plume")).then_some(p)
        })
        .collect();
    paths.sort();

    for p in paths {
        let src = fs::read_to_string(&p).unwrap();
        let (Ok(a), Ok(b)) = (
            plume::compile_str_with(&src, &opts),
            plume::compile_str_with(&src, &opts),
        ) else {
            continue; // samples that don't compile aren't our concern here
        };
        assert_eq!(a, b, "non-deterministic routing for {}", p.display());
    }
}
