# Wire Routing — Rules & Architecture

**Status:** design (rules-spec). Supersedes `WIRE_ROUTING_PLAN.md`.
**Date:** 2026-05-28.
**Approach:** validator-first rebuild (write the rules as a checker, then rebuild the core to pass it).

---

## 0. Why this exists

The current router (`src/layout/wires/`, ~2,700 LOC) is a **topology-template
engine** (straight / Z / L / U / detour per edge-pair) with per-bundle
heuristics layered on top. Its structural problems:

| # | Problem | Where |
|---|---------|-------|
| 1 | Clearance rules duplicated in ~5 places, each subtly different | `scene::obstacles_for`, `route::segments_clear_strict`, `route::path_is_clear`, `mod::simulate_path_score` (+ `bends_outside_endpoint_bboxes`, `path_entry_matches_edges`), `lanes` halo math |
| 2 | No general wire-vs-wire model | `route(_prior_paths …)` — the argument is ignored; crowding handled only inside one bundle via `redistribute_channels` |
| 3 | Bends chosen locally & greedily, no shared coordinate space | `pick_clear_column` / `pick_trunk`, recomputed per query |
| 4 | Combinatorial special-casing (H/V × Z/detour × src/tgt × facing/perp/same-dir × fan/bundle) | `z_trunk_*` / `detour_b2_*` ×4, `relieve_overloaded_bins`, `align_free_bundles` |
| 5 | No checkable contract — correctness can only be eyeballed | nothing validates the output |

Observed failures (rendered `samples/full_example`, `samples/wires_realistic`):
wires **pierce shapes**, **detour around the whole canvas**, and run
**parallel closer than the gap**.

This document defines a **predictable rule set** (a contract the output must
satisfy), a **validator** that checks it mechanically, and the **target
architecture** that satisfies it. The validator is built first — it is the
tool that lets us criticise the router algorithmically instead of by eye, and
it audits both the old engine (to quantify the baseline) and the new one.

---

## 1. Goals (the rules, in the user's words → formalized)

1. **Shortest, cleanest** orthogonal route.
2. Keep **`gap`** from every other wire; keep **`clearance`** from every other shape.
3. **Never run parallel-within-gap to, or overlap, another wire — unless it is
   the only option.** Then take the longer route: leave from a different edge,
   go around, etc.
4. **Approach shapes perpendicular**; **cross other wires only perpendicular**.
5. Always produce *a* route, even at tight gaps (uglier is acceptable). The
   user adjusts the gap or moves shapes until happy.
6. **Deterministic**: the same layout always produces the same routing.

---

## 2. Definitions

| Term | Definition |
|------|------------|
| **Endpoint shape** | A wire's source or target node, plus any ancestor container of it. **Passable** — the wire may cross its boundary to reach the endpoint. |
| **Obstacle** | A shape that is not an endpoint shape. The **outermost** non-passable shape is the obstacle: passable groups (an endpoint's ancestors) are entered, so their non-endpoint children become obstacles; a non-passable group counts as one obstacle and its descendants are not counted separately. |
| **clearance(S)** | Wire-to-shape minimum distance for obstacle `S` = the `gap` of `S`'s **parent container** (the scene's gap for a top-level shape). One number per shape, set directly by the user. |
| **gap(W)** | Wire-to-wire minimum distance carried by wire `W` = `W`'s `gap` attr (default 16). |
| **separation(W1, W2)** | Required distance between two wires = `max(gap(W1), gap(W2))`. |
| **bundle** | Wires sharing `(src, src_edge, tgt, tgt_edge)` — rendered as parallel rails exactly `separation` apart. |
| **fan-out group** | Wires materialised from one declaration (`a -> b & c`). They **share** the exit trunk and are exempt from R3 where they coincide. |

> **Consequences of "clearance = parent gap, exactly":** a small group gap lets
> wires pass closer to those shapes than they pass to each other; a large group
> gap makes wires detour wide. Both are intended and user-controllable.

