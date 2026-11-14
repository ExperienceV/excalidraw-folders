//! Point-versus-element hit testing.

use kurbo::{BezPath, PathEl, Point, Shape};

use crate::draw::Color;
use crate::element::{Element, Kind};
use crate::shape::{is_closed, outline};

fn segment_distance(p: Point, a: Point, b: Point) -> f64 {
    let ab = b - a;
    let len_sq = ab.hypot2();
    let t = if len_sq == 0.0 { 0.0 } else { ((p - a).dot(ab) / len_sq).clamp(0.0, 1.0) };
    p.distance(a + ab * t)
}

/// Shortest distance from `p` to the path's outline.
pub fn distance_to_path(path: &BezPath, p: Point) -> f64 {
    let mut best = f64::INFINITY;
    let (mut start, mut last) = (Point::ZERO, Point::ZERO);
    kurbo::flatten(path, 0.25, |el| match el {
        PathEl::MoveTo(q) => {
            start = q;
            last = q;
            best = best.min(p.distance(q));
        }
        PathEl::LineTo(q) => {
            best = best.min(segment_distance(p, last, q));
            last = q;
        }
        PathEl::ClosePath => {
            best = best.min(segment_distance(p, last, start));
            last = start;
        }
        _ => {}
    });
    best
}

/// Whether the scene point `p` touches the element, within `tolerance`
/// scene units of its stroke. Filled shapes are also hit on their interior.
pub fn hit(el: &Element, p: Point, tolerance: f64) -> bool {
    if el.is_deleted {
        return false;
    }
    let half_stroke = match el.kind {
        Kind::Freedraw => el.stroke_width * 2.125,
        _ => el.stroke_width / 2.0,
    };
    let reach = tolerance + half_stroke;
    let local = el.transform().inverse() * p;
    let bounds = el.local_bounds();
    if !bounds.inflate(reach, reach).contains(local) {
        return false;
    }
    if matches!(el.kind, Kind::Text | Kind::Image | Kind::Other(_)) {
        return true;
    }
    let path = outline(el);
    let filled = !Color::parse(&el.background_color).is_some_and(Color::is_transparent)
        && (!el.is_linear() || is_closed(el));
    (filled && path.contains(local)) || distance_to_path(&path, local) <= reach
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f64::consts::FRAC_PI_2;

    fn rect() -> Element {
        Element { x: 100.0, y: 100.0, width: 200.0, height: 50.0, ..Element::default() }
    }

    #[test]
    fn hollow_shape_hits_only_on_its_stroke() {
        let e = rect();
        assert!(hit(&e, Point::new(100.0, 120.0), 4.0));
        assert!(hit(&e, Point::new(250.0, 152.0), 4.0));
        assert!(!hit(&e, Point::new(200.0, 125.0), 4.0));
        assert!(!hit(&e, Point::new(90.0, 125.0), 4.0));
    }

    #[test]
    fn filled_shape_hits_inside() {
        let e = Element { background_color: "#ffc9c9".into(), ..rect() };
        assert!(hit(&e, Point::new(200.0, 125.0), 4.0));
    }

    #[test]
    fn rotation_is_taken_into_account() {
        // Rotated 90° about its center (200, 125) the box spans x 175..225, y 25..225.
        let e = Element { angle: FRAC_PI_2, ..rect() };
        assert!(hit(&e, Point::new(200.0, 25.0), 4.0));
        assert!(!hit(&e, Point::new(100.0, 120.0), 4.0));
    }

    #[test]
    fn lines_hit_along_their_length() {
        let e = Element {
            kind: Kind::Arrow,
            x: 10.0,
            y: 10.0,
            points: vec![[0.0, 0.0], [100.0, 100.0]],
            ..Element::default()
        };
        assert!(hit(&e, Point::new(60.0, 61.0), 4.0));
        assert!(!hit(&e, Point::new(60.0, 30.0), 4.0));
    }
}
