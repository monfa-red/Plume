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
| `mod.rs` | orchestration: seed → `select` → `ports` → `route` → `nudge` → `Vec<RoutedWire>` |
| `select.rs` | **side selection** — the turn-aware monotone hill-climb (which side each end leaves) |
| `score.rs` | the scorecard tuple the search minimises (validator counts + turns + length) |
| `oracle.rs` | `clearance(node)` / `separation(w1,w2)` — the only place distances are computed |
| `scene.rs` | per-wire obstacle set + a scene index (passable ancestors, solid non-endpoint groups, text ignored) |
| `graph.rs` | orthogonal visibility graph (candidate lines at obstacle edges ± clearance and at ports) |
| `route.rs` | per-wire A\* over the graph, cost per WIRING section B |
| `ports.rs` | uniform slot assignment + ordering + the C3 slide, **given** the chosen sides (WIRING C2–C5) |
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
- ~~`route.rs`: add the crossing penalty (B3).~~ **Moved to Phase 4.** A greedy
  per-wire penalty (committed wires charge later ones) was tried and removed: at
  any weight that dodges a crossing it either shuffles the crossing onto a
  not-yet-routed wire (total crossings *rose* on the suite) or, when strong
  enough to force a multi-bend detour, folds a wire onto itself (A5). Crossing
  minimisation is global; it belongs with the nudge pass, which can reroute
  without breaking A5.

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
  within the tidiness tolerance (B6). Bundles (E1) as parallel rails. **Done** —
  structure-preserving track assignment, validate-and-keep per cluster, band
  sweep widest-first.
- ~~Crossing penalty (B3)~~ — **deferred to Phase 5.** B3 is a *global* minimiser
  and rides on the same channel/ordering machinery as fan trunks; cleaner to land
  it there than to bolt it onto the separation pass.

**Acceptance.**
- B2 wire-vs-wire clean on `wires_dense`, `wires_realistic` (validator).
- No "almost aligned" jitter (PNG check).

---

### Phase 5 — Special cases + last-resort transparency

**Goal.** The remaining contract.

- **B3 crossing penalty (moved from Phase 4):** minimise crossings globally in the
  nudge/ordering pass — order channels and bundle rails so they don't cross each
  other, push unavoidable crossings to channel ends. Never per-wire greedy (raises
  crossings / self-crosses — proven twice). Reduces `wires_realistic`'s X:8,
  especially the avoidable `water→roof`-vs-itself rail crossings.
- Fan groups share a trunk, exempt from B2 where they coincide (E2): one slot at
  the shared end, drawn once, the duplicate label deduped, and the validator's A3
  check exempts fan-sibling coincident runs.
- Self-loops: orthogonal loop out one side, back to an adjacent side (E3).
- Side overflow: even compaction + flag (C5).
- Last-resort relaxations of B1/B2 emitted as diagnostics (WIRING section B /
  `--strict`).

**Acceptance.**
- `wires_fan`, `internal_wires`, and a self-loop sample (add a small `a -> a`
  one) validate clean.
- `full_example` and `wires_realistic` are visually clean — no piercing, no
  canvas detours, no bunching; crossings minimised (no bundle crossing itself).
- The whole sample suite: zero invariant violations; B-relaxations only where
  WIRING permits, and each one flagged.

---

## Status