---

## 3. The Contract

The validator checks exactly these. Nothing else is "correct" or "incorrect".

| Rule | Statement | Severity |
|------|-----------|----------|
| **R1 Orthogonality** | Every segment is axis-aligned; consecutive segments meet at exactly 90°; no collinear or zero-length joints (a redundant vertex is itself a defect). | invariant |
| **R2 Shape clearance** | For every segment and every obstacle `S`, perpendicular distance ≥ `clearance(S)`. Endpoint shapes and their ancestors are exempt. | error |
| **R3 Wire spacing** | For any two segments of different wires that are parallel and overlap along their shared axis, perpendicular distance ≥ `separation`. Exempt: collinear/coincident segments inside one fan-out group. | warning |
| **R4 Perpendicular crossings** | Two wire segments may intersect only if perpendicular, and only at a single point — never a shared parallel run. | invariant |
| **R5 Attachment** | The segment touching a shape edge is perpendicular to that edge, and its endpoint lies on the edge (within the corner inset). | invariant |
| **R6 Last-resort transparency** | Any relaxation of R3 (or, in a fully-enclosed worst case, R2) must be emitted as a `Violation`/diagnostic — never silent. | meta |

**Severity meanings:**
- *invariant* (R1, R4, R5) — must hold for every wire, always. A violation is a bug.
- *error* (R2) — route-anyway-and-warn only when a shape encloses an endpoint with no exit.
- *warning* (R3) — permitted strictly as a last resort per R6, and always reported.

---

## 4. Clearance oracle (single authority)

One module. The **only** code allowed to compute a clearance distance:

```
clearance(shape)      -> f64     // parent container's gap (scene gap if top-level)
separation(w1, w2)    -> f64     // max(gap(w1), gap(w2))
```

Every phase — graph construction, routing, validation — calls these. No other
function inflates a bbox or invents a halo. This kills problem #1 outright.

---

## 5. Validator (Step 1 deliverable)

- **Input:** the routed wires (polylines) + the scene index.
- **Output:** `Vec<Violation { rule: RuleId, wires: Vec<WireId>, at: Locus, detail: String }>`.
- **Uses:**
  1. A test over all `samples/` asserting **zero invariant violations** and
     snapshotting the warning list (so R3 last-resorts are tracked, not hidden).
  2. Optionally surfaced as compile diagnostics for R6 (ties into `--strict`).
- **Independence:** the validator shares **no decision code** with the router —
  it re-derives every check from geometry, reusing only the scene index and the
  §4 oracle. That is what lets it audit the *current* engine first (baseline
  report) and then gate the rebuild.
- **Tolerance:** all distance/angle comparisons use a small epsilon, so rails
  sitting *exactly* at `separation` (or bends at exactly 90°) never false-positive.

This is the "critique algorithmically" tool the whole effort hinges on.

---

## 6. Priority / cost model

How "shortest + clean + no-overlap" is ordered when choosing among candidate
routes. **Lexicographic**, highest first:

1. *(hard)* R1, R5 — orthogonal, perpendicular attachment. Never violated.
2. *(hard)* R2 — shape clearance. Obstacle clearance zones are non-traversable.
3. R3 — wire spacing. Relax **only** when no R2-respecting route exists.
4. Fewest bends.
5. Shortest length.
6. Tidiness — align bends onto shared channel midlines / existing wire tracks.

Realised as A\* edge costs: `step = length`; `+ bend`; `+ large` for entering
another wire's gap zone running parallel; `+ small` for a perpendicular
crossing. **Edge selection** (which side a wire leaves a shape from) falls out
of the search — there is no separate "relieve overloaded bins" pass.

This refines `SPEC.md` §10's fallback hierarchy (respect gaps → cross wires →
cross shapes → straight) and tightens "cross wires" to **perpendicular only**.

---

## 7. Target architecture (Step 2 — sketched; detailed after the baseline report)

