# Wire Routing — Step 2: Visibility-Grid + A* Router — Implementation Plan

> ## ▶ RESUME HERE (fresh session, 2026-05-28)
> **Tasks 2.1–2.4 are DONE and merged to `main`.** The new grid+A* router is
> live: invariants (R1/R4/R5) = 0, no shape piercing, no canvas-wide detours,
> deterministic. Files: `src/layout/wires/{grid,astar,route_graph,oracle,validate}.rs`.
> Validator gate: `cargo test --test wire_rules` (baseline R2=17, R3=34, R4=5).
>
> **The ONLY remaining task is 2.5: wire-wire separation** (parallel wires
> between the same pair stack instead of fanning into lanes — cosmetic; nothing
> pierces or mis-routes). See **Task 2.5 (revised)** below. **Three shortcuts
> were tried and reverted** (greedy A* penalty weak+strong; blind per-bundle
> sibling-shift) — read their findings there before coding. The correct fix is
> **global, obstacle-aware track-assignment**; it's the genuinely hard part of
> orthogonal routing, a focused several-hundred-LOC pass, not a tweak.
> First step when resuming: re-render `wires_realistic`/`full_example`, re-read
> Task 2.5, then implement track-assignment validator-gated.

> **For agentic workers:** execute task-by-task with TDD; the **validator** (`plume::validate_str` / `tests/wire_rules.rs`) is the gate — re-run it after every task and watch the baseline snapshot shrink. Keep every commit green.

**Goal:** Replace the topology-template router (`route.rs`/`lanes.rs`/`stamping.rs` + the edge-relief machinery in `mod.rs`) with an orthogonal **visibility-grid + A\*** router whose bends fall on shape-derived lines and whose discrete tracks give wire separation by construction — driving the validator baseline to **zero invariant + R2** violations and **R3 only when genuinely forced**.

**Reference:** rules-spec `docs/superpowers/specs/2026-05-28-wire-routing-rules-design.md` (contract R1–R6, clearance oracle). Step 1 (oracle + validator) is committed.

---

## Architecture decisions (refining spec §7 / resolving §9)

1. **Grid, not continuous nudging.** Candidate X/Y lines come from: each obstacle's clearance-inflated edges (`x = O.x − clearance(O)`, `x = O.right + clearance(O)`, and the Y analogues), the world frame, the channel **midlines** between adjacent shape edges (tidy bends), and each wire's endpoint attachment coordinates. A* travels these lines; bends land only on meaningful coordinates.

2. **§9 ordering resolved** — strict phase order, no chicken-and-egg:
   `edge-selection (geometry / forced .side)` → `per-edge lane allocation` (attachment points, separation-spaced, centred) → `build grid` (including attachment coords) → `ordered A* per wire`. Attachment points are grid nodes, so attachment is pixel-perfect and perpendicular by construction (R5).

3. **Wire–wire separation via ordered A\* + soft cost** (not a post-pass): route wires in source order; record each committed wire's segments; a later wire pays a **large** cost to run parallel within `separation` of an existing wire and a **small** cost to cross it perpendicularly. When the only path overlaps, it routes anyway → R3 warning (R6). Where a channel is wide enough, the cost gradient pushes wires onto distinct shape-derived/midline tracks ≥ `separation` apart.

4. **Determinism (spec §7):** wires routed in source-declaration order; A* ties broken by `(coordinate, direction)`; `BTreeMap`/sorted iteration only. A determinism test asserts route-twice → identical.

5. **Always-a-path:** the world frame is always in the grid, so the perimeter is always reachable; if even that fails (endpoint fully enclosed), fall back to a straight edge-to-edge segment and let the validator flag it.

---

## Module plan (new, under `src/layout/wires/`)

| File | Responsibility |
|------|----------------|
| `grid.rs` (create) | `Grid { xs: Vec<f64>, ys: Vec<f64> }`; build from obstacles (oracle clearance) + world + midlines + attachment coords; node index ↔ `(i,j)`; `edge_clear(a, b, obstacles)` test. |
| `astar.rs` (create) | A* over a `Grid` from a start node to a goal node; cost = length + bend penalty + a caller-supplied per-segment surcharge (used for wire–wire). Returns a coordinate polyline. |
| `attach.rs` (create) | Edge selection (geometry / forced) + per-edge lane allocation → each wire's `(src_point, src_edge, tgt_point, tgt_edge)`. Absorbs the useful parts of today's `planning.rs`/`endpoints.rs`. |
| `route_graph.rs` (create) | Orchestrator: build grid, route each wire (ordered A*, prior-segment surcharge), assemble `RoutedWire` (markers/texts/provenance as today). |
| `mod.rs` (modify) | `route_wires` calls `route_graph`; old modules removed at switchover. |

**Retired at switchover (Task 2.7):** `route.rs`, `lanes.rs`, `stamping.rs`, the `resolve_edges`/`relieve_overloaded_bins`/`pick_best_edges`/`simulate_path_score` machinery in `mod.rs`, and `endpoints.rs`'s align/compress passes. `channels.rs` interval math is reused by `grid.rs` or dropped. `scene.rs`, `geometry.rs`, `oracle.rs`, `validate.rs`, `text.rs` stay.

---

## Tasks (each: TDD, run validator, commit green)

### Task 2.1 — `grid.rs`: candidate coordinates + clear-edge test
- Build sorted, de-duplicated `xs`/`ys` from obstacle clearance edges + world + midlines.
- `edge_clear(a, b, obstacles)` — true iff the orthogonal segment `a→b` enters no obstacle's clearance zone (reuse `validate::segment_pierces_box` logic against oracle-inflated obstacles).
- **Tests:** a single obstacle splits the candidate lines as expected; a segment through the obstacle's zone is not clear; one beside it is.

