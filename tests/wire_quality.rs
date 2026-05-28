//! Routing *quality* gate — the closed loop beyond the legality validator.
//!
//! `validate_str` (tests/wire_rules) proves a route is *legal* (orthogonal,
//! clears shapes, separated, perpendicular). It says nothing about whether the
//! route is *good*: a wire can be legal and still wrap the whole canvas or
//! tangle. This test measures the aesthetic properties the spec's §1/§8 demand —
//! shortest, fewest bends, no canvas-wide detours, no needless crossings — and
//! both snapshots them (regression tripwire) and hard-asserts the worst ones.

use std::fmt::Write;
use std::fs;
use std::path::PathBuf;

const EPS: f64 = 0.5;

fn samples() -> Vec<PathBuf> {
    let mut paths: Vec<PathBuf> = fs::read_dir("samples")
        .unwrap()
        .filter_map(|e| {
            let p = e.unwrap().path();
            (p.extension().and_then(|x| x.to_str()) == Some("plume")).then_some(p)
        })
        .collect();
    paths.sort();
    paths
}

fn seg_len(a: (f64, f64), b: (f64, f64)) -> f64 {
    (a.0 - b.0).abs() + (a.1 - b.1).abs()
}

fn path_len(pts: &[(f64, f64)]) -> f64 {
    pts.windows(2).map(|w| seg_len(w[0], w[1])).sum()
}

fn bends(pts: &[(f64, f64)]) -> usize {
    if pts.len() < 3 {
        return 0;
    }
    (1..pts.len() - 1)
        .filter(|&i| {
            let h0 = (pts[i - 1].1 - pts[i].1).abs() < EPS;
            let h1 = (pts[i].1 - pts[i + 1].1).abs() < EPS;
            h0 != h1
        })
        .count()
}

/// Ratio of routed length to the straight orthogonal (Manhattan) distance
/// between the two *shape centres*. Using centres (not the chosen attachment
/// points) makes the metric edge-choice independent: a route that reaches a
/// shape from its far side — wrapping the canvas — blows this up, exactly the
/// "no canvas-wide detour" defect, whereas measuring to attachments would hide
/// it (the bad attachment inflates the baseline).
fn detour_ratio(w: &plume::WirePath) -> f64 {
    let ideal = seg_len(w.from_center, w.to_center).max(1.0);
    path_len(&w.points) / ideal
}

/// Count distinct points where two segments from *different* wires cross.
fn crossings(wires: &[plume::WirePath]) -> usize {
    let mut n = 0;
    for i in 0..wires.len() {
        for j in (i + 1)..wires.len() {
            for a in wires[i].points.windows(2) {
                for b in wires[j].points.windows(2) {
                    if segments_cross(a[0], a[1], b[0], b[1]) {
                        n += 1;
                    }
                }
            }
        }
    }
    n
}

/// True if axis-aligned segments `a0a1` and `b0b1` intersect at an interior
/// point (perpendicular crossing). Shared endpoints and collinear overlaps
/// don't count here — this metric is about visual crossings.
fn segments_cross(a0: (f64, f64), a1: (f64, f64), b0: (f64, f64), b1: (f64, f64)) -> bool {
    let a_h = (a0.1 - a1.1).abs() < EPS;
    let b_h = (b0.1 - b1.1).abs() < EPS;
    if a_h == b_h {
        return false; // parallel — not a perpendicular crossing
    }
    let (h0, h1, v0, v1) = if a_h {
        (a0, a1, b0, b1)
    } else {
        (b0, b1, a0, a1)
    };
    let hy = h0.1;
    let vx = v0.0;
    let (hx_lo, hx_hi) = (h0.0.min(h1.0), h0.0.max(h1.0));
    let (vy_lo, vy_hi) = (v0.1.min(v1.1), v0.1.max(v1.1));
    // strict interior on both, so a T-junction at an endpoint isn't a crossing
    vx > hx_lo + EPS && vx < hx_hi - EPS && hy > vy_lo + EPS && hy < vy_hi - EPS
}

struct Report {
    text: String,
    worst_detour: f64,
    total_crossings: usize,
}

fn measure() -> Report {
    let mut text = String::new();
    let mut worst_detour: f64 = 1.0;
    let mut total_crossings = 0;
    for p in samples() {
        let src = fs::read_to_string(&p).unwrap();
        let Ok(wires) = plume::route_str(&src) else {
            continue;
        };
        if wires.is_empty() {
            continue;
        }
        let name = p.file_name().unwrap().to_string_lossy();
        let total_len: f64 = wires.iter().map(|w| path_len(&w.points)).sum();
        let total_bends: usize = wires.iter().map(|w| bends(&w.points)).sum();
        let cross = crossings(&wires);
        total_crossings += cross;
        let worst = wires
            .iter()
            .map(|w| (detour_ratio(w), w))
            .max_by(|a, b| a.0.total_cmp(&b.0))
            .unwrap();
        worst_detour = worst_detour.max(worst.0);
        writeln!(
            text,
            "{name}: len={total_len:.0} bends={total_bends} crossings={cross} \
             worst_detour={:.2}x ({}->{})",
            worst.0, worst.1.from, worst.1.to
        )
        .unwrap();
    }
    Report {
        text,
        worst_detour,
        total_crossings,
    }
}

#[test]
fn routing_quality_report() {
    let r = measure();
    insta::assert_snapshot!(r.text);
}

/// Hard gate: no wire may wrap the canvas to reach its target. A detour ratio
/// above this means the route went the long way round when a shorter legal one
/// almost certainly existed. Tightened as the router improves.
#[test]
fn no_canvas_wide_detours() {
    let r = measure();
    assert!(
        r.worst_detour <= 3.0,
        "a wire detours {:.2}x its ideal length (canvas-wide wrap):\n{}",
        r.worst_detour,
        r.text
    );
}

/// Hard gate on total visual crossings across all samples. A tangle of wires
/// crossing each other reads as messy even when every crossing is legal.
#[test]
fn crossings_bounded() {
    let r = measure();
    assert!(
        r.total_crossings <= 12,
        "too many wire crossings ({}):\n{}",
        r.total_crossings,
        r.text
    );
}
