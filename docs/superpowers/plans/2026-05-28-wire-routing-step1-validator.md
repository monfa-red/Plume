# Wire Routing — Step 1: Clearance Oracle + Validator — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a single clearance oracle and an independent validator that checks the routing contract (R1–R6) over the laid-out scene, then snapshot a baseline "what's broken" report across all samples — with **no change to routing behavior**.

**Architecture:** Two new modules under `src/layout/wires/`: `oracle.rs` (the only authority for shape clearance and wire separation) and `validate.rs` (pure geometric checks composed into `validate_routing`). The validator reuses the existing `SceneIndex` (data only) and the oracle; it shares no routing-decision code. A new public `plume::validate_str` exposes it; a snapshot test captures the baseline.

**Tech Stack:** Rust, `insta` (snapshot tests, already a dev-dep). No new dependencies.

**Reference spec:** `docs/superpowers/specs/2026-05-28-wire-routing-rules-design.md` (the contract R1–R6, clearance = parent gap, separation = max of wire gaps).

---

## File Structure

| File | Responsibility |
|------|----------------|
| `src/layout/ir.rs` (modify) | Add provenance fields to `RoutedWire`: `seg_from`, `seg_to` (this segment's endpoint ids), `decl_span` (the wire declaration's span, for grouping siblings). |
| `src/layout/wires/scene.rs` (modify) | Store each node's full path; factor a `passable_set` helper; add `raw_obstacles` (un-inflated, path-tagged). |
| `src/layout/wires/mod.rs` (modify) | Declare `oracle`/`validate` modules; re-export validator types; populate the new `RoutedWire` fields in `build_routed_wire`. |
| `src/layout/wires/oracle.rs` (create) | `shape_clearance`, `wire_separation`, `wire_gap` — the single clearance authority. |
| `src/layout/wires/validate.rs` (create) | `Violation`/`Rule`/`Severity`, the pure per-rule predicates, and `validate_routing`. |
| `src/layout/mod.rs` (modify) | `pub fn validate_routing(&LaidOut)`; re-export validator types. |
| `src/lib.rs` (modify) | `pub fn validate_str`; re-export validator types. |
| `tests/wire_rules.rs` (create) | Baseline snapshot test over `samples/`. |

**Notes for the engineer (zero-context assumptions):**
- `AttrMap = HashMap<String, ResolvedValue>` (from `crate::resolve`). `ResolvedValue::Number(f64)` is the numeric variant.
- `Span` is `crate::span::Span`; `RoutedWire`/`ResolvedWire` already carry spans (`ResolvedWire::span`).
- `AbsBbox` (in `wires/geometry.rs`) has `x, y, w, h` and methods `right()`, `bottom()`, `inflate(by)`.
- The router guarantees each wire's polyline starts/ends exactly on a shape edge — that is what R5 verifies.
- Tolerance everywhere is `EPS = 0.5` (matches the rest of the routing code).
- **Do not** repoint the existing router at the oracle in this step — that changes behavior and is Step 2's job. Step 1 is purely additive.

---

## Task 1: Provenance fields on `RoutedWire`

**Files:**
- Modify: `src/layout/ir.rs:18-29` (the `RoutedWire` struct)
- Modify: `src/layout/wires/mod.rs:767-791` (`build_routed_wire`)

- [ ] **Step 1: Add the three fields to `RoutedWire`**

In `src/layout/ir.rs`, the struct becomes:

