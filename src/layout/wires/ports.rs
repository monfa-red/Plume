//! Port planning — which side of each endpoint a wire leaves/enters, and *where*
//! on that side it attaches (WIRING section C).
//!
//! A wire no longer just meets the side midpoint: wires sharing a side are spread
//! across uniform **slots** (C2), ordered so they don't needlessly cross (C4); a
//! side with a single wire may slide its port to kill a bend (C3). Side choice is
//! the forced side, else geometry, with a least-loaded tie-break (C1).

use super::geometry::{pick_edges, Pt, Rect};
use crate::ast::Side;
use std::collections::BTreeMap;

/// One wire-segment's port request: the two node boxes, any forced sides, and the
/// wire's clearance (which sets slot spacing on a shared side).
pub struct SegReq {
    pub a_node: String,
    pub a: Rect,
    pub forced_a: Option<Side>,
    pub b_node: String,
    pub b: Rect,
    pub forced_b: Option<Side>,
    pub clearance: f64,
    /// Fan-trunk group id per end (source, target): siblings sharing an id share
    /// one slot at that end (E2 / C2). `None` for an ordinary end.
    pub fan_a: Option<u32>,
    pub fan_b: Option<u32>,
}

/// The planned attachment for one segment: a side and a concrete port point at
/// each end.
#[derive(Clone, Copy)]
pub struct Plan {
    pub side_a: Side,
    pub side_b: Side,
    pub port_a: Pt,
    pub port_b: Pt,
}

/// Feedback from a provisional routing pass (the second pass of the router's
/// two-pass, WIRING appendix). Side selection and slot ordering otherwise run
/// blind — guessing from straight lines to target centres — which mis-orders a
/// wire that detours around an obstacle and picks a side that forces it through a
/// sub-clearance gap. These hints replace the guess with where each end's wire
/// *actually* went: `lead_*` is the point the wire heads to just past its stub
/// (the crossing-free C4 order), and `reside_*` is the side an end should move to
/// when its provisional route skimmed its own node (obstacle-aware C1).
#[derive(Clone, Copy, Default)]
pub struct PlanHint {
    pub lead_a: Option<Pt>,
    pub lead_b: Option<Pt>,
    pub reside_a: Option<Side>,
    pub reside_b: Option<Side>,
}

/// Plan every segment's ports together, so wires sharing a side can be spread
/// into uniform slots. `reqs` is in routing order (declaration, then chain); that
/// order is the deterministic tie-break for slot ordering. `hints` is empty on the
/// first pass and carries the provisional-route feedback on the second.
pub fn plan(reqs: &[SegReq], hints: &[PlanHint]) -> Vec<Plan> {
    let sides = pick_all_sides(reqs, hints);
    let mut plans: Vec<Plan> = reqs
        .iter()
        .zip(&sides)
        .map(|(r, &(sa, sb))| Plan {
            side_a: sa,
            side_b: sb,
            port_a: r.a.port(sa), // a lone wire keeps the midpoint; slots overwrite below
            port_b: r.b.port(sb),
        })
        .collect();

    let groups = group_by_side(reqs, &sides);
    for ends in groups.values() {
        assign_slots(reqs, ends, hints, &mut plans);
    }

    // C3 — a lone wire on a facing pair of sides slides its port(s) to line up
    // with the far end, trading a two-bend jog for a straight shot. Multi-wire
    // sides keep their slots untouched.
    let count = |node: &str, side: Side| groups.get(&(node, side_ord(side))).map_or(0, Vec::len);
    for (s, r) in reqs.iter().enumerate() {
        let (sa, sb) = sides[s];
        if !facing(sa, sb) {
            continue;
        }
        // A straight shot only exists when the two boxes overlap on the axis the
        // ports slide along; otherwise aligning is impossible and sliding just
        // shoves the ports to the corners for no bend saved. C3 is "kill a bend",
        // not "chase an unreachable straight" (e.g. `dog.b -> bird.t` side by side).
        let aligned_axis_overlaps = if varies_in_y(sa) {
            r.a.min_y < r.b.max_y && r.b.min_y < r.a.max_y
        } else {
            r.a.min_x < r.b.max_x && r.b.min_x < r.a.max_x
        };
        if !aligned_axis_overlaps {
            continue;
        }
        if count(&r.a_node, sa) == 1 {
            plans[s].port_a = slide(r.a, sa, axis_coord(plans[s].port_b, sa), r.clearance);
        }
        if count(&r.b_node, sb) == 1 {
            plans[s].port_b = slide(r.b, sb, axis_coord(plans[s].port_a, sb), r.clearance);
        }
    }
    plans
}

