# Next session — wire spacing bug

## The bug

In `samples/wires_realistic.plume`, the wires arriving at **Dog** sit closer
than the configured `|wire| gap:N`. With `gap:12`:

- `bowl → dog × 2` (red, two parallel siblings of one bundle of size 2)
- `water → dog` (blue, separate decl)

These three wires arrive at Dog's left edge stacked at y = −28.4, −16.4, −4.4.
At first glance that's `gap = 12` apart. **But the geometry near the
endpoint is wrong** — the blue wire's vertical segment runs in the column
right next to bowl→dog's vertical segment, *and* one ends a few pixels above
where the other begins, forming a "broken vertical line" with a ~6 px (=
gap/2) gap between the two V segments. PCB-style, that violates clearance.

Also: setting `|scene| gap:N` does **not** affect wire-to-wire spacing
(scene gap is for child layout, wire gap is for routing). User tried this and
reasonably expected it to do *something*. That's a SPEC ambiguity to revisit.

User said: "the gap between blue and red line is still smaller than the
value gap in |wire|, also I tried setting a large gap for |scene| and that
didn't change the gap between the wires."

## What was tried in the previous session (and reverted)

Commit `c3bff86` ("endpoint-runway push must respect existing wires") tried
to fix this by making `enforce_endpoint_runways` obstacle-aware. **It did
not work** — the user said it looked worse. Reverted with
`git reset --hard 6aeaf49` and force-pushed.

That commit's idea was right *in principle* (the post-process push was
crossing the CellMap), but the partial fix wasn't enough. The deeper problem
is that the **canonical's A* path** itself, before any post-processing,
chose a route whose bends land at columns/rows the other wire also wants —
and the per-spec lane allocation puts the siblings of one bundle adjacent
in Y to the next bundle's wire, creating the cluster.

## Where to look

- `src/layout/wires.rs`
  - `route_wires` — top-level orchestrator. Bundles are routed in
    declaration order; each routed wire's path is marked in CellMap so
    later A* runs avoid it.
  - `assign_bundle_lanes` / `place_lanes` — per-spec slot allocation in
    each (shape, edge) bin. Specs from the same WireDecl share a slot
    (fan-out unification); specs from different decls get distinct
    consecutive slots.
  - `enforce_endpoint_runways` / `push_tail_bend_back` — post-process
    that pushes the last bend back along its perpendicular segment to
    guarantee a minimum endpoint straight length. **Currently does not
    check the CellMap** — that's why a push can land on top of another
    wire.
  - `CellMap` (around line 850) — per-cell flags WALL / WIRE_H / WIRE_V
    / HALO_H / HALO_V. `mark_wire_path` marks WIRE_x along the wire's
    track and HALO_x along the parallel-too-close zone. `entry_for(cell,
    axis)` returns Blocked / Cross / Free for A* step decisions.
  - `ENDPOINT_PAD_CELLS = 2` (around line 920) — extends each wire's
    claim 2 cells along its own axis at each end, so two same-axis
    wires can't end one cell apart. This fixes one half of the spacing
    issue but doesn't address the "two V columns side-by-side, endpoints
    misaligned" case visible in the screenshot.

- `samples/wires_realistic.plume` — the canonical repro. The user modified
  `.quiet` to a solid stroke and `<-o` to `<-` so the issue is more visible.

## What the fix should probably look like

Two leverage points, either one might fix it:

1. **Stronger same-axis spacing in CellMap.** Right now `mark_vertical_segment`
   marks `WIRE_V` along the wire's column **plus** `HALO_V` on the columns
   immediately to the left and right. The halo only blocks *same-axis*
   approach in adjacent columns. It does **not** block another wire from
   running vertically in the SAME column at a non-overlapping Y range. The
   `ENDPOINT_PAD_CELLS = 2` extension tries to handle this but the cell
   discretisation means the boundary at the wire's end is fuzzy (one cell =
   gap/2). Bumping `ENDPOINT_PAD_CELLS` to 3 (= 1.5 gaps) might work, or
   widening the halo to ±2 columns instead of ±1. Try both and visualise.

2. **A* cost: penalise sharing X/Y track with an existing wire.** Even when
   the CellMap doesn't strictly block, give A* a substantial cost penalty
   for putting a V segment in a column that already has a V segment elsewhere
   (or H in same row). Right now the only such cost is the CROSS penalty
   (8 per crossing). Add a SAME_TRACK penalty that fires when entering a
   cell whose column (for V movement) is already used by another V wire
   anywhere in the grid. Implementation: maintain `vert_columns: Vec<bool>`
   and `horz_rows: Vec<bool>` alongside CellMap, set them when marking
   wires, check in A*.

3. **|scene| gap propagation** — the user expected `|scene| gap:N` to also
   affect wire spacing. Either:
   - Document explicitly in SPEC that `|scene|` gap is for child layout
     only, and `|wire| gap` is for wire-to-wire spacing (these are
     separate). Add a SPEC note.
   - OR make `|wire| gap` default to inherit from `|scene| gap` when not
     explicitly set.

   This is a small SPEC decision, not a bug — but the user flagged it,
   so address it explicitly.

## How to verify

```
cargo run -- samples/wires_realistic.plume --bake-vars -o /tmp/wr.svg
resvg /tmp/wr.svg /tmp/wr.png
# Read /tmp/wr.png
```

The fix is right when:
- The three wires arriving at Dog (two red bowl→dog siblings + one blue
  water→dog) are visually `gap = 12` apart **at every point near Dog**, not
  just at the final L-bend.
- The vertical segments of the wires do **not** share columns AND don't
  end with less than `gap` between their endpoints in Y.
- All other samples (`samples/full_example.plume`, `samples/wires_fan.plume`,
  `samples/wires_bus.plume`, etc.) still render cleanly — no regression.

## Don't repeat these mistakes

- Don't move `enforce_endpoint_runways` to "obstacle-aware" by iteratively
  shrinking push. That was the previous session's failed attempt — it just
  moved the problem.
- Don't lower the `min_runway` to compensate. The runway is needed for
  marker clearance; making it too small reintroduces "arrow on top of bend".
- Don't ship and trust visually until the user confirms. The previous session
  shipped a fix that looked OK in one screenshot but the user spotted the
  same bug in a slightly different layout.

## Context state at handoff

89 tests pass on `main` after the revert. clippy + fmt clean.
`samples/wires_realistic.plume` is in its current modified form (the user's
solid-style version) — leave it as-is.
