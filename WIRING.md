# Wiring — Routing Rules

The **contract** for wire routing: the rules a correct routing must satisfy,
written to be checked mechanically and independently of any algorithm. The engine
that produces routes lives in the implementation plan; its intended shape is in
the appendix. This is the source of truth for routing — SPEC section 10 points here.

A routing is **correct** when it never breaks a hard invariant (section A), holds the
constraints B1–B2 (or relaxes them only as a flagged last resort, section B), and
otherwise minimises the objective B3–B6. Invariants are absolute; B1–B2 are held
wherever possible and flagged when not; B3–B6 are optimised. The engine pursues
this greedily (route-then-nudge, appendix) — globally-optimal routing is NP-hard,
so **"perfect" means "obeys every rule and looks clean,"** not "provably minimal."

---

## Definitions

- **node** — a shape wires connect to, treated as its **bounding box**: a
  rectangle with four **sides** (top/right/bottom/left). Non-rectangular shapes
  (oval, hex, …) use the bbox, so a wire meets the bbox edge, not the visible
  curve. (R9)
- **wire** — an orthogonal polyline joining a side of one node to a side of
  another. **port** — where a wire meets a side. **segment** — one axis-aligned
  piece between bends. **bend** — a 90° corner. (R8)
- **obstacle** (per wire) — any node *except* this wire's two endpoints and their
  ancestor containers, and *except* text nodes. A container group that is not such
  an ancestor is one **solid** obstacle (its whole bbox — the router won't thread
  between its children). (R2, R3, R12)
- **passable** — a wire's endpoint nodes and their ancestor containers are not
  obstacles *for it*; it may cross their boundaries (their non-endpoint children
  still are obstacles). Ancestor containers stay **fully** passable — a wire must
  cross them to reach its endpoint. An **endpoint node**, though, is passable only
  to the wire's **attaching stub** (the single segment that lands on its port);
  every *other* segment of that wire keeps `clearance` from the endpoint, exactly
  as from any obstacle (B2). Self-loops (E3) and fan-trunk siblings (E2) are
  exempt. (R3)
- **text node / wire label** — never obstacles, never connectable; a label rides
  on its wire (the renderer haloes it). (R12)
- **clearance(w)** — the minimum distance wire `w` keeps from obstacle nodes *and*
  from other wires (per-wire, default 16) — one value covers both. **separation(w1,
  w2)** = `max(clearance1, clearance2)`. *(A wire's `clearance` is its own
  property — not to be confused with a container's `gap`, which spaces its
  children.)*
- **corner inset** — ports and passing wires stay ≥ `clearance` from a node's
  corners. (R13)
- **bundle** — wires sharing the same ordered (source side, target side); drawn as
  parallel rails `separation` apart.
- **fan group** — wires from one declaration sharing an endpoint: fan-out
  `a -> b & c` (shared source) or fan-in `a & b -> c` (shared target); they share
  the trunk there. A **chain** `a -> b -> c` is *not* a fan — it is independent
  wires, each fully separated.

---

## A. Hard invariants — never violated

- **A1 Orthogonal** — axis-aligned segments, 90° bends, no zero-length or
  collinear joints. (R8)
- **A2 Attachment** — the segment at a node is perpendicular to the side, ends on
  the side, ≥ corner inset from the corners. (R5, R11, R13)
- **A3 Crossings perpendicular** — wires cross only at a single 90° point, never
  sharing a parallel run. Coincident runs are allowed only for fan-group siblings
  on their shared trunk (drawn once). (R5)
- **A4 Sides only** — wires attach only to node sides, never to text nodes. (R9, R10, R12)
- **A5 No self-crossing** — a wire never overlaps or crosses itself.

---

## B. Priority — what a route optimises

section A is absolute. Beyond that:

**Constraints — kept; relaxed only when no route can satisfy them (lowest-impact
first, always flagged):**

1. **B1 No node overlap** — never enter an obstacle's interior. *(relax: only a
   trapped endpoint → error)* (R2)
2. **B2 Clearance** — ≥ `clearance` from obstacle nodes, ≥ `clearance` from the
   wire's own endpoint nodes (except its attaching stub), and ≥ `separation` from
   other wires, except exactly at a perpendicular crossing. *(relax: sub-clearance,
   only when nothing else routes → warning)* (R6, R7)

**Objective — among constraint-respecting routes, minimise (3–4 are weighed
together, not strictly ranked):**

3. **B3 Crossings** — a perpendicular crossing costs ≈ a few bends (a tunable
   penalty, *not* forbidden). So a clean crossing beats a multi-bend detour, while
   a crossing you can dodge with a bend or two is still dodged. Crossings are
   normal output — not flagged. (R4)
