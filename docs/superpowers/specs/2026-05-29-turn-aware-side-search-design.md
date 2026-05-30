# Turn-aware global side-search for wire routing

Status: **IMPLEMENTED** (`select.rs` + `score.rs`; see PLAN.md "Turn-aware global
side-search"). Column disaster fixed (A3 1→0), `wires_chain` X 1→0, row clean; one
residual flagged skim on `mermaid_fast`. Original design below.
Source of truth for routing rules stays [`WIRING.md`](../../../WIRING.md); this spec
describes a router change that better *satisfies* those rules. Implementation
phases land in [`PLAN.md`](../../../PLAN.md).

## Motivation

Routing review of `wires_realistic` (row) and a new `wires_realistic` (column)
surfaced four problems that all trace back to **side selection**, plus a test gap:

1. **`cat -> bowl` attaches off-centre.** The actual route puts bowl's left port at
   the bottom-left corner with a needless up-jog. Root cause: C3's "slide the port
   to a straight shot" decides a straight shot exists from **bbox overlap**, but the
   real test is **usable-span overlap** (inset-adjusted). cat's port sits *outside*
   bowl's usable span, so the slide clamps the port to the corner *and* the wire
   still jogs — the worst of both.

2. **Turns aren't minimised.** `bird -> roof` (two wires) routes with 2 and 3 turns;
   `water -> roof` with 4. Convergence forces *all* wires onto *one* side of `roof`
   (its left), so wires that would be cheaper on the bottom detour instead.

3. **Wires are biased toward the scene's layout axis.** `pick_edges` picks the
   dominant axis (left/right in a row, top/bottom in a column) and never considers a
   perpendicular exit even when far cleaner.

4. **Column layout is a mess.** Same weaknesses amplified, plus genuine `B2w`
   failures (wires at 0.0 separation the nudge can't recover).

5. **Tests are too narrow** — a single row scene, so 1–4 went unseen.

The architecture (visibility-graph A\* → ports → nudge → validator) is sound. The
fix is to replace the heuristic side selection with one principled, turn-aware
**global search**, fix C3, and broaden the test suite.

## What stays, what changes

```
resolve → layout
   → SELECT (NEW: choose each wire's two sides by a monotone turn-aware search)
   → ports (slots/order/C2–C5/C3 — GIVEN the chosen sides)
   → route (A* — unchanged)
   → nudge (track assignment — unchanged)
   → render
validator = independent ground truth (unchanged)
```

**Removed** (the patchwork being replaced): `ports::pick_all_sides` /
`least_loaded`; `mod::derive_hints`' skim-`reside`; `mod::converge_groups` /
`try_unify_group` / `overlay_resides`. Their jobs (obstacle-aware C1, crossing-aware
convergence, partitioning) are all *emergent* from the new search.

**Kept:** forced `.side`; fan trunks (E2); the provisional pass that yields
`lead_*` for C4 ordering; slot assignment / C2–C5 / C3; routing; nudge; validator.

## Design

### 1. Objective (scorecard)

A candidate routing's quality is a tuple compared lexicographically, smaller better:

```
(invariants, B1, B2n, B2w, crossings, turns, length_px)
```

The first five come from the independent validator (today's `quality`); `turns` is
the total bend count, `length_px` the summed polyline length in whole px (the
determinism-safe B6 proxy). `turns` is promoted to sit right after `crossings`, so
among equal-crossing layouts the router minimises bends (issue 2). Crossings still
rank above turns, matching the user's 0-crossing target cases. (If a real case wants
the WIRING "a crossing ≈ a few bends" trade, fold crossings+turns into one weighted
term — noted, not default.)

### 2. The search (monotone hill-climb over side assignments)

- **State** `S`: for each wire-segment, its `(side_a, side_b)`.
- **Seed**: the current geometric two-pass assignment (a good start, turn-aware via
  `pick_edges`).
- **Step**: enumerate candidate **moves** (below) in a fixed order; apply any move
  whose resulting objective is **strictly** smaller; repeat full scans until none
  improves, or an iteration/eval cap is hit.
- **Monotone**: the objective strictly decreases each accepted step ⇒ it terminates
  and **cannot oscillate** (the locked lesson). Strict `<` (never `≤`) means ties
  never move, so output is a deterministic function of the input.
- **Deterministic**: fixed wire order, fixed candidate order, deterministic scoring.

### 3. Candidate sides & moves

For a wire-end, candidate sides are the **facing** side plus its **two
perpendiculars** (never the back side — that always adds a U). Forced `.side` ends
and fan-trunk ends are pinned (not moved). Moves:

- **Single-end flip** — set one wire-end to a candidate side. Iterated, this alone
  yields convergence (flip onto a neighbour's side), **partitioning** (each wire
  finds its own best side — so `bird+water→roof` can take the bottom while
  `cat→roof` keeps the left), turn-minimisation, and perpendicular exits (issue 3).
- **Group unify** (coordinated) — for the set of wires sharing an endpoint node, try
  all-on-one-side for each candidate side. A cheap escape from local minima that a
  single flip can't reach (two wires that must move together).

Single-end flips are the workhorse; group-unify is a small, bounded add-on.

### 4. Proxy scoring & performance

Re-routing every wire for every move is the cost. The search scores moves with a
**route-only proxy** — re-plan slots, route each wire, count crossings/turns/length
on the raw routes (skip the nudge, which separates rails but rarely changes
crossings or bend count). Only the **final** winning assignment pays for nudge +
full validation. Caps: a bounded number of full scans (`MAX_SCANS`) and a total proxy
budget (`MAX_EVALS`); past the budget the search stops best-effort. (Truncation is
*silent* — a library has no log sink — but safe: the keep-better-vs-seed guarantee
means a truncated search is still **never worse than the geometric seed**.) Candidate
sides are facing + the two perpendiculars; the back side is opened only in a gated
second round when round 1 left a B1/B2n a back-side exit could rescue.

### 5. C3 usable-span fix (issue 1)

`ports`' single-wire port slide currently gates on bbox overlap. Gate it on
**usable-span overlap** instead: a straight shot exists only when the inset-adjusted
spans `[lo+clearance, hi-clearance]` of both boxes overlap. Otherwise the slide can't
reach a common coordinate without clamping to a corner, so the port stays **centred**
and the wire's turn lands on the side midpoint (`cat->bowl` fixed).

### 6. Module layout (refactor for clarity)

| File | Concern (after) |
|---|---|
| `mod.rs` | orchestration only: build reqs → seed → `select` → finish (route+nudge) |
| `select.rs` | **NEW** — candidate sides, moves, the hill-climb, proxy scoring |
| `score.rs` | **NEW** — the objective tuple (validator counts + turns + length) |
| `ports.rs` | slots / C2–C5 / C4 order / C3 slide, **given** sides (side-*choice* removed) |
| `route.rs`, `nudge.rs`, `graph.rs`, `scene.rs`, `oracle.rs`, `geometry.rs`, `validate.rs`, `text.rs` | unchanged in spirit |

One concept per file, each independently testable: `select` exposes "given reqs +
seed, return the chosen sides"; `score` exposes "given wires, return the tuple";
`ports` exposes "given sides, return ports."

### 7. Tests & samples (issue 5)

- Rename the two big samples to `wires_realistic_row` / `wires_realistic_column`.
- Add gated scenes covering the patterns side-selection must handle: a **star/hub**
  (many wires one node, every direction), a **grid** of nodes, **nested groups**,
  and a **mixed** scene. Both layout directions where it matters.
- Every scene runs the contract gates: A1–A5/B1 = 0, B2n = 0, B2w only where a side
  genuinely overflows (flagged). The scorecard snapshot tracks crossings/turns.
- Unit tests: C3 usable-span (centred vs slid); `select` finds the partition on a
  3-way convergence; `score` counts turns; a move that doesn't help is a no-op.

## Determinism & monotonicity (the two invariants that keep this safe)

- **Monotone**: every accepted move strictly lowers the objective; the seed is
  today's output, so the search can only improve it. No sample regresses on any
  tracked metric.
- **Deterministic**: fixed orders, strict-improvement, validator-based scoring, no
  hash-map iteration in decisions. `compile_is_byte_identical` stays green.

## Acceptance

- **Row** `wires_realistic_row`: `cat->bowl` connects to bowl's left midpoint;
  `bird→roof` / `water→roof` take minimal turns; crossings stay 0.
- **Column** `wires_realistic_column`: validates clean (no hard, no B2n; B2w only
  genuine overflow), visibly clean — no canvas-wide detours, no 0.0-separation
  overlaps.
- All new scenes pass the contract gates.
- Determinism, clippy `-D warnings`, `fmt --check` clean. Snapshots reviewed +
  PNG-checked.

## Risks & non-goals

- **Column routing is genuinely hard.** Goal: "much cleaner + zero hard/B2n," not
  provably optimal. The keep-better seed means worst case = today.
- **Search cost.** Bounded by the caps; the proxy keeps per-move cost low. Large
  scenes degrade to best-effort (logged), never to a wrong/over-long route.
- **Non-goal**: changing WIRING's contract (B1/B2 absolute; B3–B6 weighed). This is
  a better optimiser for the same rules. C2/C5/C3 wording already updated for the
  even-split and straight-shot guard; this change needs no further rule edits beyond
  noting perpendicular sides are eligible under C1.
