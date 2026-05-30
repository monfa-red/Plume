//! Side selection — choosing each wire-end's side by a turn-aware global search.
//!
//! Side choice is the hard part of clean orthogonal routing: pick well and a wire is
//! a short L; pick badly and it detours, crosses, or skims its own node. Rather than
//! a chain of geometric special cases (facing pick, least-loaded, re-elect-on-skim,
//! convergence-unify), we run **one monotone hill-climb**. Seeded with the geometric
//! facing pick, it repeatedly flips a single wire-end — or unifies a node's wires —
//! onto a candidate side and keeps any change that **strictly** lowers the `score`
//! scorecard. Each candidate is scored by routing it *and running the nudge* (so the
//! proxy matches the shipped geometry — a route-only score mispredicts the crossings
//! the nudge adds when it separates a bundle). Strict improvement ⇒ it terminates,
//! never oscillates, and is a deterministic function of its input. Convergence
//! nesting, partitioning, turn-minimisation, and perpendicular exits all emerge from
//! the same mechanism.

use super::geometry::{pick_edges, Pt};
use super::ports::{self, PlanHint, SegReq};
use super::score::{score, Score};
use crate::ast::Side;
use crate::layout::ir::{PlacedNode, RoutedWire};
use crate::resolve::{AttrMap, Markers, ResolvedValue};
use crate::span::Span;
use std::collections::BTreeMap;

/// Full scans of the move set before stopping (it converges in 2–3 on real scenes;
/// the cap just bounds pathological inputs — truncation only forgoes improvement).
const MAX_SCANS: usize = 6;

/// Total route-only proxy evaluations before the search stops, best-effort. Each
/// proxy routes every wire, so cost grows with scene size; this bounds the wall time
/// on pathological inputs (e.g. one hub with dozens of spokes). Real scenes converge
/// (no improving move) in a few hundred evals, far under this; truncation is safe —
/// the result is still the monotone best-so-far, never worse than the seed.
const MAX_EVALS: usize = 4000;

/// The search's working context: the inputs, the per-wire scene cache the nudge
/// reuses across every candidate, and the spent-evaluation budget — so every
/// candidate scoring goes through one place that counts and can cut off.
struct Search<'a> {
    reqs: &'a [SegReq],
    hints: &'a [PlanHint],
    nodes: &'a [PlacedNode],
    scenes: Vec<super::nudge::WireScene>,
    evals: usize,
}

impl Search<'_> {
    fn proxy(&mut self, sides: &[(Side, Side)]) -> Score {
        self.evals += 1;
        proxy(self.reqs, sides, self.hints, self.nodes, &self.scenes)
    }

    fn exhausted(&self) -> bool {
        self.evals >= MAX_EVALS
    }
}

/// The geometric seed: every end on the facing side `pick_edges` gives, honouring a
/// forced `.side`. A self-loop has no facing geometry → right→top (E3); the search
/// leaves self-loops alone.
pub fn seed_sides(reqs: &[SegReq]) -> Vec<(Side, Side)> {
    reqs.iter()
        .map(|r| {
            if r.a_node == r.b_node {
                return (
                    r.forced_a.unwrap_or(Side::Right),
                    r.forced_b.unwrap_or(Side::Top),
                );
            }
            let (ga, gb) = pick_edges(r.a, r.b);
            (r.forced_a.unwrap_or(ga), r.forced_b.unwrap_or(gb))
        })
        .collect()
}

/// Hill-climb from `seed`, returning the chosen `(side_a, side_b)` per wire. Every
/// accepted move strictly lowers the proxy scorecard, so the search is monotone and,
/// with fixed wire/candidate order, deterministic.
pub fn search(
    reqs: &[SegReq],
    seed: &[(Side, Side)],
    hints: &[PlanHint],
    nodes: &[PlacedNode],
) -> Vec<(Side, Side)> {
    // The per-wire scene cache (obstacles + endpoint rects) is constant across the
    // search — build it once from a template and reuse it for every candidate nudge.
    let template: Vec<RoutedWire> = reqs.iter().map(|r| proxy_wire(r, Vec::new())).collect();
    let mut cx = Search {
        reqs,
        hints,
        nodes,
        scenes: super::nudge::build_scenes(&template, nodes),
        evals: 0,
    };
    let mut sides = seed.to_vec();
    let mut best = cx.proxy(&sides);

    // Round 1 hill-climbs with facing + perpendicular candidates only — the back
    // side usually just adds a U-turn, and admitting it into the main greedy climb
    // can steer it into a worse local optimum. Round 2 re-opens the back side, but
    // only when round 1 left a node overlap (B1) or an endpoint skim (B2n) that a
    // back-side exit could rescue — a wire boxed in on its other three sides. It
    // climbs from round 1's state, so it only ever improves; gating it keeps the
    // common (already-clean) scene to a single round.
    hill_climb(&mut cx, &mut sides, &mut best, seed, false);
    if best.1 > 0 || best.2 > 0 {
        hill_climb(&mut cx, &mut sides, &mut best, seed, true);
    }
    sides
}