- [x] **Phase 0** — pipeline + invariant validator (A1/A2/A5 gated; A3 reported, gated from the multi-wire phases; A4 deferred to Phase 1)
- [x] **Phase 1** — oracle + obstacles + full validator (A1–A5 all checked, incl. A4; B1/B2 measured, B3 crossings counted; baseline report snapshotted)
- [x] **Phase 2** — visibility graph + A\* per wire (B1 + wire-node B2 clean on every sample; cost = bends-then-length; minimal geometry ports; `wires_realistic` clearance 10→6 to fit its gap:8)
- [x] **Phase 3** — multi-wire ports C1–C4 (least-loaded side, uniform centred slots, lone-wire bend-avoidance, crossing-free ordering). A3 61→5, B2w 40→15; A5/B1/B2n stay 0; even spacing verified on `wires_dense`/`wires_fan`. Grid gained a mid-channel turning line so slot-offset facing ports route cleanly (no self-cross). B3 crossing penalty moved to Phase 4 (greedy per-wire is counter-productive); residual A3/B2w are co-linear runs for the nudge pass. Fan-trunk consolidation (E2, incl. the duplicate-label) stays Phase 5; siblings spread as bundle members for now.
- [x] **Phase 4** — nudge / separate (`nudge.rs`): structure-preserving track assignment — interior segments slide onto tracks `separation` apart, ports pinned, every vertex rebuilt as its two segments' intersection; each move committed only if it stays node-safe and orthogonal (so a boxed channel compacts or is left, never pierced). Per cluster it sweeps the feasible band, widest separation first, nearest the original. Result: **A3 → 0 and B2w → 0 on every sample except `wires_labels`** (5 wires on one tiny edge — genuine C5 overflow, flagged). `wires_realistic` loosened (gap/sizes) so its bundles fit. Crossing-penalty (B3) still open. Invariants A1–A5/B1/B2n stay 0; byte-identical.
- [x] **Phase 5** — special cases + transparency. **Self-loops (E3):** `route::self_loop` wraps a corner out one side back to the adjacent one (default right→top, forced sides honoured); new `wires_selfloop` sample validates clean. **Fan trunks (E2):** the duplicate fan label is deduped at resolve (only the first expanded sibling keeps it); fan hubs get a trunk id (`mod::fan_ids`) so siblings on a shared side collapse to one slot (`ports::assign_slots` occupant grouping) and the validator exempts their coincident trunk from A3/B2 (`validate::fan_siblings`). A fan whose targets straddle the hub still splits across sides — "one slot" yields to "looks clean" (WIRING's own meta-rule); noted below. **B3 crossings (global):** the nudge now tries each cluster's track orderings (sorted first) and commits the node-safe placement with the fewest crossings among the affected wires, so a bundle no longer crosses itself — `wires_realistic` X:8→6 (the rest topological, allowed). **C5:** `wires_labels` is the even-compacted, flagged overflow (spacing 4.4, uniform). **Transparency:** `plume::routing_diagnostics` surfaces B1/B2 relaxations as warnings (never silent); the CLI prints them and `--strict` fails on them. Invariants A1–A5/B1/B2n stay 0 suite-wide; byte-identical; clippy/fmt clean.

When all six are checked and the full suite validates clean, the router meets
WIRING.md. Reconcile any wording drift back into SPEC.md / WIRING.md and update
this Status.

**Known nuance (E2 vs C1/“looks clean”).** WIRING C2 says a fan group's shared end
is "one slot." We honour that when the siblings land on the same side; when a
fan-out's targets straddle the hub (some up, some down) the siblings keep their
geometrically-best sides rather than forcing one slot (which would force ugly
detours). This follows WIRING's meta-rule ("perfect" = obeys every rule *and looks
clean*) over the literal "one slot." If strict one-slot-always is wanted, C1 would
need a fan-hub side election — deferred as it degrades the common straddling case.

---

## Post-Phase-5 hardening — endpoint-clearance blind spot

Review of `wires_realistic` surfaced a wire running parallel ~4px under `roof`'s
own bottom edge — a clearance breach the validator passed because a wire's
endpoints were excluded from its obstacle set entirely (the one
`scene::obstacles_for` feeds the validator, the router grid, and the nudge — so all
three were blind). WIRING said an endpoint is "passable"; that's now scoped to the
wire's perpendicular **attaching stub** only (every other segment keeps `clearance`
from its own endpoints; ancestors stay fully passable so a wire can still exit its
container).

- **Validator** (`validate::check_endpoint_clearance`): non-stub segments are now
  measured against the wire's own endpoint rects → the skim is a flagged B2.
- **Router** (`route::route`): two-tier — route from the ports first and keep it if
  it already clears its endpoints; else route the interior between two approach
  points (`clearance` out) with the endpoints as obstacles and re-add the stubs;
  else (a node within `clearance` of the port makes it geometrically impossible)
  keep the relaxed route with a flagged skim — never the node-piercing dumb route.
- **Nudge** (`nudge::is_safe`): endpoint clearance checked *relatively* — a move may
  preserve an unavoidable skim but must never deepen one or create a new one.
- **Gate:** `tests/wiring.rs::no_sample_breaks_a_hard_guarantee` pins A1–A5/B1/A3 = 0
  on every sample; the scorecard snapshot pins the B2 counts.

Result: `wires_realistic` bird→roof is clean (B2n:0); the systemic skim is closed.

## Turn-aware global side-search — CURRENT (`select.rs`)

Side selection was the deeper root of every remaining issue (off-centre ports,
extra turns, the scene-direction bias, the column-layout tangle). The old approach —
geometric facing pick, then a chain of patches (least-loaded, skim-`reside`,
all-or-nothing convergence-unify) — was replaced by **one monotone hill-climb**
([design](docs/superpowers/specs/2026-05-29-turn-aware-side-search-design.md)).