Five modules, each one concern, each independently testable:

| Module | Responsibility | Guarantees |
|--------|----------------|------------|
| **oracle** | `clearance` / `separation` (Step 1). | single source of distance truth |
| **graph** | Orthogonal visibility graph: candidate lines at every obstacle edge ± `clearance`, at the world frame, and through every endpoint; nodes at intersections; edges = clear orthogonal segments. Deterministic. | bends only at meaningful coords; endpoints inserted as nodes → **pixel-perfect attachment** (preserved from today); shared coordinate space → cross-wire alignment for free (fixes #3) |
| **route** | Per-wire A\* on the graph with the §6 cost. Wires routed in a deterministic order; each committed wire's segments are added as soft costs (parallel = expensive, perpendicular crossing = cheap). | R1, R4, R5 by construction; R2 hard; wire-vs-wire is now a first-class part of the search (fixes #2) |
| **separate / nudge** | Group co-linear segments into channels; assign tracks (left-edge sweep) so parallels sit exactly `separation` apart; nudge bends onto shared midlines. | R3 by construction; neat parallel rails; replaces the stamp/redistribute machinery (fixes #4) |
| **validator** | The §5 checker. Runs in tests; gates the rebuild. | the contract is enforced, not hoped-for |

**Determinism (goal #6).** Routing order is fixed: wires in source-declaration
order, ties broken geometrically. No routing decision may depend on `HashMap`
iteration order — ordered maps (`BTreeMap`) or explicit sorts only. (The current
engine bins endpoints/bundles with `HashMap`, which can make output vary
run-to-run; the rebuild must not.)

**Retired:** `stamping::stamp_sibling`/`shift_polyline`, `lanes::redistribute_channels`
+ `z_trunk_*`/`detour_b2_*` bounds, `mod::relieve_overloaded_bins`,
`endpoints::align_free_bundles`, and the 5 duplicated clearance checks.

**Kept / adapted:** `scene.rs` (index + endpoint-passable logic, repointed at
the oracle), `planning.rs` (chain/fan explosion → segment list),
`channels.rs` interval math (reused by graph construction), `geometry.rs`,
`text.rs` (label placement along the final polyline).

---

## 8. Phasing & acceptance

**Step 1 — oracle + validator.** No routing behaviour change.
*Accept:* validator compiles, runs over every sample, prints a baseline
violation report; zero false positives on hand-verified clean wires.

**Step 2 — rebuild the core to pass the contract.**
*Accept:*
- Zero invariant violations (R1, R4, R5) on every sample.
- R2 clean except genuinely-enclosed endpoints.
- R3 warnings only where a clearance-respecting detour is truly impossible.
- `wires_realistic` and `full_example` visually clean: no piercing, no
  canvas-wide detours, no parallel bunching.
- Determinism test: route twice → byte-identical polylines.
- All existing snapshot tests reviewed and re-accepted.

---

## 9. Deferred / open

- **Self-loops** (`a -> a`, SPEC §10) — currently error; route in a follow-up.
- **Rounded wire corners**, **manual waypoints** — SPEC non-goals; unaffected.
- **Large parent-gap → wide detours** — accepted per the clearance decision;
  revisit only if it proves annoying in practice.
- **Wire labels vs R3** — labels are not wires; R3 ignores them. The existing
  label halo (clipping the wire under the label) is a render concern, untouched.
- **Endpoint lanes vs the shared graph** — how per-edge lane offsets (many wires
  on one edge) interact with a once-built graph is a **Step-2 design question**
  to settle with the baseline report in hand: likely resolve edges → allocate
  lanes → route trunks → track-assign. Flagged so §7 isn't mistaken for a
  finished design.
- **SPEC §10 alignment** — §10's clearance wording is updated to the *parent
  gap* model (it previously said the wire's gap) and its "crosses other wires"
  tier tightened to perpendicular-only. Self-loops stay documented in §10 as
  intended behaviour but remain deferred in the implementation.
