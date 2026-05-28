//! Channel discovery for the channel-based router.
//!
//! Two primitives drive every routing decision:
//!
//! * `clear_y_intervals` — given an x-span and a list of obstacles
//!   (already inflated by `gap`), return the set of y intervals where a
//!   horizontal segment spanning that x-range would stay clear.
//! * `clear_x_intervals` — same idea for vertical segments.
//!
//! Obstacles are passed in as their gap-inflated bboxes, so "clear"
//! already implies the required wire-to-shape clearance.

use super::geometry::AbsBbox;

#[derive(Debug, Clone, Copy)]
pub struct Interval {
    pub min: f64,
    pub max: f64,
}

impl Interval {
    pub fn mid(&self) -> f64 {
        (self.min + self.max) / 2.0
    }
    pub fn contains(&self, v: f64) -> bool {
        self.min <= v && v <= self.max
    }
}

/// Y-intervals where a horizontal segment spanning `[x_min, x_max]` is
/// clear of every obstacle's y-extent (obstacles are pre-inflated by gap).
/// The result is bounded by `(world_y_min, world_y_max)` so it's a finite
/// list, not unbounded ranges.
pub fn clear_y_intervals(
    x_min: f64,
    x_max: f64,
    obstacles: &[AbsBbox],
    world_y_min: f64,
    world_y_max: f64,
) -> Vec<Interval> {
    let mut blocked: Vec<Interval> = obstacles
        .iter()
        .filter(|o| o.right() > x_min && o.x < x_max)
        .map(|o| Interval {
            min: o.y,
            max: o.bottom(),
        })
        .collect();
    invert_intervals(&mut blocked, world_y_min, world_y_max)
}

/// X-intervals where a vertical segment spanning `[y_min, y_max]` is
/// clear of every obstacle's x-extent (obstacles are pre-inflated by gap).
pub fn clear_x_intervals(
    y_min: f64,
    y_max: f64,
    obstacles: &[AbsBbox],
    world_x_min: f64,
    world_x_max: f64,
) -> Vec<Interval> {
    let mut blocked: Vec<Interval> = obstacles
        .iter()
        .filter(|o| o.bottom() > y_min && o.y < y_max)
        .map(|o| Interval {
            min: o.x,
            max: o.right(),
        })
        .collect();
    invert_intervals(&mut blocked, world_x_min, world_x_max)
}

/// Sort + merge a list of `[min, max]` intervals into the disjoint set
/// covering their union, then invert against `[bound_min, bound_max]`
/// to return the free intervals.
fn invert_intervals(blocked: &mut [Interval], bound_min: f64, bound_max: f64) -> Vec<Interval> {
    if blocked.is_empty() {
        return vec![Interval {
            min: bound_min,
            max: bound_max,
        }];
    }
    blocked.sort_by(|a, b| {
        a.min
            .partial_cmp(&b.min)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    // Merge overlapping/touching blocked intervals in place.
    let mut merged: Vec<Interval> = Vec::with_capacity(blocked.len());
    for iv in blocked.iter().copied() {
        match merged.last_mut() {
            Some(last) if iv.min <= last.max => {
                if iv.max > last.max {
                    last.max = iv.max;
                }
            }
            _ => merged.push(iv),
        }
    }

    // Invert against [bound_min, bound_max].
    let mut out: Vec<Interval> = Vec::with_capacity(merged.len() + 1);
    let mut cursor = bound_min;
    for b in &merged {
        if b.min > cursor {
            out.push(Interval {
                min: cursor,
                max: b.min,
            });
        }
        cursor = cursor.max(b.max);
    }
    if cursor < bound_max {
        out.push(Interval {
            min: cursor,
            max: bound_max,
        });
    }
    out
}

/// Pick the interval whose midline lies closest to `target`. If `target`
/// falls inside an interval, return that interval (and its midline value
/// will be the natural snap point). Returns `None` only when the list is
/// empty.
pub fn nearest_interval(intervals: &[Interval], target: f64) -> Option<Interval> {
    intervals.iter().copied().min_by(|a, b| {
        let da = if a.contains(target) {
            0.0
        } else {
            (a.mid() - target).abs()
        };
        let db = if b.contains(target) {
            0.0
        } else {
            (b.mid() - target).abs()
        };
        da.partial_cmp(&db).unwrap_or(std::cmp::Ordering::Equal)
    })
}

/// True if a horizontal segment at world-y `y`, spanning `[x_min, x_max]`,
/// would clear every obstacle.
pub fn row_clear(y: f64, x_min: f64, x_max: f64, obstacles: &[AbsBbox]) -> bool {
    !obstacles
        .iter()
        .any(|o| o.y < y && y < o.bottom() && o.right() > x_min && o.x < x_max)
}

/// True if a vertical segment at world-x `x`, spanning `[y_min, y_max]`,
/// would clear every obstacle.
pub fn column_clear(x: f64, y_min: f64, y_max: f64, obstacles: &[AbsBbox]) -> bool {
    !obstacles
        .iter()
        .any(|o| o.x < x && x < o.right() && o.bottom() > y_min && o.y < y_max)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn b(x: f64, y: f64, w: f64, h: f64) -> AbsBbox {
        AbsBbox { x, y, w, h }
    }

    #[test]
    fn no_obstacles_returns_full_bound() {
        let r = clear_y_intervals(0.0, 100.0, &[], -1000.0, 1000.0);
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].min, -1000.0);
        assert_eq!(r[0].max, 1000.0);
    }

    #[test]
    fn obstacle_x_outside_query_is_ignored() {
        let obs = [b(200.0, 0.0, 50.0, 50.0)];
        let r = clear_y_intervals(0.0, 100.0, &obs, -1000.0, 1000.0);
        assert_eq!(r.len(), 1);
    }

    #[test]
    fn obstacle_splits_free_intervals() {
        let obs = [b(0.0, 50.0, 100.0, 30.0)]; // blocks y in [50, 80]
        let r = clear_y_intervals(20.0, 80.0, &obs, -100.0, 200.0);
        assert_eq!(r.len(), 2);
        assert_eq!(r[0].max, 50.0);
        assert_eq!(r[1].min, 80.0);
    }

    #[test]
    fn overlapping_obstacles_merge() {
        let obs = [b(0.0, 50.0, 100.0, 30.0), b(0.0, 70.0, 100.0, 30.0)];
        let r = clear_y_intervals(20.0, 80.0, &obs, -100.0, 200.0);
        assert_eq!(r.len(), 2);
        assert_eq!(r[0].max, 50.0);
        assert_eq!(r[1].min, 100.0);
    }

    #[test]
    fn row_clear_detects_obstacle() {
        let obs = [b(0.0, 50.0, 100.0, 30.0)];
        assert!(!row_clear(60.0, 20.0, 80.0, &obs));
        assert!(row_clear(40.0, 20.0, 80.0, &obs));
        assert!(row_clear(60.0, 200.0, 300.0, &obs));
    }
}
