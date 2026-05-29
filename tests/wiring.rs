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
fn routing_relaxations_are_surfaced_as_diagnostics() {
    // wires_labels packs 5 wires onto one short edge — a genuine C5 overflow whose
    // sub-separation is a flagged B2 relaxation. It must reach the user as a
    // diagnostic, never silently.
    let src = std::fs::read_to_string("samples/wires_labels.plume").unwrap();
    let diags = plume::routing_diagnostics(&src).expect("routing diagnostics");
    assert!(
        diags.iter().any(|d| d.message.contains("separation")),
        "the C5 overflow must be flagged: {diags:?}"
    );
}

#[test]
fn a_rule_clean_sample_has_no_routing_diagnostics() {
    let src = std::fs::read_to_string("samples/wires_basic.plume").unwrap();
    assert!(
        plume::routing_diagnostics(&src).expect("diags").is_empty(),
        "a clean routing emits nothing"
    );
}

#[test]
fn dumb_router_holds_per_wire_invariants() {
    for path in sample_paths() {
        let src = std::fs::read_to_string(&path).unwrap();
        let violations = plume::validate_str(&src).expect("validate");
        // Gate the per-wire invariants the dumb router guarantees by construction
        // (A1/A2/A4/A5). A3 (PerpCrossing) is a multi-wire property it can't yet
        // satisfy, and the B-constraints are the measured baseline, not a gate.
        let blocking: Vec<&plume::Violation> = violations
            .iter()
            .filter(|v| {
                v.rule.severity() == plume::Severity::Invariant
                    && v.rule != plume::Rule::PerpCrossing
            })
            .collect();
        assert!(
            blocking.is_empty(),
            "{}: per-wire invariant violations: {:?}",
            path.display(),
            blocking
        );
    }
}

fn count_rule(src: &str, rule: plume::Rule) -> usize {
    plume::validate_str(src)
        .expect("validate")
        .iter()
        .filter(|v| v.rule == rule)
        .count()
}

/// Tally violations per rule for one source: indices follow the column order
/// A1, A2, A3, A4, A5, B1, B2-node, B2-wire, B3.
fn rule_counts(src: &str) -> [usize; 9] {
    let mut c = [0usize; 9];
    for v in plume::validate_str(src).expect("validate") {
        let i = match v.rule {
            plume::Rule::Orthogonality => 0,
            plume::Rule::Attachment => 1,
            plume::Rule::PerpCrossing => 2,
            plume::Rule::SidesOnly => 3,
            plume::Rule::SelfCross => 4,
            plume::Rule::NodeOverlap => 5,
            plume::Rule::Clearance => 6,
            plume::Rule::Separation => 7,
            plume::Rule::Crossing => 8,
        };
        c[i] += 1;
    }
    c
}

/// The router's contract scorecard across the whole sample suite. Invariants
/// (A1–A5) and B1/B2n hold everywhere; A3 shared runs are gone; the only B2w left
/// is `wires_labels`, where five wires are crammed onto one tiny edge — genuine
/// C5 overflow that WIRING flags rather than removes. X counts perpendicular
/// crossings, which are normal output.
#[test]
fn baseline_contract_report() {
    use std::fmt::Write;
    let mut report = String::new();
    report.push_str("Router contract scorecard — validator counts per sample.\n");
    report.push_str("A1–A5 invariants; A3 = shared parallel runs; B1 = node overlap;\n");
    report.push_str("B2n = wire-node clearance; B2w = wire-wire separation; X = B3 crossings.\n\n");

    let mut totals = [0usize; 9];
    for path in sample_paths() {
        let src = std::fs::read_to_string(&path).unwrap();
        let c = rule_counts(&src);
        for (t, v) in totals.iter_mut().zip(c) {
            *t += v;
        }
        let name = path.file_name().unwrap().to_string_lossy();
        writeln!(
            report,
            "{name:<22} A1:{} A2:{} A3:{} A4:{} A5:{}  B1:{} B2n:{} B2w:{}  X:{}",
            c[0], c[1], c[2], c[3], c[4], c[5], c[6], c[7], c[8]
        )
        .unwrap();
    }
    writeln!(
        report,
        "\n{:<22} A1:{} A2:{} A3:{} A4:{} A5:{}  B1:{} B2n:{} B2w:{}  X:{}",
        "TOTAL",
        totals[0],
        totals[1],
        totals[2],
        totals[3],
        totals[4],
        totals[5],
        totals[6],
        totals[7],
        totals[8]
    )
    .unwrap();

    insta::assert_snapshot!(report);
}

#[test]
fn router_threads_around_a_blocking_box() {
    // via sits between src and dst; with gap (30) > clearance (16) the A* router
    // must detour around it — neither piercing it (B1) nor grazing it (B2n).
    let source = "{ |scene| layout:row gap:30 }\n\
                  src |rect| size:(40,40)\n\
                  via |rect| size:(40,40)\n\
                  dst |rect| size:(40,40)\n\
                  src -> dst\n";
    assert_eq!(
        count_rule(source, plume::Rule::NodeOverlap),
        0,
        "must not pierce via"
    );
    assert_eq!(
        count_rule(source, plume::Rule::Clearance),
        0,
        "must keep clearance from via"
    );
}

#[test]
fn wire_to_a_text_node_violates_sides_only() {
    // Wires attach to shape sides only, never to a text node (A4).
    let source = "box |rect| size:(40,40)\n\
                  txt |text| \"hi\"\n\
                  box -> txt\n";
    assert_eq!(count_rule(source, plume::Rule::SidesOnly), 1);
}

#[test]
fn perpendicular_wires_count_as_a_crossing() {
    // A horizontal wire and a vertical wire meet at one interior point (B3).
    let source = "h1 |rect| size:(20,20) at:(0,0)\n\
                  h2 |rect| size:(20,20) at:(200,0)\n\
                  v1 |rect| size:(20,20) at:(100,-100)\n\
                  v2 |rect| size:(20,20) at:(100,100)\n\
                  h1 -> h2\n\
                  v1 -> v2\n";
    assert_eq!(count_rule(source, plume::Rule::Crossing), 1);
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
