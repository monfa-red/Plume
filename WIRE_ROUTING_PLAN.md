# Wire Routing — Multi-Phase Plan

A roadmap for the PCB-grade orthogonal router. Phase 1 is **shipped**.
Phases 2–4 are designed but unimplemented; this doc captures enough
context that any session can pick up the next phase cleanly.

The overall goal (in user's words):

1. Wires default to the shortest path.
2. Wires don't touch unless they must cross. Crossings are always 90°.
3. Wires only travel on the four cardinal axes.
4. If there's no space, wires go around. If genuinely impossible, surface
   it as a warning.

Phase 1 mostly delivers (2) and (3) and reinforces (1). Phases 2–4 finish
(1), (4), and improve (1) further with channel-aware routing.

---

## Phase 1 — Hard-obstacle routed wires (DONE)

**File:** `src/layout/wires.rs`. **Status:** shipped.

The router now treats every previously-routed wire as a hard obstacle
with a single carve-out: a perpendicular crossing is allowed at moderate
cost. Key types:

- `Cell` (u8 bitfield): `WALL`, `WIRE_H`, `WIRE_V`, `HALO_H`, `HALO_V`.
- `CellMap`: builds from shape obstacles + a list of routed paths; per-cell
  rules in `entry_for(cell, axis)` return `Free` / `Cross` / `Blocked`.
- A* enforces "no bend at a cross" by checking
  `perpendicular_cross_here(cells, cell, dir)` on the current cell — if
  set, only continuing in `dir` is legal.
- Cost shape: step 1, bend +4, cross +8.

Lane fanning step bumped to `gap` (was `gap/2`) so adjacent lanes land in
different grid rows after the `floor`-snap. Halo zones now have room to
breathe.

**Visible result:** parallel wires actually fan apart; perpendicular
crossings happen at clean 90° angles; no more "L-bend in random place".

---

## Phase 2 — Bus routing for parallel pairs (DONE)

**File:** `src/layout/wires.rs`. **Status:** shipped.

When N wires share `(src, src_edge, tgt, tgt_edge)` they form a *bundle*.
The bundle's canonical wire is routed once via A*; siblings are stamped
by perpendicular polyline shift. Result: true rails — siblings are
guaranteed parallel because they share an A* route.

Key additions:

- `group_bundles(&[SegmentSpec]) -> Vec<Bundle>` — group by the 4-tuple key.
- `assign_bundle_lanes(&[Bundle], &[SegmentSpec])` — per-bundle lane offsets.
  Each (shape, edge) bin packs its bundles into contiguous lane ranges;
  the bundle's lane = the centre of its range.
- `shift_polyline(&path, delta)` — translate each orthogonal segment
  perpendicular to its axis; replace each bend with the intersection of
  the two shifted lines.
- Replacing the per-spec routing loop with a per-bundle loop in
  `route_wires`. After routing the canonical, every sibling's path is
  marked in CellMap so subsequent bundles route around the whole bus.

**Acceptance:** `samples/wires_bus.plume`, `samples/wires_realistic.plume`'s
parallel `bowl -> dog` lines, and `samples/wires_fan.plume`'s
`bowl -> apple & mug & mouse` all render as clean parallel tracks.

### Not yet done in Phase 2 (deferred to a future task)

- **True fan-out collapse** — `cat -> dog & bird` currently emits two
  independent bundles (one per target) because the bundle key requires
  same target. If we want the wires to share a single trunk out of the
  source and split at a branch point, we'd extend the bundle concept:
  group by `(src_id, src_edge)` alone, route the bundle, then split each
  sibling at a chosen branch cell. This is a Phase 2½ extension; not
  blocking.

---

## Phase 3-lite — Channel-aware A* (DONE)

**File:** `src/layout/wires.rs`. **Status:** shipped.

A pragmatic intermediate step between Phase 2 and full channel
decomposition. Instead of restructuring routing to navigate a channel
graph, we identify "corridor" rows and columns (entirely wall-free) and
give A* a small cost surcharge (+1) for travelling along non-corridor
tracks. Wires gravitate toward natural gaps between shapes.

Key additions:

- `Corridors::from(&CellMap)` — precompute per-row and per-column flags.
- `corridors.on_corridor(cell, axis)` — query whether `cell` sits on a
  corridor track for the given travel axis.
- `a_star` now takes `corridors: &Corridors` and adds `OFF_CORRIDOR` (+1)
  per non-corridor step.

Cost shape becomes: step 1, off-corridor +1, bend +4, cross +8.

**Visible result:** in `samples/full_example.plume`, the orange dashed
`owl --> rabbit` wire now routes along the wide corridor at the top of
the canvas (above all the shapes) instead of weaving between them.
Across all wires, L-bends land at more geometrically meaningful Y/X
values — the visible randomness drops.

This isn't full Phase 3 (no channel graph, no two-level routing, no track
assignment). It's the 80% win for ~50 LOC. Full Phase 3 below is still
worth doing for diagram-heavy use cases, but most hand-authored Plume
files won't benefit much beyond this.