```rust
#[derive(Clone)]
pub struct RoutedWire {
    /// Orthogonal polyline through the scene, in scene coordinates.
    pub path: Vec<(f64, f64)>,
    pub markers: Markers,
    pub attrs: AttrMap,
    pub texts: Vec<RoutedText>,
    /// First and last endpoint IDs of the chain this segment belongs to —
    /// emitted as `data-from` / `data-to` for CSS / a11y hooks.
    pub data_from: String,
    pub data_to: String,
    /// This segment's own endpoint ids (resolved dot-paths). For a chain
    /// `a -> b -> c`, the `b -> c` segment has `seg_from = "b"`, `seg_to = "c"`
    /// (whereas `data_from`/`data_to` stay the chain ends `a`/`c`). The
    /// validator uses these to know which shapes the wire is allowed to touch.
    pub seg_from: String,
    pub seg_to: String,
    /// Span of the wire *declaration* this segment came from. Segments sharing
    /// a `decl_span` are siblings of one statement (chain links or a `a -> b & c`
    /// fan-out) and are exempt from wire-spacing checks where they coincide.
    pub decl_span: Span,
}
```

`Span` is already imported at the top of `ir.rs` (`use crate::span::Span;`).

- [ ] **Step 2: Populate them in `build_routed_wire`**

In `src/layout/wires/mod.rs`, `build_routed_wire` already receives `spec: &SegmentSpec`. Add the three fields to the returned struct (everything else stays):

```rust
fn build_routed_wire(spec: &SegmentSpec, path: Vec<(f64, f64)>) -> RoutedWire {
    RoutedWire {
        markers: Markers {
            start: if spec.is_first {
                spec.wire.markers.start
            } else {
                MarkerKind::None
            },
            end: if spec.is_last {
                spec.wire.markers.end
            } else {
                MarkerKind::None
            },
        },
        attrs: spec.wire.attrs.clone(),
        texts: if spec.is_first {
            place_texts(&spec.wire.texts, &path)
        } else {
            Vec::new()
        },
        data_from: spec.data_from.clone(),
        data_to: spec.data_to.clone(),
        seg_from: spec.src_id.clone(),
        seg_to: spec.tgt_id.clone(),
        decl_span: spec.wire.span,
        path,
    }
}
```

- [ ] **Step 3: Build and run the suite to confirm no behavior change**

Run: `cargo build && cargo test`
Expected: PASS. No snapshot changes (the new fields are not rendered). If `insta` reports changes, something else is wrong — investigate, don't accept.

- [ ] **Step 4: Commit**

```bash
git add src/layout/ir.rs src/layout/wires/mod.rs
git commit -m "wire ir: add seg_from/seg_to/decl_span provenance to RoutedWire"
```

---

## Task 2: Scene index — full path + raw obstacles

**Files:**
- Modify: `src/layout/wires/scene.rs`

- [ ] **Step 1: Store the full path on each indexed node**

Add a `path` field to `IndexedNode` and set it during `walk`. In the struct:

```rust
struct IndexedNode {
    bbox: AbsBbox,
    /// Indices into `nodes` for every ancestor that has an id, root-first.
    ancestors: Vec<usize>,
    is_leaf: bool,
    clearance: f64,
    /// Fully-qualified dot-path of this node (same key as `by_path`).
    path: String,
}
```

In `walk`, inside `if let Some(id) = &node.id {` — `full_path` is already computed; pass it in:

```rust
            self.nodes.push(IndexedNode {
                bbox,
                ancestors: ancestors.to_vec(),
                is_leaf: node.children.is_empty(),
                clearance,
                path: full_path.clone(),
            });
            self.by_path.insert(full_path, i);
```

- [ ] **Step 2: Factor the passable-set logic out of `obstacles_for`**

Add a private helper and rewrite `obstacles_for` to use it (behavior identical):

```rust
    /// Indices of nodes a wire between `src_id` and `tgt_id` may cross: the
    /// endpoints and all their named ancestors.
    fn passable_set(&self, src_id: &str, tgt_id: &str) -> Vec<usize> {
        let mut passable: Vec<usize> = Vec::new();
        for id in [src_id, tgt_id] {
            if let Some(&i) = self.by_path.get(id) {
                passable.push(i);
                passable.extend(self.nodes[i].ancestors.iter().copied());
            }
        }
        passable
    }
```

Then `obstacles_for`'s head becomes:

```rust
    pub fn obstacles_for(&self, src_id: &str, tgt_id: &str, wire_gap: f64) -> Vec<AbsBbox> {
        let passable = self.passable_set(src_id, tgt_id);
        let cap = wire_gap * 2.0;
        let mut out = Vec::new();
        for (i, n) in self.nodes.iter().enumerate() {
            if passable.contains(&i) {
                continue;
            }
            if !n.ancestors.iter().all(|a| passable.contains(a)) {
                continue;
            }
            if !n.is_leaf && n.bbox.w == 0.0 && n.bbox.h == 0.0 {
                continue;
            }
            let pad = wire_gap.max(n.clearance.min(cap));
            out.push(n.bbox.inflate(pad));
        }
        out
    }
```

- [ ] **Step 3: Add `raw_obstacles` (path-tagged, un-inflated)**

```rust
    /// Like `obstacles_for` but returns each obstacle's *path* and its
    /// *un-inflated* bbox. The validator inflates by the oracle clearance
    /// itself, so this stays free of any clearance policy.
    pub fn raw_obstacles(&self, src_id: &str, tgt_id: &str) -> Vec<(String, AbsBbox)> {
        let passable = self.passable_set(src_id, tgt_id);
        let mut out = Vec::new();
        for (i, n) in self.nodes.iter().enumerate() {
            if passable.contains(&i) {
                continue;
            }
            if !n.ancestors.iter().all(|a| passable.contains(a)) {
                continue;
            }
            if !n.is_leaf && n.bbox.w == 0.0 && n.bbox.h == 0.0 {
                continue;
            }
            out.push((n.path.clone(), n.bbox));
        }
        out
    }
```

- [ ] **Step 4: Build and test**

Run: `cargo build && cargo test`
Expected: PASS, no snapshot changes (only additive + a behavior-preserving refactor).

- [ ] **Step 5: Commit**

```bash
git add src/layout/wires/scene.rs
git commit -m "wire scene: store node path; add raw_obstacles; factor passable_set"
```

---

## Task 3: The clearance oracle

**Files:**
- Create: `src/layout/wires/oracle.rs`
- Modify: `src/layout/wires/mod.rs` (add `mod oracle;`)

- [ ] **Step 1: Write the module with unit tests first**

Create `src/layout/wires/oracle.rs`:

```rust
//! The single authority for wire clearance distances.
//!
//! Every phase that needs "how far must a wire stay from this shape?" or
//! "how far apart must these two wires sit?" calls these functions — never
//! its own inline math. Per the rules-spec, shape clearance is the shape's
//! parent-container gap (already computed per-node by `SceneIndex`), and wire
//! separation is the larger of the two wires' `gap` attrs.

use super::scene::SceneIndex;
use crate::layout::ir::RoutedWire;
use crate::layout::values::layout_var;
use crate::resolve::{ResolvedValue, VarTable};

/// Minimum distance a wire must keep from obstacle `shape` — the gap of the
/// shape's parent container (scene gap for a top-level shape). `0.0` if the
/// shape is unknown.
pub fn shape_clearance(scene: &SceneIndex, shape: &str) -> f64 {
    scene.clearance(shape).unwrap_or(0.0)
}

/// Minimum distance two wires must keep from each other — the larger of their
/// gaps, so the more generous wire wins.
pub fn wire_separation(gap_a: f64, gap_b: f64) -> f64 {
    gap_a.max(gap_b)
}

/// The wire's own gap: its `gap` attr, else the `--plume-wire-gap` layout
/// default (16). Mirrors `planning::wire_gap` so the validator measures wires
/// the same way the router spaced them.
pub fn wire_gap(wire: &RoutedWire, vars: &VarTable) -> f64 {
    if let Some(ResolvedValue::Number(n)) = wire.attrs.get("gap") {
        return *n;
    }
    layout_var(vars, "wire-gap").unwrap_or(16.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn separation_is_the_larger_gap() {
        assert_eq!(wire_separation(8.0, 16.0), 16.0);
        assert_eq!(wire_separation(20.0, 5.0), 20.0);
    }
}
```

