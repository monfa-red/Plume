# Turn-aware Global Side-Search — Implementation Plan

> **For agentic workers:** execute task-by-task with TDD; tick boxes as you go.
> Spec: `docs/superpowers/specs/2026-05-29-turn-aware-side-search-design.md`.
> Contract: `WIRING.md`. Living status: `PLAN.md`.

**Goal:** Replace the heuristic wire side-selection (geometric pick + least-loaded +
skim-reside + all-or-nothing convergence) with one monotone, deterministic,
turn-aware global search, fix the C3 straight-shot bug, and broaden test coverage.

**Architecture:** `select.rs` runs a hill-climb over per-wire `(side_a, side_b)`
assignments, seeded with today's output, scored by `score.rs`'s lexicographic tuple
`(invariants, B1, B2n, B2w, crossings, turns, length)` via a fast route-only proxy.
`ports.rs` is reduced to slots/order/C2–C5/C3 *given* sides. `mod.rs` orchestrates.

**Standing rules:** monotone (accept only strict improvements → no oscillation),
deterministic (fixed orders, no hash-map iteration in decisions), `cargo test` green
+ clippy `-D warnings` + `fmt --check` after every phase, PNG-check visual changes.

---

## Phase A — C3 usable-span fix (issue 1)

**Files:** `src/layout/wires/ports.rs`.

The single-wire slide gates on bbox overlap; make it case-precise on the
**usable span** `[lo+clearance, hi-clearance]` (slid axis). The bug is one-sided:
when a lone end's facing partner is *fixed* (a multi-wire slot or a fan hub), the
lone end slides toward the fixed coord and clamps to its corner. Cases:
- **both ends lone** → slide both to a common coord only if the usable spans
  *overlap* (else a straight shot is unreachable; keep centred).