4. **B4 Bends** — each 90° turn costs, weighed with B3 (and heavy enough that
   fewer turns usually wins). (R1)
5. **B5 Length** — breaks what crossings and bends leave. (R1)
6. **B6 Tidiness** — align segments onto shared channels; alignment wins whenever
   it costs less than a small length tolerance, so near-parallel wires snap
   together instead of sitting slightly offset.

A wire spends bends, length, and crossings freely to stay within `clearance` (B2)
and never overlaps a node (B1) unless trapped. Only B1/B2 relaxations are flagged
(error / warning) — never silent.

---

## C. Ports & sides

- **C1 Side selection** — a forced side (`a.r`) wins; else the side giving the best
  route (section B). Among *equally good* sides — same bend count, length within a small
  tolerance (e.g. a diagonal target reachable by an L off either adjacent side) —
  pick the **least-loaded**, so wires fan across sides instead of crowding one (a
  tie-break, never a worse side). Remaining ties: right → bottom → left → top. A
  side crowds (C5) only when geometry truly forces it (many targets the same way). (R10)
- **C2 Even spacing** — port **slots** on a side are uniform, fixed by the side and
  wire *count* only (never per-wire from targets). Slots sit symmetric about the
  centre, `separation` apart when they fit, else the largest uniform spacing that
  does (C5); outermost ≥ corner inset. *Which* wire takes *which* slot is the C4
  order. One wire → centre. A fan group's shared end is one slot. Every wire on a
  side shares one spacing, so you never get mixed compacted/uncompacted wires. (R11)
- **C3 Bend-avoidance (single-wire side)** — a side with one wire may slide its
  port off-centre to kill a bend (avoiding a turn beats centring). Multi-wire sides
  keep C2. (E1)
- **C4 Ordering** — order the wires sharing a side/channel so they don't needlessly
  cross (removes avoidable crossings for free); any crossing left is one B3 judged
  cheaper than detouring. Deterministic (section D).
- **C5 Overflow** — if a side *still* can't fit its wires at `separation` + inset,
  space them evenly across the span (inset to inset); spacing drops below
  `separation` — flagged (B2). No floor and no spilling to other sides — density is
  the user's lever (`clearance`, node spacing). Even spacing never overlaps at any
  real side length.

---

## D. Determinism

Same diagram → byte-identical routes. Wires route in a fixed order (declaration
order, then as-written within a fan/chain, then geometric), and no decision depends
on hash-map iteration order.

---

## E. Multi-wire & special cases

- **E1 Bundles** — parallel/duplicate wires between the same pair of sides run as
  rails `separation` apart.
- **E2 Fan groups** — a fan-out/fan-in shares its trunk and is exempt from B2 where
  the siblings coincide there; they split cleanly toward their ends. (Chains aren't
  fans.)
- **E3 Self-loops** (`a -> a`, or any same-node wire) — an orthogonal loop: exit
  one side (default right), out ≥ `clearance`, return to an adjacent side (default
  top); forced sides honoured.
- **E4 Passable ancestors** — a wire ignores its own endpoints' ancestor
  containers; only non-endpoint shapes are obstacles. (R3)

---

## Appendix — intended engine (non-normative)

Follows the proven **libavoid** model (Wybrow, Marriott & Stuckey, *Orthogonal
Connector Routing*, GD'09; cf. ELK's orthogonal router and PCB autorouters):

1. **Orthogonal visibility graph** — horizontal/vertical lines through every side
   ± clearance and every port; graph nodes at intersections; edges are clear
   segments.
2. **A\*** per wire, cost ordered by section B; committed wires add a crossing penalty
   (≈ a few bends, B3), so a crossing is taken whenever it is cheaper than detouring.
3. **Order & nudge** — group co-linear segments into channels, assign tracks
   `separation` apart (C2/C4), push crossings to channel ends, centre lone wires
   (C3), snap near-parallels onto shared tracks (B6); then validate sections A–B and flag
   relaxations.

Routing one wire at a time in the section D order keeps it deterministic and tractable;
the nudge pass recovers most of the quality a global optimiser would, which is why
it looks clean without solving the NP-hard global problem.

**Rule map** (nothing dropped): R1→B4/B5 · R2→B1 · R3→E4/Defs · R4→B3 · R5→A2/A3 ·
R6→B2 · R7→B2 · R8→A1 · R9→Defs/A4 · R10→C1 · R11→C2 · R12→A4/Defs · R13→A2/Defs · E1→C3