- [ ] **Step 2: Register the module**

In `src/layout/wires/mod.rs`, add to the module list near the top (keep alphabetical with the others):

```rust
mod channels;
mod endpoints;
mod geometry;
mod lanes;
mod oracle;
mod planning;
mod route;
mod scene;
mod stamping;
mod text;
mod validate;
```

(`validate` is added now too so the next task compiles; it will exist after Task 4. If building between tasks, add `mod validate;` only in Task 4 instead.)

- [ ] **Step 3: Build and test the oracle**

Run: `cargo test -p plume oracle`
Expected: PASS (`separation_is_the_larger_gap`).

> If `cargo test -p plume` errors that `validate` is missing, temporarily comment `mod validate;` until Task 4, or add the modules in their own tasks.

- [ ] **Step 4: Commit**

```bash
git add src/layout/wires/oracle.rs src/layout/wires/mod.rs
git commit -m "wire oracle: single authority for shape clearance + wire separation"
```

---

## Task 4: The validator

**Files:**
- Create: `src/layout/wires/validate.rs`
- Modify: `src/layout/wires/mod.rs` (ensure `mod validate;` + re-exports)

- [ ] **Step 1: Write the pure geometry predicates with unit tests first**

Create `src/layout/wires/validate.rs` with the predicates and their tests (no `validate_routing` yet):

