# Plume Wiring — Implementation Plan

How we build the wire router defined by [`WIRING.md`](WIRING.md), in steps that
survive across sessions without the code drifting or degrading.

---

## How to use this file (read every session)

The work is split into **phases**. Each phase has an executable **acceptance
gate** — when its tests pass, the phase is done, full stop. That gate is the
thing that keeps quality constant no matter who (or which session) writes the
code.

**A fresh session re-orients in five minutes:**

1. Read [`SPEC.md`](SPEC.md) (the language) and [`WIRING.md`](WIRING.md) (the
   routing contract — the *what*).
2. Read this file: find the first unchecked phase in [Status](#status).
3. Run `cargo test`. Green = the previous phase landed; the failing/absent tests
   are your target.
4. Work that **one** phase. Don't start a phase you can't finish, verify, and
   commit in this session.
5. End green: acceptance tests pass, a PNG visual check looks right, snapshots
   reviewed, **commit**, tick the box here.

**The validator is ground truth.** "Correct routing" = the validator (see
Phase 1) reports no invariant violations and only the expected, flagged
relaxations. Code may be written ten ways; it must pass the same validator and
the same snapshots. This is why sessions converge instead of drift.

**Altitude.** This plan fixes the *what* (acceptance) and the *architecture*
(module layout, types). It deliberately does **not** script every line — the
implementing agent breaks each phase into ~5–10 micro-steps (write → `cargo
test` → eyeball a PNG → repeat), and makes the real algorithmic calls, because
those only become clear while hands are in the code. When a phase's note and
reality disagree, reality wins — as long as the acceptance gate still passes.

**Standing rules** (hold in every phase):

- **Determinism** — same diagram → byte-identical SVG. Add a compile-twice test
  in Phase 0 and never let it go red. No `HashMap` iteration order in routing
  decisions; use `BTreeMap`/sorted vecs.
- **Port, don't invent** — follow the libavoid model in WIRING's appendix
  (visibility graph → A\* → nudge). Re-deriving a proven algorithm is far more
  stable across sessions than improvising one. This is also why the last attempt
  thrashed: it improvised, against a contract that was still moving.
- **CLAUDE.md** — no `unsafe`; one concept per file; split at ~500 LOC; `insta`
  snapshots; verify SVG by rendering to PNG (`resvg`) and looking.

---

## Architecture (locked)

The router slots back into the existing pipeline as the **route** phase, between
layout and render:

```
resolve (ResolvedWire — exists) → layout (PlacedNode tree — exists)
   → route (NEW: ResolvedWire + placed nodes → RoutedWire polylines)
   → render (fill the <g class="plume-wires"> group)
```

**Module layout** — `src/layout/wires/`, one concern per file:

| File | Concern |
|---|---|
| `mod.rs` | orchestration: route every wire in deterministic order → `Vec<RoutedWire>` |
| `oracle.rs` | `clearance(node)` / `separation(w1,w2)` — the only place distances are computed |
| `scene.rs` | per-wire obstacle set + a scene index (passable ancestors, solid non-endpoint groups, text ignored) |
| `graph.rs` | orthogonal visibility graph (candidate lines at obstacle edges ± clearance and at ports) |
| `route.rs` | per-wire A\* over the graph, cost per WIRING section B |
| `ports.rs` | side selection + uniform slot assignment + ordering (WIRING section C) |
| `nudge.rs` | channels / track assignment / separation / snap-to-shared-lane (WIRING B6, E1) |
| `validate.rs` | the contract checker (WIRING sections A–B) — independent of the router |
| `geometry.rs` | shared point/segment math (intersection, perpendicular distance, …) |
| `text.rs` | wire-label placement along the final polyline |

**Key type** (re-introduced in `src/layout/ir.rs`):

```rust
pub struct RoutedWire {
    pub path: Vec<(f64, f64)>,   // orthogonal polyline, scene coords
    pub markers: Markers,
    pub attrs: AttrMap,
    pub texts: Vec<RoutedText>,  // labels placed along the path
    pub data_from: String,       // for data-from / data-to + a11y
    pub data_to: String,
    // provenance for the validator: which shapes this segment may touch,
    // and the declaration it came from (fan siblings share it).
    pub seg_from: String,
    pub seg_to: String,
    pub decl_span: Span,
}
```

`LaidOut` regains `wires: Vec<RoutedWire>`; `render` emits them into the
`plume-wires` group (it currently emits the empty placeholder).

`plume::validate_str(src) -> Vec<Violation>` returns to the public API, gating
the rebuild.

---

## How to execute one phase

1. Re-read the phase's WIRING rules and acceptance gate.
2. Break it into 5–10 micro-steps. Each micro-step: smallest change that moves a
   test; run `cargo test`; for anything visual, `cargo run -- samples/X.plume
   --bake-vars -o /tmp/x.svg && resvg /tmp/x.svg /tmp/x.png` and look.
3. Loop micro-steps until the **acceptance gate** is fully green.
4. Run `cargo clippy --all-targets -- -D warnings` and `cargo fmt`.
5. Review snapshot diffs (`cargo insta` or the `.snap.new` files) — they must
   change only in ways you expect.
6. Commit (one phase = one commit). Tick [Status](#status).

---

## Phases

### Phase 0 — Pipeline + validator skeleton

**Goal.** Wires draw *something* again, and the contract starts being checked.

- Re-add `RoutedWire` and the **route** phase with a deliberately dumb router:
  the simplest *perpendicular-attached* orthogonal route per wire — straight when
  the ports line up, an L when the chosen edges are perpendicular, otherwise a Z
  (two bends) — ignoring obstacles. Pick the facing edge by geometry, attach at
  its centre.
- Render the polyline into the `plume-wires` group, with markers (reuse
  `render::markers`) and labels at `mid`.
- Implement `validate.rs` for the per-wire invariants the dumb router can
  guarantee — **A1** (orthogonal), **A2** (perpendicular attachment), **A5** (no
  self-cross) — as gating checks, plus **A3** (perpendicular crossings) as a
  *reported, non-gating* check. **A4** (sides-only / never to text) needs the
  text-aware scene model from Phase 1, so its check lands there. Re-expose
  `plume::validate_str`.
- Add the **determinism** test (compile twice → identical bytes).

**Builds on.** Existing resolve/layout/render.

**Acceptance.**
- Every sample compiles and the dumb router's wires render (PNG check on
  `wires_basic`, `wires_chain`).
- `validate_str` reports **zero A1/A2/A5 violations** for the dumb router (its
  straight/L/Z routes are orthogonal, perpendicular-attached, and non-self-
  crossing by construction). **A3** is reported but *not* gated — a per-wire
  router that ignores other wires inevitably shares parallel runs; those resolve
  in the multi-wire phases (3–4). **A4** arrives with Phase 1.
- Determinism test passes.
- Snapshots regenerated and reviewed.

---

### Phase 1 — Oracle + obstacle model + full validator

**Goal.** The complete contract checker, and the distance/obstacle truth the
router will share.

- `oracle.rs`: `clearance` / `separation` per WIRING definitions.
- `scene.rs`: per-wire obstacle set — endpoints and their ancestor containers
  are passable; other non-endpoint shapes (incl. groups, as solid bboxes) are
  obstacles; text nodes ignored.
- Extend `validate.rs` to the **constraints** (WIRING B1 overlap, B2 clearance /
  separation) and report crossings (B3) — using the oracle.

**Acceptance.**
- Validator runs over every sample and prints a **baseline report**: the dumb
  router's B-violations (it will pierce shapes and ignore clearance — that's
  expected and now *measured*). Snapshot the report.