---

## Phase 3 — Channel decomposition (deferred)

**Goal:** end the "random L-bend" feel. Bends should happen at
*channel corners* — geometrically meaningful inflection points (where
two empty rectangles between shapes meet) — not wherever A* decides.
This is what makes ELK / dagre layouts look "tidy".

### What changes

Today A* explores a uniform grid where every cell is equivalent. After
Phase 3 it navigates a **channel graph**:

- A **channel** is a maximal rectangular region of free space between
  shapes (and the canvas border).
- Each channel has a **capacity** = `floor(width / wire-gap)` along its
  short axis (the number of parallel tracks that fit).
- The channel graph connects channels that share an edge — wires move
  between channels through those shared boundaries.

A* runs at TWO levels:
1. **Global** routing: pick a sequence of channels from source to target,
   minimising track demand and channel crossings.
2. **Detailed** routing: within each channel, assign each wire to a
   specific track (1, 2, ..., capacity), using the classic left-edge
   algorithm (sort by start position, sweep).

### Where it slots in

This is a partial rewrite, not a refactor. The current `Grid` + `CellMap`
+ uniform A* becomes a `ChannelMap` + `TrackAssignment` + two-level
routing. The orchestrator in `route_wires` is mostly the same — it just
calls into the new routing layer.

Suggested file split:

```
src/layout/wires/
  mod.rs              -- orchestrator (today's route_wires)
  channels.rs         -- ChannelMap: decompose plane into channels
  global.rs           -- per-wire channel-sequence A*
  detailed.rs         -- track assignment within channels
  cells.rs            -- (today's CellMap; kept for fallback / leaf routing)
```

### Algorithm sketch

1. **Channel decomposition.** Given shape bboxes inflated by `wire-gap`,
   slice the plane along their edges. Each rectangular region between
   shapes is a channel. (Standard plane-sweep, ~O(n log n) on shape
   count.)
2. **Channel graph.** Edges between channels that share a boundary.
   Channel capacity = `floor(short_dim / wire-gap)`.
3. **Per-wire global route.** A* on the channel graph. Cost = channel
   length + a term for "track demand" so wires prefer less-crowded
   channels.
4. **Track assignment.** For each channel, collect the wires passing
   through, run left-edge: sort wires by entry position along the
   channel's long axis; assign each to the lowest free track.
5. **Emit polylines.** For each wire, stitch its assigned tracks across
   channels into a final polyline. Bends happen exactly at channel
   corners — that's the visual win.

### Tricky bits

- **Channel decomposition is the hard part.** ELK's implementation is
  ~1000 lines just for this. We can use a simpler "horizontal strip"
  decomposition (slice the plane by horizontal lines at every shape's
  top and bottom Y) for a first pass — coarser channels, but easier to
  implement (~150 LOC).
- **Endpoint connection.** Wires start at a shape edge, not inside a
  channel. The first/last segment is a short "stub" from shape edge to
  the entry of the first channel.