- **one lone, partner fixed** → slide the lone end only if the fixed partner's coord
  is *reachable* (within the lone side's usable span); else keep centred.
- **neither lone** → no slide (slots fix both).

- [ ] Test `lone_target_stays_centred_when_a_fixed_source_is_out_of_its_span`: a tall
  `src` with two wires (a fixed slot side) and a low lone target whose usable span
  can't reach the slot → the target keeps its side midpoint. Watch it fail.
- [ ] Replace the `aligned_axis_overlaps` block with the three-case logic above
  (helper `usable(rect, side, clearance) -> (lo, hi)`; reuse `port_at`). Remove the
  now-unused `slide`.
- [ ] Existing `lone_facing_wire_slides_to_a_straight_shot`,
  `forced_opposite_but_offset_sides_keep_centred_ports`,
  `lone_wire_keeps_the_side_midpoint` still pass.
- [ ] Render `wires_realistic` (row): `cat->bowl` connects to bowl's left midpoint.
  Verify via the SVG path (bowl port y ≈ bowl centre y).
- [ ] clippy/fmt; accept the changed snapshots after PNG-check; commit.

**Gate:** `cat->bowl` centred; no hard/B2n regression suite-wide.

---

## Phase B — `score.rs`: the objective tuple

**Files:** Create `src/layout/wires/score.rs`; modify `mod.rs` (move `quality`),
`mod.rs` module list.

Move `quality` out of `mod.rs` into `score::score(wires, nodes) -> Score`, where
`Score = (usize,usize,usize,usize,usize,usize,usize)` =
`(invariants, B1, B2n, B2w, crossings, turns, length_px)`. `turns` = Σ
`path.len().saturating_sub(2)`; `length_px` = Σ `geometry::length(path).round()`.
(This is today's `quality` verbatim plus the two trailing terms it already computes —
just relocated and named.)

- [ ] Test `score_counts_turns_and_length`: a hand-built `RoutedWire` list with known
  bends/length → expected tuple. Watch it fail (module absent).
- [ ] Create `score.rs` with `Score` + `score()`; re-export from `mod.rs`.
- [ ] Replace `mod::quality` calls with `score::score`; delete `mod::quality` + the
  local `Score` alias.
- [ ] `cargo test`; clippy/fmt; commit. (No snapshot change — same numbers.)

**Gate:** suite green; `score` unit-tested; `mod.rs` no longer defines the tuple.

---

## Phase C — `select.rs`: the search (core)

**Files:** Create `src/layout/wires/select.rs`; modify `ports.rs` (remove side
*choice*), `mod.rs` (orchestrate, delete old convergence/reside code).

### C.1 — `ports::plan` takes sides as input

Split side-*choice* out of `ports::plan`. New signature:
`ports::assign(reqs, sides: &[(Side, Side)], hints: &[PlanHint]) -> Vec<Plan>` —
does only slots (C2/C5), C4 order (lead hints), C3 slide. Delete `pick_all_sides`,
`least_loaded`, `side_pref`, and the forced/reside side logic from `ports`.

- [ ] Move the existing tests that assert *slot* behaviour to drive `assign`; keep
  them green. (Side-choice tests like `diagonal_wire_leaves_the_least_loaded_side`
  move to `select.rs` in C.2, re-expressed against the search.)

### C.2 — candidate sides + seed

`select.rs`:
- `fn seed_sides(reqs, hints) -> Vec<(Side,Side)>` — the geometric facing pick
  (`pick_edges`) honouring forced `.side`; this is the search's starting point.
- `fn candidates(req, end, seed_side) -> Vec<Side>` — `{facing, perp1, perp2}` minus
  the back side; a forced end or fan-trunk end returns just its pinned side.

- [ ] Test `candidates_exclude_the_back_side_and_pin_forced`.
- [ ] Test `seed_matches_geometry_on_a_plain_pair`.

### C.3 — proxy scorer

- `fn proxy(reqs, sides, hints, nodes) -> Score` — `assign` → route each wire (no
  nudge) → `score` on the raw routes. Fast; used inside the search loop.

- [ ] Test `proxy_prefers_fewer_turns_between_two_side_choices` on a constructed pair.

### C.4 — the hill-climb (single-end flip)

`fn search(reqs, seed, hints, nodes) -> Vec<(Side,Side)>`:
deterministic scan over (wire, end, candidate); apply the first move with strictly
smaller `proxy`; repeat full scans until none improves or caps hit
(`MAX_SCANS`, `MAX_EVALS`, log on truncation). Forced/fan ends skipped.

- [ ] Test `search_unifies_a_two_way_convergence` (the old roof case → 0 crossings).
- [ ] Test `search_partitions_a_three_way_convergence` (cat+bird+water → roof: cat
  one side, bird+water another, 0 crossings, fewer turns than all-on-one-side).
- [ ] Test `search_is_a_no_op_when_seed_is_optimal` (a clean pair → seed unchanged).

### C.5 — group-unify escape move

Add a coordinated move: for each endpoint node's wire set, try all-on-each-side;
apply if strictly better. Interleave with single-end scans.

- [ ] Test `group_unify_escapes_a_local_minimum` (a case single-flips can't reach).

### C.6 — wire into `mod.rs`, delete the old machinery

`route_wires`: build reqs → provisional (for `lead` hints) → `seed_sides` →
`select::search` → `ports::assign` → `finish` (route+nudge). Delete `converge_groups`,
`try_unify_group`, `overlay_resides`, `side_of`, `side_rank`, the skim-`reside` half
of `derive_hints` (keep `lead_point`), and the `Resides`/`UnifyTrial` types.

- [ ] `cargo test` green; the convergence integration test
  (`realistic_convergence_crossings_are_minimised`) still passes (now via `search`).
- [ ] `compile_is_byte_identical` green.
- [ ] clippy/fmt; review + accept snapshots after PNG-check; commit.

**Gate:** row crossings 0 with minimal turns; suite hard/B2n 0; deterministic.

---

## Phase D — tests & samples (issue 5)

**Files:** rename `samples/wires_realistic.plume` → `wires_realistic_row.plume`;
keep `wires_realistic_column.plume`; add `samples/wires_star.plume`,
`samples/wires_grid.plume`, `samples/wires_nested.plume`. Update any test that names
`wires_realistic`. Regenerate snapshots.

- [ ] `wires_star`: one hub with ≥6 spokes in every direction → all reachable,
  even slots, 0 hard/B2n.
- [ ] `wires_grid`: 3×3 nodes with row+column wires → clean, minimal crossings.
- [ ] `wires_nested`: groups within groups, cross-boundary wires → passable
  ancestors respected, 0 hard/B2n.
- [ ] `no_sample_breaks_a_hard_guarantee` + `baseline_contract_report` cover all new
  scenes. PNG-check each.
- [ ] Commit (samples + snapshots).

**Gate:** every scene validates clean (B2w only genuine overflow); column visibly
clean (no canvas detours / 0.0-separation overlaps).

---

## Phase E — polish & docs

- [ ] `PLAN.md`: replace the convergence section with the side-search; mark issues
  1–5 resolved; note residuals.
- [ ] `WIRING.md`: confirm C1 wording covers perpendicular eligibility (it already
  says "the side giving the best route"); no other rule edits.
- [ ] Final: `cargo test`, clippy `-D warnings`, `fmt --check`, determinism all
  green; snapshots reviewed; PNG-check row + column + new scenes.
- [ ] Commit.

**Gate:** all acceptance criteria in the spec met, or residuals honestly noted.

---

## Self-review notes

- Every spec section maps to a phase: C3 fix→A, objective→B, search/moves/proxy→C,
  module split→B+C, tests/samples→D, determinism+monotonicity→standing rules +
  C.6/E gates, acceptance→A/C/D/E gates.
- Monotonicity holds because the seed = today's assignment and only strict
  improvements are accepted; worst case = today.
- Risk (column hardness) is acknowledged in the spec; Phase D gate requires "clean
  validate," not "optimal."