- Oracle + obstacle set unit-tested on a hand-built case.

---

### Phase 2 — Visibility graph + A\* (single wire)

**Goal.** Each wire, routed alone, respects clearance with the fewest bends.

- `graph.rs`: orthogonal visibility graph.
- `route.rs`: A\* per wire, cost = bends then length (WIRING B4/B5), obstacle
  clearance hard (B1/B2). One wire at a time; ignore other wires for now.
- `ports.rs` (minimal): pick the side by geometry (forced `.side` honoured),
  attach at the centre.

**Acceptance.**
- For every sample, **B1 (no overlap) and the wire↔node half of B2 (clearance)
  are clean** per wire (validator; wire-vs-wire separation arrives in Phases 3–4).
- Routes are visibly sane on `wires_basic`, `wires_realistic` (PNG): no
  shape-piercing, no canvas-wide detours.
- Determinism holds.

---

### Phase 3 — Multi-wire: ports, ordering, crossings

**Goal.** Many wires coexist cleanly.

- `ports.rs` full: side selection with least-loaded tie-break (C1), uniform slot
  assignment (C2), single-wire bend-avoidance (C3), ordering to minimise
  crossings (C4).
- `route.rs`: add the crossing penalty (B3 — a crossing ≈ a few bends, tunable
  constant) so committed wires influence later ones.

**Acceptance.**
- Wire crossings minimised; any that remain are perpendicular (validator A3).
- Even spacing on shared sides (C2) — visually verified on `wires_dense`,
  `wires_fan`.
- Byte-identical across two runs.

---

### Phase 4 — Nudge / separate

**Goal.** Parallel runs become neat rails; near-parallels snap together.

- `nudge.rs`: group co-linear segments into channels, assign tracks so parallels
  sit exactly `separation` apart (B2), snap near-parallels onto a shared lane
  within the tidiness tolerance (B6). Bundles (E1) as parallel rails.

**Acceptance.**
- B2 wire-vs-wire clean on `wires_dense`, `wires_realistic` (validator).
- No "almost aligned" jitter (PNG check).

---

### Phase 5 — Special cases + last-resort transparency

**Goal.** The remaining contract.

- Fan groups share a trunk, exempt from B2 where they coincide (E2).
- Self-loops: orthogonal loop out one side, back to an adjacent side (E3).
- Side overflow: even compaction + flag (C5).
- Last-resort relaxations of B1/B2 emitted as diagnostics (WIRING section B /
  `--strict`).

**Acceptance.**
- `wires_fan`, `internal_wires`, and a self-loop sample (add a small `a -> a`
  one) validate clean.
- `full_example` and `wires_realistic` are visually clean — no piercing, no
  canvas detours, no bunching.
- The whole sample suite: zero invariant violations; B-relaxations only where
  WIRING permits, and each one flagged.

---

## Status

- [x] **Phase 0** — pipeline + invariant validator (A1/A2/A5 gated; A3 reported, gated from the multi-wire phases; A4 deferred to Phase 1)
- [x] **Phase 1** — oracle + obstacles + full validator (A1–A5 all checked, incl. A4; B1/B2 measured, B3 crossings counted; baseline report snapshotted)
- [ ] **Phase 2** — visibility graph + A\* (single wire)
- [ ] **Phase 3** — multi-wire: ports, ordering, crossings
- [ ] **Phase 4** — nudge / separate
- [ ] **Phase 5** — special cases + transparency

When all six are checked and the full suite validates clean, the router meets
WIRING.md. Reconcile any wording drift back into SPEC.md / WIRING.md and update
this Status.