`select::search` seeds with the geometric facing pick and repeatedly proposes a
move — flip a single wire-end, or move a whole node group / fan trunk as a unit —
onto a candidate side (the facing side + its two perpendiculars; forced `.side`
pinned). It keeps any move that **strictly** lowers `score`'s tuple
`(invariants, B1, B2n, B2w, crossings, turns, length)`, evaluated by a fast
route-only **proxy**. The same lead hints feed the proxy and the final build, the
winner is built + nudged, and kept only if it betters the seed baseline — so the
result is **monotone** (never worse than geometric), **can't oscillate** (strict
improvement only), and is a **deterministic function of input**. Convergence
nesting, partitioning, turn-minimisation, and perpendicular-side exits all *emerge*
from this one mechanism; `ports` lost side *choice* (`plan` → `assign`, taking sides
as input) and `mod` became orchestration-only.

**Result vs the old heuristics:**

- `wires_realistic_column` (the column "disaster"): **A3 1→0, B2w 8→1, X 10→5** — the
  shared-run overlaps are gone; it passes every hard guarantee.
- `wires_chain` X 1→0; `wires_realistic_row` clean (`cat→bowl` to the midpoint,
  `bird/water→roof` minimal turns, X:0); `wires_fan` clean (fan-trunk re-election).
- New gated scenes `wires_star` / `wires_grid` / `wires_nested` all validate 0/0/0/0.
- Only residual: `mermaid_fast` B2n:1 — a 4px **flagged** skim on a tight fan-in,
  where keeping the fan unified (E2) beats the old split-the-fan trick.

All hard guarantees + B2n 0 suite-wide (bar that one flagged skim); byte-identical
compiles; clippy/fmt clean. Tests: `select`/`score` unit tests; the slot/C3 tests in
`ports`; `realistic_convergence_crossings_are_minimised`; the hard-guarantee gate +
scorecard over the broadened sample suite.

**Locked:** side selection is a monotone search over a turn-aware objective — never a
per-wire greedy A\* penalty (raises crossings / self-crosses, proven), never an
oscillating re-route loop (the strict-improvement guard is what makes it safe).

### Follow-up routing fixes (same contract, sharper geometry)

- **Even-spacing overflow (C2/C5):** `ports::assign_slots` spacing was
  `(span-2·sep)/(k-1)` — the corner inset stayed at `clearance` while the *spacing*
  collapsed, so cranking `clearance` bunched wires to a point. Now
  `spacing = sep.min(span/(k+1))`: under overflow the inset shrinks in lockstep with
  the spacing (both → `span/(k+1)`), so wires split the side evenly and never reach
  the corners. WIRING C2/C5/R13 updated to match.
- **C3 straight-shot guard:** the single-wire port slide fired on any facing side
  pair, so `dog.b -> bird.t` (opposite sides, boxes side-by-side) slid both ports to
  the corners chasing an impossible straight. Now it slides only when the boxes
  overlap on the slid axis (a real straight shot); else the port stays centred.
- **Dot marker (render):** the dot was centred on the endpoint, so it poked past the
  shape edge while the line stopped short (a gap). `markers::dot_center` now seats it
  tangent to the edge on the wire side, and `markers::line_inset` stops the line at
  the dot's back edge — no overshoot, no gap. Pointed markers keep the 4 px stub.

**Residual / possible next steps (none blocking):**
- `wires_labels` B2w:9 is the only flagged relaxation left — a genuine C5 overflow
  (5 wires on one tiny shared edge). Density is the user's lever; not a bug.
- `wires_chain` X:1 is topological (two under-row wires must cross). Accepted by B3.
- The convergence keep-better compares **pre-nudge** candidates via the full
  validator after nudge (`finish` nudges, then `quality` validates the nudged
  result), so it already reflects the final geometry.

**Do NOT (locked):** add a per-wire greedy crossing penalty (fails — see below), or
an iterative re-elect/re-route loop (can oscillate). Convergence stays a fixed set
of candidates + the deterministic, monotone keep-better compare.

### Locked lessons (don't relearn the hard way)

- **Crossing/separation minimisation is global** — it belongs in the ports
  ordering + nudge, never a single-wire A\* penalty. A greedy per-wire penalty was
  tried 3× and always shuffled the problem onto a later wire or forced a self-cross
  (A5).
- **The two-pass key must come from the *real* obstacle-aware route**, not a blind
  `dumb_route` probe — a blind probe gives the same wrong order as straight-line aim
  (it can't see the detour). (Confirmed by adversarial review.)
- **Endpoint clearance** is shared by three consumers via `scene::obstacles_for`
  (validator, router grid, nudge) — any change must keep all three consistent.
- Reserved words can't be node ids in test `.plume` scenes (`b t l r mid` …); use
  `aa bb`, `src via dst`. Accept snapshots with `INSTA_UPDATE=always cargo test`
  then delete `*.snap.new`. Render-check with `resvg` and actually read the PNG.