/// Are these two sides a facing pair (opposite, same varying axis)? Only then can
/// sliding a port turn a jog into a straight wire.
fn facing(a: Side, b: Side) -> bool {
    use Side::*;
    matches!(
        (a, b),
        (Right, Left) | (Left, Right) | (Top, Bottom) | (Bottom, Top)
    )
}

/// A port's coordinate along `side`'s varying axis.
fn axis_coord(p: Pt, side: Side) -> f64 {
    if varies_in_y(side) {
        p.1
    } else {
        p.0
    }
}

/// Slide a lone port toward `target`, clamped to the side's usable span (≥ corner
/// inset from each corner); degenerate spans collapse to the centre.
fn slide(r: Rect, side: Side, target: f64, clearance: f64) -> Pt {
    let (lo, hi) = side_span(r, side);
    let c = if lo + clearance <= hi - clearance {
        target.clamp(lo + clearance, hi - clearance)
    } else {
        (lo + hi) / 2.0
    };
    port_at(r, side, c)
}

/// One wire-end attached to some side: which segment, and which of its two ends.
struct End {
    seg: usize,
    is_a: bool,
}

impl End {
    fn side(&self, plans: &[Plan]) -> Side {
        if self.is_a {
            plans[self.seg].side_a
        } else {
            plans[self.seg].side_b
        }
    }

    fn rect(&self, reqs: &[SegReq]) -> Rect {
        if self.is_a {
            reqs[self.seg].a
        } else {
            reqs[self.seg].b
        }
    }

    /// This end's fan-trunk id, if it shares a trunk with siblings (E2).
    fn fan(&self, reqs: &[SegReq]) -> Option<u32> {
        if self.is_a {
            reqs[self.seg].fan_a
        } else {
            reqs[self.seg].fan_b
        }
    }

    /// The coordinate this end's wire heads to, projected onto the side's varying
    /// axis — the key that orders wires so they fan out without crossing (C4).
    /// With a provisional-route hint it is where the wire *actually* went (so a
    /// detour around an obstacle sorts correctly); otherwise it falls back to the
    /// straight-line aim at the opposite node's centre.
    fn order_key(&self, reqs: &[SegReq], hints: &[PlanHint], side: Side) -> f64 {
        let lead = hints
            .get(self.seg)
            .and_then(|h| if self.is_a { h.lead_a } else { h.lead_b });
        match lead {
            Some(p) => axis_coord(p, side),
            None => {
                let other = if self.is_a {
                    reqs[self.seg].b
                } else {
                    reqs[self.seg].a
                };
                let (cx, cy) = other.center();
                if varies_in_y(side) {
                    cy
                } else {
                    cx
                }
            }
        }
    }
}

/// Bucket every end by the `(node, side)` it lands on, deterministically (sorted
/// keys), so wires meeting on one side are planned as a group.
fn group_by_side<'a>(
    reqs: &'a [SegReq],
    sides: &[(Side, Side)],
) -> BTreeMap<(&'a str, u8), Vec<End>> {
    let mut groups: BTreeMap<(&str, u8), Vec<End>> = BTreeMap::new();
    for (s, (r, &(sa, sb))) in reqs.iter().zip(sides).enumerate() {
        groups
            .entry((&r.a_node, side_ord(sa)))
            .or_default()
            .push(End { seg: s, is_a: true });
        groups
            .entry((&r.b_node, side_ord(sb)))
            .or_default()
            .push(End {
                seg: s,
                is_a: false,
            });
    }
    groups
}