```rust
//! Independent routing validator — checks the contract R1–R6 from the
//! rules-spec against the final polylines. Shares no decision code with the
//! router: it re-derives every check from geometry, reusing only the scene
//! index (data) and the clearance oracle.

use super::geometry::AbsBbox;
use super::oracle;
use super::scene::SceneIndex;
use crate::layout::ir::{PlacedNode, RoutedWire};
use crate::resolve::{AttrMap, VarTable};

const EPS: f64 = 0.5;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Rule {
    Orthogonality, // R1
    ShapeClearance, // R2
    WireSpacing,   // R3
    Crossing,      // R4
    Attachment,    // R5
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    Invariant,
    Error,
    Warning,
}

impl Rule {
    pub fn id(self) -> &'static str {
        match self {
            Rule::Orthogonality => "R1",
            Rule::ShapeClearance => "R2",
            Rule::WireSpacing => "R3",
            Rule::Crossing => "R4",
            Rule::Attachment => "R5",
        }
    }
    pub fn severity(self) -> Severity {
        match self {
            Rule::Orthogonality | Rule::Crossing | Rule::Attachment => Severity::Invariant,
            Rule::ShapeClearance => Severity::Error,
            Rule::WireSpacing => Severity::Warning,
        }
    }
}

#[derive(Debug, Clone)]
pub struct Violation {
    pub rule: Rule,
    pub detail: String,
}

fn order(a: f64, b: f64) -> (f64, f64) {
    if a <= b {
        (a, b)
    } else {
        (b, a)
    }
}

fn approx(a: f64, b: f64) -> bool {
    (a - b).abs() < EPS
}

/// Axis of an axis-aligned segment: `Some(true)` horizontal, `Some(false)`
/// vertical, `None` if degenerate (zero length) or diagonal.
fn axis(a: (f64, f64), b: (f64, f64)) -> Option<bool> {
    let dx = (a.0 - b.0).abs();
    let dy = (a.1 - b.1).abs();
    if dy < EPS && dx >= EPS {
        Some(true)
    } else if dx < EPS && dy >= EPS {
        Some(false)
    } else {
        None
    }
}

/// True if axis-aligned segment `a→b` has any point strictly inside `bx`.
fn segment_pierces_box(a: (f64, f64), b: (f64, f64), bx: &AbsBbox) -> bool {
    let (x_lo, x_hi) = order(a.0, b.0);
    let (y_lo, y_hi) = order(a.1, b.1);
    let x_overlap = x_lo < bx.right() - EPS && x_hi > bx.x + EPS;
    let y_overlap = y_lo < bx.bottom() - EPS && y_hi > bx.y + EPS;
    x_overlap && y_overlap
}

/// For a pair of segments from *different* wires, return a violation if they
/// run parallel within `sep` (R3) or overlap collinearly (R4). Perpendicular
/// crossings return `None` — they are legal by construction.
fn pair_violation(
    sa0: (f64, f64),
    sa1: (f64, f64),
    sb0: (f64, f64),
    sb1: (f64, f64),
    sep: f64,
) -> Option<Rule> {
    match (axis(sa0, sa1), axis(sb0, sb1)) {
        (Some(true), Some(true)) => {
            let dist = (sa0.1 - sb0.1).abs();
            let (axl, axh) = order(sa0.0, sa1.0);
            let (bxl, bxh) = order(sb0.0, sb1.0);
            let overlap = axl < bxh - EPS && bxl < axh - EPS;
            classify(overlap, dist, sep)
        }
        (Some(false), Some(false)) => {
            let dist = (sa0.0 - sb0.0).abs();
            let (ayl, ayh) = order(sa0.1, sa1.1);
            let (byl, byh) = order(sb0.1, sb1.1);
            let overlap = ayl < byh - EPS && byl < ayh - EPS;
            classify(overlap, dist, sep)
        }
        _ => None, // perpendicular (or a degenerate segment) — fine
    }
}

fn classify(overlap: bool, dist: f64, sep: f64) -> Option<Rule> {
    if !overlap {
        return None;
    }
    if dist <= EPS {
        Some(Rule::Crossing) // collinear overlap — illegal
    } else if dist < sep - EPS {
        Some(Rule::WireSpacing) // parallel, too close
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bx(x: f64, y: f64, w: f64, h: f64) -> AbsBbox {
        AbsBbox { x, y, w, h }
    }

    #[test]
    fn axis_classifies_segments() {
        assert_eq!(axis((0.0, 0.0), (10.0, 0.0)), Some(true));
        assert_eq!(axis((0.0, 0.0), (0.0, 10.0)), Some(false));
        assert_eq!(axis((0.0, 0.0), (0.0, 0.0)), None);
        assert_eq!(axis((0.0, 0.0), (10.0, 10.0)), None);
    }

    #[test]
    fn pierces_box_detects_crossing() {
        let b = bx(-5.0, -5.0, 10.0, 10.0); // covers [-5,5]^2
        assert!(segment_pierces_box((-20.0, 0.0), (20.0, 0.0), &b));
        assert!(!segment_pierces_box((-20.0, 50.0), (20.0, 50.0), &b));
    }

    #[test]
    fn parallel_too_close_is_r3() {
        // two horizontal segments 6 apart, separation 16, x-overlap
        assert_eq!(
            pair_violation((0.0, 0.0), (50.0, 0.0), (10.0, 6.0), (60.0, 6.0), 16.0),
            Some(Rule::WireSpacing)
        );
    }

    #[test]
    fn parallel_at_separation_is_clean() {
        assert_eq!(
            pair_violation((0.0, 0.0), (50.0, 0.0), (10.0, 16.0), (60.0, 16.0), 16.0),
            None
        );
    }

    #[test]
    fn collinear_overlap_is_r4() {
        assert_eq!(
            pair_violation((0.0, 0.0), (50.0, 0.0), (25.0, 0.0), (75.0, 0.0), 16.0),
            Some(Rule::Crossing)
        );
    }

    #[test]
    fn perpendicular_crossing_is_clean() {
        assert_eq!(
            pair_violation((0.0, 0.0), (50.0, 0.0), (25.0, -20.0), (25.0, 20.0), 16.0),
            None
        );
    }

    #[test]
    fn no_overlap_no_violation() {
        // parallel and close, but x-extents don't overlap
        assert_eq!(
            pair_violation((0.0, 0.0), (10.0, 0.0), (50.0, 4.0), (60.0, 4.0), 16.0),
            None
        );
    }
}
```