/// One hill-climb pass: scan the move set until no move strictly improves, or the
/// eval budget runs out. `allow_back` opens the fourth (back) side for single-end
/// flips. Every accepted move lowers the proxy scorecard, so this is monotone.
fn hill_climb(
    cx: &mut Search,
    sides: &mut Vec<(Side, Side)>,
    best: &mut Score,
    seed: &[(Side, Side)],
    allow_back: bool,
) {
    'scans: for _ in 0..MAX_SCANS {
        let mut improved = false;

        // Single-end flips — the workhorse. Yields convergence (flip onto a
        // neighbour's side), partitioning (each end finds its own best side),
        // turn-min and perpendicular exits.
        for s in 0..cx.reqs.len() {
            if cx.reqs[s].a_node == cx.reqs[s].b_node {
                continue; // self-loop: no side search
            }
            for is_a in [true, false] {
                for cand in candidates(&cx.reqs[s], is_a, side_at(seed, s, is_a), allow_back) {
                    if cand == side_at(sides, s, is_a) {
                        continue;
                    }
                    if cx.exhausted() {
                        break 'scans;
                    }
                    let mut trial = sides.clone();
                    set_side(&mut trial, s, is_a, cand);
                    let q = cx.proxy(&trial);
                    if q < *best {
                        *sides = trial;
                        *best = q;
                        improved = true;
                    }
                }
            }
        }

        // Group-unify escape — pin all of a node's movable ends onto one side at
        // once, reaching layouts no single flip can (two wires that must move
        // together to stop crossing).
        for group in node_groups(cx.reqs) {
            let g_sides = group_sides(seed, &group);
            improved |= try_unit(cx, sides, best, &group, &g_sides);
        }

        // Fan-trunk re-election — a fan's siblings share one slot at their hub, so a
        // single flip can't move them (they're pinned). Move the whole trunk as a
        // unit instead, trying each side, so a hub side that skims can re-elect.
        for trunk in fan_trunks(cx.reqs) {
            improved |= try_unit(cx, sides, best, &trunk, &ALL_SIDES);
        }

        if !improved || cx.exhausted() {
            break;
        }
    }
}

/// Candidate sides for one end: the facing side and its two perpendiculars, plus the
/// back side when `allow_back` (it usually adds a U-turn, but with obstacles it can
/// be the only clean exit). A forced `.side` or a fan-trunk end is pinned to its
/// single side (a fan's siblings must share one slot, E2).
fn candidates(req: &SegReq, is_a: bool, seed_side: Side, allow_back: bool) -> Vec<Side> {
    let forced = if is_a { req.forced_a } else { req.forced_b };
    if let Some(f) = forced {
        return vec![f];
    }
    if (if is_a { req.fan_a } else { req.fan_b }).is_some() {
        return vec![seed_side];
    }
    let back = back_side(seed_side);
    ALL_SIDES
        .into_iter()
        .filter(|&s| allow_back || s != back)
        .collect()
}

/// The opposite side — a flip to here from `s` is a U-turn.
fn back_side(s: Side) -> Side {
    match s {
        Side::Top => Side::Bottom,
        Side::Bottom => Side::Top,
        Side::Left => Side::Right,
        Side::Right => Side::Left,
    }
}

/// The score of a side assignment, by routing it the way the final build will:
/// assign ports, route each wire, **and run the nudge**, then score. Routing without
/// the nudge mispredicts both ways — it over-counts the shared runs / sub-separation
/// the nudge fixes, yet misses the crossings the nudge introduces when it separates a
/// bundle. Nudging here makes the proxy match the real geometry the search is choosing
/// between, so it optimises what actually ships. The nudge runs in its cheap
/// (`thorough = false`) mode with the precomputed scene cache, so this hot-loop
/// scoring stays fast; the final build re-nudges thoroughly. `MAX_EVALS` bounds it.
fn proxy(
    reqs: &[SegReq],
    sides: &[(Side, Side)],
    hints: &[PlanHint],
    nodes: &[PlacedNode],
    scenes: &[super::nudge::WireScene],
) -> Score {
    let plans = ports::assign(reqs, sides, hints);
    let mut wires: Vec<RoutedWire> = reqs
        .iter()
        .zip(&plans)
        .map(|(req, plan)| proxy_wire(req, super::route_one(req, plan, nodes)))
        .collect();
    super::nudge::nudge_with(&mut wires, scenes, false);
    score(&wires, nodes)
}