/// C2/C4 — place a side's wires on uniform slots, symmetric about its centre and
/// `separation` apart (compacting to fit, C5), ordered so they don't cross (C4).
/// A side with a single wire is left at its midpoint (C3 may move it later).
fn assign_slots(reqs: &[SegReq], ends: &[End], hints: &[PlanHint], plans: &mut [Plan]) {
    // Collapse fan siblings into one occupant: a fan group's shared end is a
    // single slot (C2 / E2). Every other end is its own occupant.
    let mut occupants: Vec<Vec<&End>> = Vec::new();
    let mut by_fan: BTreeMap<u32, usize> = BTreeMap::new();
    for e in ends {
        match e.fan(reqs) {
            Some(f) if by_fan.contains_key(&f) => occupants[by_fan[&f]].push(e),
            Some(f) => {
                by_fan.insert(f, occupants.len());
                occupants.push(vec![e]);
            }
            None => occupants.push(vec![e]),
        }
    }
    if occupants.len() < 2 {
        return; // one occupant (a lone wire or a single fan trunk) keeps the midpoint
    }

    let side = ends[0].side(plans);
    let (lo, hi) = side_span(ends[0].rect(reqs), side);
    let centre = (lo + hi) / 2.0;

    // C4 — order occupants by where they head (a fan trunk by its members' mean),
    // breaking ties by routing order.
    let key = |occ: &[&End]| {
        occ.iter()
            .map(|e| e.order_key(reqs, hints, side))
            .sum::<f64>()
            / occ.len() as f64
    };
    let first_seg = |occ: &[&End]| occ.iter().map(|e| e.seg).min().unwrap();
    let mut order: Vec<&Vec<&End>> = occupants.iter().collect();
    order.sort_by(|x, y| {
        key(x)
            .total_cmp(&key(y))
            .then(first_seg(x).cmp(&first_seg(y)))
    });

    // Uniform spacing: the target `separation` when it fits, else the largest that
    // lets the k wires AND the two corner insets split the side evenly (C2/C5). The
    // wires are centred and `spacing` apart, so the inset is `(span-(k-1)·spacing)/2`;
    // with `spacing = span/(k+1)` that inset equals the spacing, so under overflow the
    // corner inset shrinks in lockstep — wires never bunch to a point nor crowd the
    // corners (the even split the user expects when `clearance` is cranked up).
    let k = order.len();
    let sep = ends
        .iter()
        .map(|e| reqs[e.seg].clearance)
        .fold(0.0_f64, f64::max);
    let spacing = sep.min((hi - lo) / (k as f64 + 1.0));

    for (rank, occ) in order.iter().enumerate() {
        let offset = (rank as f64 - (k as f64 - 1.0) / 2.0) * spacing;
        for e in occ.iter() {
            let port = port_at(e.rect(reqs), side, centre + offset);
            if e.is_a {
                plans[e.seg].port_a = port;
            } else {
                plans[e.seg].port_b = port;
            }
        }
    }
}

/// Does a wire on this side vary its port along the y axis (left/right sides) or
/// the x axis (top/bottom)?
fn varies_in_y(side: Side) -> bool {
    matches!(side, Side::Left | Side::Right)
}

/// The `(lo, hi)` extent of the coordinate a port slides along on this side.
fn side_span(r: Rect, side: Side) -> (f64, f64) {
    if varies_in_y(side) {
        (r.min_y, r.max_y)
    } else {
        (r.min_x, r.max_x)
    }
}

/// The port point on `side` of `r` at varying-axis coordinate `c`.
fn port_at(r: Rect, side: Side, c: f64) -> Pt {
    match side {
        Side::Left => (r.min_x, c),
        Side::Right => (r.max_x, c),
        Side::Top => (c, r.min_y),
        Side::Bottom => (c, r.max_y),
    }
}