- [ ] **Step 2: Run the predicate tests; verify they pass**

Run: `cargo test -p plume validate`
Expected: PASS (7 tests).

- [ ] **Step 3: Add the per-wire checks (R1, R5, R2) and the composer**

Append to `src/layout/wires/validate.rs`:

```rust
fn check_orthogonal(w: &RoutedWire, out: &mut Vec<Violation>) {
    let n = w.path.len();
    if n < 2 {
        return;
    }
    for win in w.path.windows(2) {
        if axis(win[0], win[1]).is_none() {
            out.push(Violation {
                rule: Rule::Orthogonality,
                detail: format!(
                    "{}->{}: non-orthogonal or zero-length segment {:?}->{:?}",
                    w.seg_from, w.seg_to, win[0], win[1]
                ),
            });
        }
    }
    // Interior vertices must be 90° turns (axis flips), never collinear.
    for i in 1..n - 1 {
        let a = axis(w.path[i - 1], w.path[i]);
        let b = axis(w.path[i], w.path[i + 1]);
        if let (Some(ax), Some(bx)) = (a, b) {
            if ax == bx {
                out.push(Violation {
                    rule: Rule::Orthogonality,
                    detail: format!(
                        "{}->{}: collinear/ redundant vertex at {:?}",
                        w.seg_from, w.seg_to, w.path[i]
                    ),
                });
            }
        }
    }
}

fn check_attachment(w: &RoutedWire, scene: &SceneIndex, out: &mut Vec<Violation>) {
    let n = w.path.len();
    if n < 2 {
        return;
    }
    if let Some(s) = scene.lookup(&w.seg_from) {
        if !end_on_edge_perp(w.path[0], w.path[1], &s.bbox) {
            out.push(Violation {
                rule: Rule::Attachment,
                detail: format!(
                    "{}->{}: source end {:?} not on a perpendicular edge of '{}'",
                    w.seg_from, w.seg_to, w.path[0], w.seg_from
                ),
            });
        }
    }
    if let Some(t) = scene.lookup(&w.seg_to) {
        if !end_on_edge_perp(w.path[n - 1], w.path[n - 2], &t.bbox) {
            out.push(Violation {
                rule: Rule::Attachment,
                detail: format!(
                    "{}->{}: target end {:?} not on a perpendicular edge of '{}'",
                    w.seg_from, w.seg_to, w.path[n - 1], w.seg_to
                ),
            });
        }
    }
}

/// True if `p` lies on an edge of `b` and the segment `p→next` leaves that
/// edge perpendicularly (horizontal off a left/right edge, vertical off a
/// top/bottom edge).
fn end_on_edge_perp(p: (f64, f64), next: (f64, f64), b: &AbsBbox) -> bool {
    let on_v_edge = (approx(p.0, b.x) || approx(p.0, b.right()))
        && p.1 >= b.y - EPS
        && p.1 <= b.bottom() + EPS;
    let on_h_edge = (approx(p.1, b.y) || approx(p.1, b.bottom()))
        && p.0 >= b.x - EPS
        && p.0 <= b.right() + EPS;
    match axis(p, next) {
        Some(true) => on_v_edge,   // horizontal segment ⟂ a vertical edge
        Some(false) => on_h_edge,  // vertical segment ⟂ a horizontal edge
        None => false,
    }
}

fn check_shape_clearance(w: &RoutedWire, scene: &SceneIndex, out: &mut Vec<Violation>) {
    let obstacles = scene.raw_obstacles(&w.seg_from, &w.seg_to);
    for win in w.path.windows(2) {
        for (path, bbox) in &obstacles {
            let c = oracle::shape_clearance(scene, path);
            // No-go zone = bbox grown by clearance. `-EPS` so a segment lying
            // exactly at distance `c` is accepted, not flagged.
            let zone = bbox.inflate((c - EPS).max(0.0));
            if segment_pierces_box(win[0], win[1], &zone) {
                out.push(Violation {
                    rule: Rule::ShapeClearance,
                    detail: format!(
                        "{}->{}: segment {:?}->{:?} within {} of shape '{}'",
                        w.seg_from, w.seg_to, win[0], win[1], c, path
                    ),
                });
            }
        }
    }
}

fn check_pair(a: &RoutedWire, b: &RoutedWire, vars: &VarTable, out: &mut Vec<Violation>) {
    if a.decl_span == b.decl_span {
        return; // same declaration: chain links or fan-out siblings — exempt
    }
    let sep = oracle::wire_separation(oracle::wire_gap(a, vars), oracle::wire_gap(b, vars));
    for sa in a.path.windows(2) {
        for sb in b.path.windows(2) {
            if let Some(rule) = pair_violation(sa[0], sa[1], sb[0], sb[1], sep) {
                out.push(Violation {
                    rule,
                    detail: format!(
                        "{}->{} vs {}->{}: {} (sep {})",
                        a.seg_from, a.seg_to, b.seg_from, b.seg_to, rule.id(), sep
                    ),
                });
            }
        }
    }
}

/// Validate the full routing of a laid-out scene against the contract.
pub fn validate_routing(
    nodes: &[PlacedNode],
    scene_attrs: &AttrMap,
    wires: &[RoutedWire],
    vars: &VarTable,
) -> Vec<Violation> {
    let scene = SceneIndex::build(nodes, scene_attrs);
    let mut out = Vec::new();
    for w in wires {
        check_orthogonal(w, &mut out);
        check_attachment(w, &scene, &mut out);
        check_shape_clearance(w, &scene, &mut out);
    }
    for i in 0..wires.len() {
        for j in (i + 1)..wires.len() {
            check_pair(&wires[i], &wires[j], vars, &mut out);
        }
    }
    out
}
```