/// A minimal `RoutedWire` carrying exactly what `score` reads: the path, the two
/// endpoint ids, the fan ids, and the wire's clearance. Markers / labels / a11y ids
/// don't affect the score, so they're left empty.
fn proxy_wire(req: &SegReq, path: Vec<Pt>) -> RoutedWire {
    let mut attrs = AttrMap::new();
    attrs.insert("clearance", ResolvedValue::Number(req.clearance));
    RoutedWire {
        path,
        markers: Markers::default(),
        attrs,
        texts: Vec::new(),
        data_from: String::new(),
        data_to: String::new(),
        seg_from: req.a_node.clone(),
        seg_to: req.b_node.clone(),
        decl_span: Span::empty(),
        fan_from: req.fan_a,
        fan_to: req.fan_b,
    }
}

const ALL_SIDES: [Side; 4] = [Side::Top, Side::Right, Side::Bottom, Side::Left];

/// Move a whole unit (a node-group or a fan trunk) onto each candidate side at once,
/// keeping any strict improvement; returns whether it improved. A forced end can't
/// leave its side, so a unit is skipped for any side that would violate one.
fn try_unit(
    cx: &mut Search,
    sides: &mut Vec<(Side, Side)>,
    best: &mut Score,
    unit: &[(usize, bool)],
    candidate_sides: &[Side],
) -> bool {
    let mut improved = false;
    for &side in candidate_sides {
        if cx.exhausted() {
            break;
        }
        let mut trial = sides.clone();
        let mut ok = true;
        for &(s, is_a) in unit {
            let forced = if is_a {
                cx.reqs[s].forced_a
            } else {
                cx.reqs[s].forced_b
            };
            if forced.is_some_and(|f| f != side) {
                ok = false;
                break;
            }
            set_side(&mut trial, s, is_a, side);
        }
        if ok {
            let q = cx.proxy(&trial);
            if q < *best {
                *sides = trial;
                *best = q;
                improved = true;
            }
        }
    }
    improved
}

/// Fan trunks: the sets of wire-ends sharing a fan-trunk id (E2). A fan's siblings
/// move together so they keep their one shared slot at the hub.
fn fan_trunks(reqs: &[SegReq]) -> Vec<Vec<(usize, bool)>> {
    let mut by_id: BTreeMap<u32, Vec<(usize, bool)>> = BTreeMap::new();
    for (s, r) in reqs.iter().enumerate() {
        if let Some(f) = r.fan_a {
            by_id.entry(f).or_default().push((s, true));
        }
        if let Some(f) = r.fan_b {
            by_id.entry(f).or_default().push((s, false));
        }
    }
    by_id.into_values().filter(|t| t.len() >= 2).collect()
}

/// The movable wire-ends grouped by the endpoint node they meet at (≥2 per group).
/// Forced and fan-trunk ends are excluded — they can't move.
fn node_groups(reqs: &[SegReq]) -> Vec<Vec<(usize, bool)>> {
    let mut by_node: BTreeMap<&str, Vec<(usize, bool)>> = BTreeMap::new();
    for (s, r) in reqs.iter().enumerate() {
        if r.a_node == r.b_node {
            continue;
        }
        if r.forced_a.is_none() && r.fan_a.is_none() {
            by_node.entry(&r.a_node).or_default().push((s, true));
        }
        if r.forced_b.is_none() && r.fan_b.is_none() {
            by_node.entry(&r.b_node).or_default().push((s, false));
        }
    }
    by_node.into_values().filter(|g| g.len() >= 2).collect()
}

/// The distinct sides a group's ends seed onto — the natural unify targets, sorted.
fn group_sides(seed: &[(Side, Side)], group: &[(usize, bool)]) -> Vec<Side> {
    let mut sides: Vec<Side> = group
        .iter()
        .map(|&(s, is_a)| side_at(seed, s, is_a))
        .collect();
    sides.sort_by_key(|&s| side_rank(s));
    sides.dedup();
    sides
}

fn side_at(sides: &[(Side, Side)], s: usize, is_a: bool) -> Side {
    if is_a {
        sides[s].0
    } else {
        sides[s].1
    }
}