fn side_ord(side: Side) -> u8 {
    match side {
        Side::Top => 0,
        Side::Right => 1,
        Side::Bottom => 2,
        Side::Left => 3,
    }
}

/// WIRING C1 tie-break order when sides are equally good: right → bottom → left → top.
fn side_pref(side: Side) -> u8 {
    match side {
        Side::Right => 0,
        Side::Bottom => 1,
        Side::Left => 2,
        Side::Top => 3,
    }
}

/// C1 — the side each end attaches to: a forced side wins; else the geometric
/// pick, with a least-loaded tie-break for diagonal hops that could leave off
/// either of two sides.
fn pick_all_sides(reqs: &[SegReq], hints: &[PlanHint]) -> Vec<(Side, Side)> {
    // The side each end attaches to, most-authoritative first: a forced side, then
    // a re-elected side (an end whose provisional route skimmed its own node moves
    // to the side it actually headed toward — obstacle-aware C1), then the
    // geometric pick. A self-loop has no geometry: it defaults to right→top (E3).
    let pick = |r: &SegReq, s: usize| -> (Side, Side) {
        let h = hints.get(s).copied().unwrap_or_default();
        if r.a_node == r.b_node {
            return (
                r.forced_a.unwrap_or(Side::Right),
                r.forced_b.unwrap_or(Side::Top),
            );
        }
        let (ga, gb) = pick_edges(r.a, r.b);
        (
            r.forced_a.or(h.reside_a).unwrap_or(ga),
            r.forced_b.or(h.reside_b).unwrap_or(gb),
        )
    };
    let mut sides: Vec<(Side, Side)> = reqs.iter().enumerate().map(|(s, r)| pick(r, s)).collect();

    let mut load: BTreeMap<(&str, u8), usize> = BTreeMap::new();
    for (r, &(sa, sb)) in reqs.iter().zip(&sides) {
        *load.entry((&r.a_node, side_ord(sa))).or_default() += 1;
        *load.entry((&r.b_node, side_ord(sb))).or_default() += 1;
    }

    // Rebalance only the genuinely ambiguous (diagonal) ends — never a forced or
    // re-elected one, whose side is already decided.
    for (s, r) in reqs.iter().enumerate() {
        let h = hints.get(s).copied().unwrap_or_default();
        if r.forced_a.is_none() && h.reside_a.is_none() {
            sides[s].0 = least_loaded(&r.a_node, r.a, r.b, sides[s].0, &mut load);
        }
        if r.forced_b.is_none() && h.reside_b.is_none() {
            sides[s].1 = least_loaded(&r.b_node, r.b, r.a, sides[s].1, &mut load);
        }
    }
    sides
}