- **Falling back.** When channel routing fails (e.g. wire endpoints are
  inside groups not on a channel boundary), fall back to Phase 1's A*
  with the existing `CellMap`. So `cells.rs` stays.

### Estimated effort

This is the big one. Realistic: 2 focused sessions.
- Session A: channel decomposition + channel graph + visualisation
  (debug output of channels as faint rectangles in the SVG, for
  development).
- Session B: global routing + track assignment + integration.

### Acceptance test

`samples/wires_realistic.plume` and `samples/full_example.plume` should
show:
- Wires bend only at channel corners (you can mentally trace where shapes
  end and start — bends line up).
- Tracks within channels are evenly spaced.
- No more random "L is here, no wait L is 3px to the right".

---

## Phase 4 — Capacity overflow diagnostics

**Goal:** when a channel demands more tracks than fit, the router should
either (a) tighten spacing locally, (b) detour the overflowing wires
around to less-crowded channels, or (c) emit a warning. Default policy:
**warn-then-detour** (matches PCB DRC behaviour).

### Where it slots in

A small addition on top of Phase 3's `detailed.rs`. After the left-edge
sweep, for each channel:

```
if demand > capacity {
    excess = demand - capacity
    pick `excess` wires whose alternative routes are cheapest
    re-route those wires forbidding this channel
    if re-route fails: emit warning, allow over-capacity assignment
}
```

### Diagnostic surface

Reuses `crate::error::Diagnostic` — emit a warning per overflow with the
channel's bbox and the overflowing wires' source/target paths.

Same machinery the lint pass uses (`src/lint.rs`). The `Diagnostic` flows
through to the CLI; `--strict` promotes to error.

### Estimated effort

~½ session, after Phase 3 is in.

---

## Order of operations

Recommended:

1. **Phase 1** (DONE).
2. **Phase 2** — high leverage, low risk, no architecture change. Ship next.
3. **Phase 3** — bigger commitment. Wait until samples actually benefit.
4. **Phase 4** — easy once Phase 3 lands.

Phase 2 alone gets us *most* of the visible improvement the user is
asking for. Phase 3 is the difference between "looks good" and "looks
like KiCad". Phase 4 is the safety rail.

---

## Files / functions to know

- `src/layout/wires.rs::route_wires` — top-level orchestrator. Phase 2
  changes here.
- `src/layout/wires.rs::SegmentSpec` — per-wire state for routing.
  Phase 2 bundles these.
- `src/layout/wires.rs::CellMap` — Phase 1 abstraction. Phase 2 still
  uses it; Phase 3 supplements with channels.
- `src/layout/wires.rs::a_star` — current single-wire router. Stays for
  fallback in Phase 3.
- `src/resolve/mod.rs::resolve_wire` — produces `ResolvedWire`s. No
  changes needed for Phase 2+; the router does all the work.

## Testing strategy

For each phase:

1. Add 1-2 minimal samples that exercise the new behaviour (e.g.
   `samples/wires_bus.plume` for Phase 2: `cat -> dog & bird` with
   multiple parallel pairs).
2. Snapshot via the existing `tests/conformance.rs` glob.
3. Visually verify with `cargo run -- <sample> --bake-vars -o /tmp/x.svg
   && resvg /tmp/x.svg /tmp/x.png` and read the PNG.
4. Re-render `samples/wires_realistic.plume` — the canonical
   "is the router still good?" test.

## Decisions left to make

When picking up the next phase, settle these first:

- **Phase 2:** "bundle" definition — same `(src,src_edge,tgt,tgt_edge)`
  only, or also same source different targets (one-to-many)? Recommend
  starting with the strict version; extend to one-to-many in a follow-up.
- **Phase 3:** strip decomposition (cheap, coarser) vs full plane sweep
  (correct, harder)? Recommend strip for v1; full sweep later if quality
  demands.
- **Phase 4:** policy — warn-then-detour, or auto-tighten gap? Recommend
  warn-then-detour; auto-tighten is a one-line change to add later.