fn set_side(sides: &mut [(Side, Side)], s: usize, is_a: bool, side: Side) {
    if is_a {
        sides[s].0 = side;
    } else {
        sides[s].1 = side;
    }
}

/// A stable total order over sides for deterministic candidate iteration.
fn side_rank(s: Side) -> u8 {
    match s {
        Side::Top => 0,
        Side::Right => 1,
        Side::Bottom => 2,
        Side::Left => 3,
    }
}

#[cfg(test)]
mod tests {
    use super::super::build_reqs_for_test;
    use super::super::geometry::Rect;
    use super::*;

    fn scene(src: &str) -> (Vec<SegReq>, Vec<PlacedNode>) {
        let toks = crate::lexer::lex(src).expect("lex");
        let file = crate::parser::parse(&toks).expect("parse");
        let prog = crate::resolve::resolve(file).expect("resolve");
        let ns = crate::layout::layout(&prog).expect("layout").nodes;
        (build_reqs_for_test(&prog, &ns), ns)
    }

    /// Search a scene and return the chosen sides plus the (proxy) scorecard.
    fn run(src: &str) -> (Vec<(Side, Side)>, Score) {
        let (reqs, ns) = scene(src);
        let seed = seed_sides(&reqs);
        let sides = search(&reqs, &seed, &[], &ns);
        let template: Vec<RoutedWire> = reqs.iter().map(|r| proxy_wire(r, Vec::new())).collect();
        let scenes = super::super::nudge::build_scenes(&template, &ns);
        let q = proxy(&reqs, &sides, &[], &ns, &scenes);
        (sides, q)
    }

    fn req(a_node: &str, a: Rect, b_node: &str, b: Rect) -> SegReq {
        SegReq {
            a_node: a_node.into(),
            a,
            forced_a: None,
            b_node: b_node.into(),
            b,
            forced_b: None,
            clearance: 16.0,
            fan_a: None,
            fan_b: None,
        }
    }

    #[test]
    fn candidates_exclude_the_back_side_and_pin_forced_and_fan() {
        let mut r = req(
            "a",
            Rect {
                min_x: 0.0,
                min_y: 0.0,
                max_x: 10.0,
                max_y: 10.0,
            },
            "b",
            Rect {
                min_x: 100.0,
                min_y: 0.0,
                max_x: 110.0,
                max_y: 10.0,
            },
        );
        // Facing Right, back closed → {Top, Right, Bottom}, never Left (the back).
        let c = candidates(&r, true, Side::Right, false);
        assert!(!c.contains(&Side::Left), "back side excluded: {c:?}");
        assert_eq!(c.len(), 3);
        // Back open (round 2) → all four sides are candidates.
        assert_eq!(candidates(&r, true, Side::Right, true).len(), 4);

        r.forced_a = Some(Side::Top);
        assert_eq!(
            candidates(&r, true, Side::Right, true),
            vec![Side::Top],
            "forced pins"
        );
        r.forced_a = None;
        r.fan_a = Some(1);
        assert_eq!(
            candidates(&r, true, Side::Right, true),
            vec![Side::Right],
            "a fan trunk pins to its seed side"
        );
    }

    #[test]
    fn search_keeps_a_clean_facing_pair() {
        // aa -> bb straight across: the seed is already optimal, the search a no-op.
        let src = "{ |scene| layout:row gap:80 }\n\
                   aa |rect| size:(40,40)\n\
                   bb |rect| size:(40,40)\n\
                   aa -> bb\n";
        let (sides, q) = run(src);
        assert_eq!(sides[0], (Side::Right, Side::Left), "facing sides kept");
        assert_eq!(
            (q.0, q.1, q.2, q.3, q.4),
            (0, 0, 0, 0, 0),
            "clean: no invariant / B / crossing"
        );
    }

    #[test]
    fn search_routes_a_four_way_hub_cleanly() {
        // One hub wired to neighbours on all four sides — the search must place each
        // spoke without an invariant break, node overlap, or crossing.
        let src = "{ |scene| }\n\
                   hub   |rect| size:(40,40) at:(200,200)\n\
                   north |rect| size:(40,40) at:(200,0)\n\
                   south |rect| size:(40,40) at:(200,400)\n\
                   east  |rect| size:(40,40) at:(400,200)\n\
                   west  |rect| size:(40,40) at:(0,200)\n\
                   hub -> north\n\
                   hub -> south\n\
                   hub -> east\n\
                   hub -> west\n";
        let (_, q) = run(src);
        assert_eq!(
            (q.0, q.1, q.4),
            (0, 0, 0),
            "hub spokes: no invariant, no overlap, no crossing"
        );
    }
}