- [ ] **Step 4: Re-export the validator from the wires module**

In `src/layout/wires/mod.rs`, after the `mod` list / `use` block, add:

```rust
pub use validate::{validate_routing, Rule, Severity, Violation};
```

- [ ] **Step 5: Build and run all wire tests**

Run: `cargo test -p plume wires`
Expected: PASS (oracle + validate unit tests; existing wire tests unchanged).

- [ ] **Step 6: Commit**

```bash
git add src/layout/wires/validate.rs src/layout/wires/mod.rs
git commit -m "wire validator: R1-R5 checks over routed polylines"
```

---

## Task 5: Expose `validate_str` + baseline snapshot

**Files:**
- Modify: `src/layout/mod.rs` (wrapper + re-exports)
- Modify: `src/lib.rs` (`validate_str` + re-exports)
- Create: `tests/wire_rules.rs`

- [ ] **Step 1: Add the `LaidOut` wrapper and re-exports in `layout`**

In `src/layout/mod.rs`, below `pub use ir::*;` add:

```rust
pub use wires::{Rule, Severity, Violation};

/// Run the routing validator over a laid-out scene. See
/// `docs/superpowers/specs/2026-05-28-wire-routing-rules-design.md`.
pub fn validate_routing(laid: &LaidOut) -> Vec<Violation> {
    wires::validate_routing(&laid.nodes, &laid.scene_attrs, &laid.wires, &laid.vars)
}
```

- [ ] **Step 2: Add the public `validate_str` in `lib.rs`**

In `src/lib.rs`, extend the layout re-export line and add the function near `check_with`:

```rust
pub use layout::{Rule, Severity, Violation};

/// Lex, parse, resolve, lay out, route, then validate the routing against the
/// contract (R1–R6). Returns the list of violations (empty = clean). Parse or
/// resolve errors surface as `Err`.
pub fn validate_str(src: &str) -> Result<Vec<Violation>, Error> {
    let program = resolve_pipeline(src, &Options::default())?;
    let laid_out = layout::layout(&program)?;
    Ok(layout::validate_routing(&laid_out))
}
```