/// A diagonal hop (boxes sharing neither an x- nor a y-overlap) can leave off
/// either the horizontal or the vertical side for one bend; pick the less-loaded
/// (ties: right→bottom→left→top), updating `load`. Non-diagonal ends keep their
/// clear facing side.
fn least_loaded<'a>(
    node: &'a str,
    from: Rect,
    to: Rect,
    current: Side,
    load: &mut BTreeMap<(&'a str, u8), usize>,
) -> Side {
    let (fc, tc) = (from.center(), to.center());
    let y_overlap = from.min_y < to.max_y && to.min_y < from.max_y;
    let x_overlap = from.min_x < to.max_x && to.min_x < from.max_x;
    if y_overlap || x_overlap {
        return current; // a clear facing side, not a corner choice
    }
    let horizontal = if tc.0 >= fc.0 {
        Side::Right
    } else {
        Side::Left
    };
    let vertical = if tc.1 >= fc.1 {
        Side::Bottom
    } else {
        Side::Top
    };
    let load_of = |s: Side| {
        let base = load.get(&(node, side_ord(s))).copied().unwrap_or(0);
        base - usize::from(s == current) // discount this end from its own side
    };
    let pick = [horizontal, vertical]
        .into_iter()
        .min_by_key(|&s| (load_of(s), side_pref(s)))
        .unwrap();
    if pick != current {
        *load.get_mut(&(node, side_ord(current))).unwrap() -= 1;
        *load.entry((node, side_ord(pick))).or_default() += 1;
    }
    pick
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(min_x: f64, min_y: f64, max_x: f64, max_y: f64) -> Rect {
        Rect {
            min_x,
            min_y,
            max_x,
            max_y,
        }
    }

    fn seg(a_node: &str, a: Rect, b_node: &str, b: Rect) -> SegReq {
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
    fn shared_side_spreads_into_even_centred_slots() {
        // Three wires leave src's right side for three stacked targets — they must
        // take three distinct, evenly-spaced slots centred on the side, not all
        // pile onto the midpoint.
        let src = rect(0.0, 0.0, 40.0, 160.0); // tall; centre y = 80
        let t = |y: f64| rect(200.0, y, 240.0, y + 40.0);
        let reqs = vec![
            seg("src", src, "a", t(0.0)),
            seg("src", src, "b", t(60.0)),
            seg("src", src, "c", t(120.0)),
        ];
        let plans = plan(&reqs, &[]);

        assert!(
            plans.iter().all(|p| (p.port_a.0 - 40.0).abs() < 1e-9),
            "all leave src's right edge"
        );
        let mut ys: Vec<f64> = plans.iter().map(|p| p.port_a.1).collect();
        ys.sort_by(f64::total_cmp);
        assert!(
            ys[0] < ys[1] && ys[1] < ys[2],
            "three distinct slots, got {ys:?}"
        );
        assert!(
            ((ys[0] + ys[2]) / 2.0 - 80.0).abs() < 1e-9,
            "centred on the side"
        );
        assert!(
            ((ys[1] - ys[0]) - (ys[2] - ys[1])).abs() < 1e-9,
            "uniform spacing"
        );
    }

    #[test]
    fn slots_are_ordered_by_target_so_wires_do_not_cross() {
        // Two wires leave src's right side, declared in "crossing" order: the
        // first aims low, the second high. C4 must give the higher-aimed wire the
        // higher slot, so they don't cross on the way out.
        let src = rect(0.0, 0.0, 40.0, 120.0);
        let low = rect(200.0, 100.0, 240.0, 140.0); // larger y
        let high = rect(200.0, 0.0, 240.0, 40.0); // smaller y
        let reqs = vec![seg("src", src, "low", low), seg("src", src, "high", high)];
        let plans = plan(&reqs, &[]);
        assert!(
            plans[1].port_a.1 < plans[0].port_a.1,
            "the wire aiming higher (smaller y) takes the higher slot"
        );
    }

    #[test]
    fn overflow_splits_the_side_evenly_including_the_margins() {
        // A side too short to fit 9 wires at separation 20 (it's only 50 tall) packs
        // them evenly: the corner inset shrinks in lockstep with the spacing so the
        // wires AND the two margins split the span equally — inset == spacing ==
        // span/(k+1) = 50/10 = 5. They never bunch to a point nor reach the corners.
        let src = rect(0.0, 0.0, 40.0, 50.0);
        let t = |i: usize| rect(200.0, i as f64 * 12.0, 240.0, i as f64 * 12.0 + 6.0);
        let reqs: Vec<SegReq> = (0..9)
            .map(|i| {
                let mut s = seg("src", src, "t", t(i));
                s.clearance = 20.0;
                s.forced_a = Some(Side::Right); // pin all 9 to the one 50px side
                s
            })
            .collect();
        let plans = plan(&reqs, &[]);
        let mut ys: Vec<f64> = plans.iter().map(|p| p.port_a.1).collect();
        ys.sort_by(f64::total_cmp);
        assert!((ys[0] - 5.0).abs() < 1e-9, "top inset == 5, got {}", ys[0]);
        assert!(
            (ys[8] - 45.0).abs() < 1e-9,
            "bottom inset == 5 (port at 45 on a 50 side), got {}",
            ys[8]
        );
        for w in ys.windows(2) {
            assert!(
                ((w[1] - w[0]) - 5.0).abs() < 1e-9,
                "uniform 5px spacing, got {}",
                w[1] - w[0]
            );
        }
    }

    #[test]
    fn fan_siblings_share_one_slot() {
        // Two wires leave src's right side as a fan group (same fan id) for two
        // stacked targets. C2: a fan group's shared end is ONE slot — both ports
        // land on the same point, not two spread slots.
        let src = rect(0.0, 0.0, 40.0, 160.0);
        let t = |y: f64| rect(200.0, y, 240.0, y + 40.0);
        let mut s0 = seg("src", src, "one", t(0.0));
        let mut s1 = seg("src", src, "two", t(120.0));
        s0.fan_a = Some(7);
        s1.fan_a = Some(7);
        let plans = plan(&[s0, s1], &[]);
        assert_eq!(
            plans[0].port_a, plans[1].port_a,
            "fan siblings share the source port"
        );
    }

    #[test]
    fn a_lead_hint_orders_slots_by_where_the_wire_actually_goes() {
        // Two wires leave src's right. By straight-line aim, s0 (target high) takes
        // the upper slot. But a provisional route shows s0 actually detours DOWN
        // (its lead is low) and s1 heads UP — the hint must flip the slot order so
        // they don't cross (the two-pass C4 fix for obstacle detours).
        let src = rect(0.0, 0.0, 40.0, 160.0);
        let t = |y: f64| rect(200.0, y, 240.0, y + 40.0);
        let reqs = [
            seg("src", src, "hi", t(0.0)),
            seg("src", src, "lo", t(120.0)),
        ];

        let aim = plan(&reqs, &[]);
        assert!(
            aim[0].port_a.1 < aim[1].port_a.1,
            "by aim, the high-target wire takes the upper (smaller-y) slot"
        );

        let hints = [
            PlanHint {
                lead_a: Some((40.0, 200.0)), // s0 really heads DOWN
                ..Default::default()
            },
            PlanHint {
                lead_a: Some((40.0, -40.0)), // s1 really heads UP
                ..Default::default()
            },
        ];
        let led = plan(&reqs, &hints);
        assert!(
            led[0].port_a.1 > led[1].port_a.1,
            "the lead hint orders by real heading, flipping the slots"
        );
    }

    #[test]
    fn a_reside_hint_re_elects_the_side() {
        // A level a→b wire takes b's facing side by geometry; a reside hint (its
        // provisional route skimmed an endpoint) moves it to the named side (C1
        // obstacle-aware re-election). Forced sides still win over a hint.
        let a = rect(0.0, 0.0, 40.0, 40.0);
        let b = rect(160.0, 0.0, 200.0, 40.0);
        let reqs = [seg("a", a, "bb", b)];
        assert_eq!(
            plan(&reqs, &[])[0].side_a,
            Side::Right,
            "geometry picks right"
        );

        let hints = [PlanHint {
            reside_a: Some(Side::Bottom),
            ..Default::default()
        }];
        assert_eq!(
            plan(&reqs, &hints)[0].side_a,
            Side::Bottom,
            "a reside hint re-elects the side"
        );
    }

    #[test]
    fn a_fan_trunk_and_a_plain_wire_take_separate_slots() {
        // src's right hosts a 2-sibling fan (one slot) plus an unrelated wire.
        // That's two occupants → two distinct slots; the siblings still coincide.
        let src = rect(0.0, 0.0, 40.0, 160.0);
        let t = |y: f64| rect(200.0, y, 240.0, y + 40.0);
        let mut s0 = seg("src", src, "one", t(0.0));
        let mut s1 = seg("src", src, "two", t(60.0));
        let plain = seg("src", src, "three", t(120.0));
        s0.fan_a = Some(3);
        s1.fan_a = Some(3);
        let plans = plan(&[s0, s1, plain], &[]);
        assert_eq!(plans[0].port_a, plans[1].port_a, "siblings coincide");
        assert_ne!(
            plans[0].port_a, plans[2].port_a,
            "the plain wire gets its own slot"
        );
    }

    #[test]
    fn lone_wire_keeps_the_side_midpoint() {
        let a = rect(0.0, 0.0, 40.0, 40.0);
        let b = rect(120.0, 0.0, 160.0, 40.0);
        let plans = plan(&[seg("a", a, "b", b)], &[]);
        assert_eq!(plans[0].port_a, (40.0, 20.0));
        assert_eq!(plans[0].port_b, (120.0, 20.0));
    }

    #[test]
    fn lone_facing_wire_slides_to_a_straight_shot() {
        // cat's right faces dog's left; dog sits lower but their usable spans
        // overlap, so C3 slides both ports to one y → a straight, bend-free wire.
        let cat = rect(0.0, 0.0, 40.0, 80.0);
        let dog = rect(120.0, 40.0, 160.0, 120.0);
        let plans = plan(&[seg("cat", cat, "dog", dog)], &[]);
        assert!(
            (plans[0].port_a.1 - plans[0].port_b.1).abs() < 1e-9,
            "ports aligned to one y → straight, got {:?} / {:?}",
            plans[0].port_a,
            plans[0].port_b
        );
    }

    #[test]
    fn forced_opposite_but_offset_sides_keep_centred_ports() {
        // dog.b -> bird.t: forced Bottom→Top, but the boxes sit side by side (their
        // x-spans don't overlap), so no straight vertical shot exists. C3 must NOT
        // slide the ports toward the corners chasing an impossible straight — they
        // stay at the side midpoints (the wire takes its Z either way).
        let dog = rect(0.0, 0.0, 40.0, 40.0);
        let bird = rect(120.0, 0.0, 160.0, 40.0);
        let mut req = seg("dog", dog, "bird", bird);
        req.forced_a = Some(Side::Bottom);
        req.forced_b = Some(Side::Top);
        let plans = plan(&[req], &[]);
        assert_eq!(plans[0].side_a, Side::Bottom);
        assert_eq!(plans[0].side_b, Side::Top);
        assert_eq!(
            plans[0].port_a,
            (20.0, 40.0),
            "dog's bottom port stays centred"
        );
        assert_eq!(
            plans[0].port_b,
            (140.0, 0.0),
            "bird's top port stays centred"
        );
    }

    #[test]
    fn lone_facing_wire_with_overlapping_span_still_slides_straight() {
        // a stacked above b, a.bottom -> b.top: their x-spans overlap, so a common x
        // gives a straight vertical wire — C3 should still slide to it.
        let a = rect(40.0, 0.0, 120.0, 40.0);
        let b = rect(0.0, 120.0, 80.0, 160.0); // x-spans [40,120] / [0,80] overlap
        let mut req = seg("a", a, "b", b);
        req.forced_a = Some(Side::Bottom);
        req.forced_b = Some(Side::Top);
        let plans = plan(&[req], &[]);
        assert!(
            (plans[0].port_a.0 - plans[0].port_b.0).abs() < 1e-9,
            "ports aligned to one x → straight vertical, got {:?} / {:?}",
            plans[0].port_a,
            plans[0].port_b
        );
    }

    #[test]
    fn diagonal_wire_leaves_the_least_loaded_side() {
        // hub feeds a node straight right (loads its Right side) and a second node
        // diagonally down-right. The diagonal one is reachable off Right or Bottom
        // equally well (C1), so it should pick the less-loaded Bottom.
        let hub = rect(0.0, 0.0, 40.0, 40.0);
        let east = rect(200.0, 10.0, 240.0, 30.0); // level → straight off Right
        let down_right = rect(200.0, 200.0, 240.0, 240.0); // diagonal
        let reqs = vec![
            seg("hub", hub, "east", east),
            seg("hub", hub, "dr", down_right),
        ];
        let plans = plan(&reqs, &[]);
        assert_eq!(
            plans[0].side_a,
            Side::Right,
            "level target → straight right"
        );
        assert_eq!(
            plans[1].side_a,
            Side::Bottom,
            "diagonal target dodges the loaded Right side"
        );
    }
}
