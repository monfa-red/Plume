//! Place wire-label texts along the routed polyline at a requested
//! fractional position (`at:start`, `at:mid`, `at:end`, or `at:0.75`).

use crate::layout::ir::RoutedText;
use crate::resolve::{ResolvedText, WireAt};

pub fn place_texts(texts: &[ResolvedText], path: &[(f64, f64)]) -> Vec<RoutedText> {
    let mut out = Vec::with_capacity(texts.len());
    for t in texts {
        let fraction = match &t.at {
            WireAt::Start => 0.0,
            WireAt::Mid => 0.5,
            WireAt::End => 1.0,
            WireAt::Fraction(f) => *f,
        };
        let (pos, tangent) = point_at_fraction(path, fraction);
        out.push(RoutedText {
            content: t.text.clone(),
            position: pos,
            tangent,
            attrs: t.attrs.clone(),
        });
    }
    out
}

fn point_at_fraction(path: &[(f64, f64)], f: f64) -> ((f64, f64), (f64, f64)) {
    if path.is_empty() {
        return ((0.0, 0.0), (1.0, 0.0));
    }
    if path.len() == 1 {
        return (path[0], (1.0, 0.0));
    }
    let total: f64 = path.windows(2).map(|w| dist(w[0], w[1])).sum();
    let target = total * f.clamp(0.0, 1.0);
    let mut acc = 0.0;
    for w in path.windows(2) {
        let seg = dist(w[0], w[1]);
        if acc + seg >= target {
            let local_f = if seg > 0.0 { (target - acc) / seg } else { 0.0 };
            let x = w[0].0 + (w[1].0 - w[0].0) * local_f;
            let y = w[0].1 + (w[1].1 - w[0].1) * local_f;
            let dx = (w[1].0 - w[0].0) / seg.max(1e-9);
            let dy = (w[1].1 - w[0].1) / seg.max(1e-9);
            return ((x, y), (dx, dy));
        }
        acc += seg;
    }
    let last = *path.last().unwrap();
    let prev = path[path.len() - 2];
    let dx = last.0 - prev.0;
    let dy = last.1 - prev.1;
    let len = (dx * dx + dy * dy).sqrt().max(1e-9);
    (last, (dx / len, dy / len))
}

fn dist(a: (f64, f64), b: (f64, f64)) -> f64 {
    ((b.0 - a.0).powi(2) + (b.1 - a.1).powi(2)).sqrt()
}