(Find the existing `pub use error::{...}` / `pub use ...` block at the top of `lib.rs` and add the `pub use layout::{Rule, Severity, Violation};` line alongside the others. `layout` is a private `mod`, but re-exporting named items from it is fine.)

- [ ] **Step 3: Build**

Run: `cargo build`
Expected: PASS.

- [ ] **Step 4: Write the baseline snapshot test**

Create `tests/wire_rules.rs`:

```rust
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
            report.push_str(&format!("  [{}/{:?}] {}\n", v.rule.id(), v.rule.severity(), v.detail));
        }
        report.push('\n');
    }

    if report.is_empty() {
        report.push_str("(no violations across any sample)\n");
    }
    insta::assert_snapshot!(report);
}
```

- [ ] **Step 5: Run the test and capture the baseline**

Run: `cargo test --test wire_rules`
Expected: FAIL the first time — `insta` reports a new snapshot. Inspect it:

Run: `cargo insta review`
Read the report. It SHOULD list real violations (e.g. `full_example.plume` R2 piercing, `wires_realistic.plume` R3 parallel-close). Sanity-check a few against the rendered PNGs (`samples/full_example` → red wire through Water = an R2 line). If the report looks geometrically wrong (e.g. flags a clearly-clean `hello.plume` wire), the validator has a bug — fix it before accepting. Once it reflects reality, accept:

Run: `cargo insta accept`

- [ ] **Step 6: Full suite + clippy + fmt**

Run: `cargo test && cargo clippy --all-targets -- -D warnings && cargo fmt --check`
Expected: PASS / clean.

- [ ] **Step 7: Commit**

```bash
git add src/layout/mod.rs src/lib.rs tests/wire_rules.rs tests/snapshots/
git commit -m "validate_str + baseline routing-contract snapshot over samples"
```

---

## Self-Review

**Spec coverage** (against the rules-spec §3–§6):
- R1 Orthogonality → `check_orthogonal` (axis + collinear-vertex check). ✓
- R2 Shape clearance → `check_shape_clearance` using `raw_obstacles` + `oracle::shape_clearance`. ✓
- R3 Wire spacing → `check_pair` / `pair_violation` (`WireSpacing`), with `decl_span` fan-out exemption. ✓
- R4 Perpendicular crossings → `pair_violation` flags collinear overlap as `Crossing`; H×V crossings return clean. ✓
- R5 Attachment → `check_attachment` / `end_on_edge_perp`. ✓
- R6 Last-resort transparency → the snapshot *is* the surfaced report; nothing is silent. ✓ (Compile-time `--strict` diagnostics are explicitly Step-2+, not in scope here.)
- Single oracle (§4) → `oracle.rs`; validator routes all clearance/separation through it. ✓ (Repointing the *router* at the oracle is Step 2 — called out in the plan header.)
- Determinism note (§7): the validator iterates `wires` and `path.windows` in order and `raw_obstacles` in node order — no `HashMap` iteration in the validator. ✓

**Placeholder scan:** no TBD/TODO; every code step shows complete code. ✓

**Type consistency:** `Violation { rule, detail }`, `Rule` (5 variants), `Severity` (3) are defined once in `validate.rs` and re-exported unchanged through `wires` → `layout` → `lib`. `validate_routing` signature is identical at definition (`validate.rs`) and call site (`layout::validate_routing`). `seg_from`/`seg_to`/`decl_span` are defined in Task 1 and consumed with the same names in Tasks 4. `raw_obstacles` returns `Vec<(String, AbsBbox)>` and is consumed as such. ✓

**Known limitation (acceptable for a baseline):** R4's `Crossing` only catches *collinear overlap* of different-decl wires; genuine non-perpendicular crossings can't occur with orthogonal polylines, so there is nothing else to catch at this stage. Documented in the spec (R4 is structurally implied by R1+R3 for orthogonal paths).