### Task 2.2 — `astar.rs`: orthogonal A* with bend penalty
- Nodes = grid intersections; neighbours = nearest grid node in each of 4 directions whose connecting segment is `edge_clear`.
- Cost = Manhattan length + `BEND` per turn + `surcharge(seg)` (default 0). Heuristic = Manhattan distance (admissible).
- Deterministic tie-break by `(node index)`.
- **Tests:** straight shot when unobstructed; routes around a single box with exactly 2 bends; returns `None` only when truly disconnected.

### Task 2.3 — `attach.rs`: edge selection + lane allocation
- Edge selection: forced `.side` wins; else nearest edge by geometry (reuse `geometry::nearest_edge`).
- Per (shape, edge) bin: distribute the wires on it to lanes `separation` apart, centred, clamped to the edge (reuse the centring math from `endpoints.rs`, drop the align/compress heuristics).
- Output per wire: `src_point`, `tgt_point` (exact, on the edge), `src_edge`, `tgt_edge`.
- **Tests:** 3 wires on one edge land centred and `separation` apart; a forced side is honoured.

### Task 2.4 — `route_graph.rs`: wire it in (shapes only, no wire–wire yet)
- For each wire (source order): add its attachment points to the grid, A* from src to tgt (surcharge = 0), collapse collinear, build `RoutedWire`.
- Point `route_wires` at `route_graph`. Keep old modules compiling (not yet deleted).
- **Gate:** `cargo test`; run `tests/wire_rules` → **expect invariant + R2 violations to collapse** (orthogonal, perpendicular, shape-clearing). R3 may remain. Review the snapshot diff; accept. Visually check `wires_realistic` + `full_example` PNGs (no piercing, no canvas detours).

### Task 2.5 — wire–wire separation  ⚠ NEEDS TRACK-ASSIGNMENT (revised)
**Finding (2026-05-28):** a greedy per-wire A* `surcharge` (penalise running
parallel within `separation` of an already-routed wire), with or without
inserting `separation`-spaced channel tracks, **regressed** R3 (34 → 58) and
reintroduced an R1/R5. Reverted. Why it fails:
- The penalty just makes wire B *detour* around wire A — and the detour then
  runs parallel-close to wire C, so R4 overlaps convert into *more* R3
  parallels rather than disappearing.
- Much of R3 is **inherent**, not a router bug: `wires_chain` (9 wires) and
  `wires_labels` (5 wires) cram more wires onto a 40 px edge than fit at 16 px
  spacing — no router can make them ≥ `separation` apart. **First, split the
  baseline into inherent (bundle-overflow) vs fixable R3 and only target the
  fixable set** (and `log`/annotate the inherent ones as R6 "no room").

**Second attempt — per-bundle canonical + perpendicular sibling shift (also reverted, 2026-05-28):** route one canonical A* path per bundle, then offset each sibling by `shift_polyline` (flow-aware perpendicular). This *did* separate (R4 → 0, `wires_realistic` rendered as clean rails) but had two real flaws: (a) the blind offset **clips shape clearance** — siblings shifted into obstacle zones (R2 17 → 21, e.g. `water->roof` rails within 8 of garden); (b) bend re-intersection **drifts** ~0.3 px, breaking orthogonality/attachment (R5). These are the exact problems the *old* engine's runway/sibling-radius/clear-range machinery existed to patch — so this path leads straight back to that complexity. Conclusion: **separation must be obstacle-aware**, i.e. each sibling's track must itself be A*-clear, not a blind offset.

**Correct approach — obstacle-aware track assignment (left-edge sweep), not penalties or blind shifts:**
1. Group wire segments that share a channel (same axis, overlapping span,
   between the same pair of obstacle edges).
2. Within each channel, treat it as an interval-graph colouring: sort segments
   by entry coordinate, assign each the lowest track (offset = `k·separation`
   from the channel's near edge) not conflicting with an overlapping segment
   already placed. Tracks are reserved, so wires are ≥ `separation` apart by
   construction — the classic channel-router result.
3. Route topology first (A*, shapes only — that's Task 2.4, already shipped),
   then *shift* each segment onto its assigned track and re-join bends. Fan-out
   siblings (same `decl_span`) share a track.
4. If a channel needs more tracks than fit, that's the inherent case → place at
   min spacing and emit the R3 warning (R6).

- **Gate:** `tests/wire_rules` → **fixable R3/R4 → 0**, inherent R3 annotated;
  bundles render as separated rails. Accept snapshot; visual `wires_bus`,
  `wires_realistic`, `wires_fan`.

### Task 2.6 — determinism + polish
- Add a determinism test (route a sample twice via `validate_str`/layout → identical polylines).
- Replace any `HashMap` in the new path with `BTreeMap`/sorted.
- **Gate:** `cargo test`, clippy, fmt.

### Task 2.7 — retire the old engine
- Delete `route.rs`, `lanes.rs`, `stamping.rs`; strip the dead orchestration from `mod.rs`; trim `endpoints.rs`/`channels.rs`.
- Re-run full suite; review + accept conformance snapshot changes (wire paths change shape — verify each is an improvement, not a regression).
- **Gate:** zero invariant/R2 in `tests/wire_rules`; clippy/fmt clean; visual spot-check.

---

## Acceptance (Step 2 done)
- `tests/wire_rules` snapshot: **no invariant (R1/R4/R5), no R2**; R3 only where a clearance-respecting detour is impossible (and the count is small + explained).
- `wires_realistic`, `full_example`, `wires_bus` render clean: no piercing, no canvas-wide detours, parallels separated, crossings perpendicular.
- Determinism test passes.
- Conformance snapshots re-accepted with each change verified as an improvement.
- Old topology-template modules deleted; `src/layout/wires/` is graph-based end to end.
