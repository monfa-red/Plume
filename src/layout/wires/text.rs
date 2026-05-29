//! Place a wire's text children along its routed polyline.

use super::geometry::{point_at, Pt};
use crate::layout::ir::RoutedText;
use crate::resolve::{ResolvedText, WireAt};

pub fn place_texts(path: &[Pt], texts: &[ResolvedText]) -> Vec<RoutedText> {
    texts
        .iter()
        .map(|t| {
            let frac = match t.at {
                WireAt::Start => 0.0,
                WireAt::Mid => 0.5,
                WireAt::End => 1.0,
                WireAt::Fraction(f) => f,
            };
            let (position, tangent) = point_at(path, frac);
            RoutedText {
                content: t.text.clone(),
                position,
                tangent,
                attrs: t.attrs.clone(),
            }
        })
        .collect()
}
