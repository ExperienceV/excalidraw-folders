//! Arrows attached to shapes. Ported (simplified) from Excalidraw's
//! `binding.ts`, for straight and curved arrows.
//!
//! A binding stores a *fixed point*: a position inside the shape, as a
//! ratio of its width and height, that the arrow aims at. In `orbit` mode
//! the arrow stops at the shape's outline, a small gap away, on the line
//! toward that point; in `inside` mode it reaches the point itself.

use kurbo::{Point, Shape};
use serde_json::{json, Value};

use crate::element::{Element, Kind};
use crate::hit::distance_to_path;
use crate::shape::{flatten, outline};

const BASE_GAP: f64 = 5.0;
/// Below this length a bound arrow aims at the fixed point instead of the
/// outline, so that it does not flip direction.
const MIN_ARROW_LENGTH: f64 = 10.0;
/// How far from a shape's outline (in scene units at 100% zoom) an arrow
/// end still attaches to it.
const BIND_DISTANCE: f64 = 15.0;
const FIXED_POINT_BOUND: f64 = 10.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum End {
    Start,
    End,
}

impl End {
    pub const BOTH: [End; 2] = [End::Start, End::End];

    fn key(self) -> &'static str {
        match self {
            End::Start => "startBinding",
            End::End => "endBinding",
        }
    }

    pub fn other(self) -> End {
        match self {
            End::Start => End::End,
            End::End => End::Start,
        }
    }

    /// Index of this end's point, and of the point next to it.
    pub fn indices(self, len: usize) -> (usize, usize) {
        match self {
            End::Start => (0, 1.min(len - 1)),
            End::End => (len - 1, len.saturating_sub(2)),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Binding {
    pub element_id: String,
    pub fixed_point: [f64; 2],
    /// `inside` mode: the arrow reaches the fixed point itself.
    pub inside: bool,
}

pub(crate) fn is_bindable(kind: &Kind) -> bool {
    matches!(kind, Kind::Rectangle | Kind::Diamond | Kind::Ellipse | Kind::Text | Kind::Image)
}

pub(crate) fn read(arrow: &Element, end: End) -> Option<Binding> {
    let value = arrow.extra.get(end.key())?;
    let element_id = value["elementId"].as_str()?.to_owned();
    let ratio = |i: usize| value["fixedPoint"][i].as_f64();
    let fixed_point = ratio(0).zip(ratio(1)).map_or([0.5, 0.5], |(x, y)| [x, y]);
    Some(Binding { element_id, fixed_point: normalize(fixed_point), inside: value["mode"] == "inside" })
}

pub(crate) fn write(arrow: &mut Element, end: End, binding: Option<&Binding>) {
    let value = match binding {
        Some(b) => json!({
            "elementId": b.element_id,
            "fixedPoint": b.fixed_point,
            "mode": if b.inside { "inside" } else { "orbit" },
        }),
        None => Value::Null,
    };
    arrow.extra.insert(end.key().to_owned(), value);
}

/// Whether the binding at `end` predates fixed points (it has `focus` and
/// `gap` instead) and needs [`migrate`].
fn is_legacy(arrow: &Element, end: End) -> bool {
    arrow.extra.get(end.key()).is_some_and(|v| v["elementId"].is_string() && !v["mode"].is_string())
}

/// Converts a legacy binding into a fixed-point one that keeps the arrow's
/// end exactly where it is now.
fn migrate(arrow: &Element, end: End, shape: &Element) -> Binding {
    let points = global_points(arrow);
    let at = points[end.indices(points.len()).0];
    Binding { element_id: shape.id.clone(), fixed_point: fixed_point_for(shape, at), inside: contains(shape, at) }
}

/// Upgrades, in place, the bindings in `elements` that were saved by older
/// Excalidraw versions. Bindings to shapes outside the slice are dropped.
pub(crate) fn migrate_legacy(elements: &mut [Element]) {
    for i in 0..elements.len() {
        for end in End::BOTH {
            let arrow = &elements[i];
            if !is_legacy(arrow, end) {
                continue;
            }
            let shape = read(arrow, end)
                .and_then(|b| elements.iter().find(|e| !e.is_deleted && e.id == b.element_id))
                .filter(|_| arrow.points.len() >= 2);
            let upgraded = shape.map(|shape| migrate(arrow, end, shape));
            write(&mut elements[i], end, upgraded.as_ref());
        }
    }
}

/// Excalidraw never stores an exact 0.5 ratio (it makes arrow headings
/// jitter) nor ratios far outside the shape.
fn normalize(fixed_point: [f64; 2]) -> [f64; 2] {
    fixed_point.map(|ratio| {
        let ratio = if ratio.is_finite() { ratio.clamp(-FIXED_POINT_BOUND, FIXED_POINT_BOUND) } else { 0.5 };
        if (ratio - 0.5).abs() < 0.0001 { 0.5001 } else { ratio }
    })
}

pub(crate) fn gap(shape: &Element) -> f64 {
    BASE_GAP + shape.stroke_width / 2.0
}

/// Scene position of a fixed point of `shape`.
pub(crate) fn global_fixed_point(shape: &Element, fixed_point: [f64; 2]) -> Point {
    shape.transform() * Point::new(shape.width * fixed_point[0], shape.height * fixed_point[1])
}

/// The fixed point of `shape` that lies at the scene position `at`.
pub(crate) fn fixed_point_for(shape: &Element, at: Point) -> [f64; 2] {
    if shape.width < 1.0 || shape.height < 1.0 {
        return normalize([0.5, 0.5]);
    }
    let local = shape.transform().inverse() * at;
    let floor = gap(shape);
    normalize([local.x / shape.width.max(floor), local.y / shape.height.max(floor)])
}

fn contains(shape: &Element, at: Point) -> bool {
    outline(shape).contains(shape.transform().inverse() * at)
}

fn distance(shape: &Element, at: Point) -> f64 {
    distance_to_path(&outline(shape), shape.transform().inverse() * at)
}

/// Whether an arrow end dropped at `at` should attach to `shape`.
fn attracts(shape: &Element, at: Point, zoom: f64) -> bool {
    // Zoomed out, a slightly larger reach keeps small shapes easy to hit.
    let reach = (BIND_DISTANCE / (zoom.min(1.0) * 1.5)).clamp(BIND_DISTANCE, BIND_DISTANCE * 2.0);
    !shape.is_deleted && is_bindable(&shape.kind) && (contains(shape, at) || distance(shape, at) <= reach)
}

/// Index of the shape an arrow end at `at` attaches to: the topmost one
/// that is under or near the point. Labels are skipped in favor of their
/// containers.
pub(crate) fn candidate(elements: &[Element], at: Point, zoom: f64) -> Option<usize> {
    elements.iter().rposition(|e| e.container_id.is_none() && attracts(e, at, zoom))
}

/// `shape` grown by `offset` on every side, keeping its center and angle.
fn grown(shape: &Element, offset: f64) -> Element {
    // A diamond's edges are slanted, so growing its box moves them less.
    let grow = match shape.kind {
        Kind::Diamond if shape.width + shape.height > 0.0 => {
            offset * shape.width.hypot(shape.height) / (shape.width + shape.height) * 2.0
        }
        _ => offset,
    };
    Element {
        x: shape.x - grow,
        y: shape.y - grow,
        width: shape.width + 2.0 * grow,
        height: shape.height + 2.0 * grow,
        extra: Default::default(),
        ..shape.clone()
    }
}

/// Points where the segment `a`–`b` crosses the outline of `shape` pushed
/// outward by `offset`.
pub(crate) fn intersections(shape: &Element, a: Point, b: Point, offset: f64) -> Vec<Point> {
    let grown = grown(shape, offset);
    let to_scene = grown.transform();
    let (a, b) = (to_scene.inverse() * a, to_scene.inverse() * b);
    let polygon = flatten(&outline(&grown), 0.25);
    let d = b - a;
    let mut hits = Vec::new();
    for i in 0..polygon.len() {
        let (p, q) = (polygon[i], polygon[(i + 1) % polygon.len()]);
        let e = q - p;
        let denom = d.cross(e);
        if denom.abs() < 1e-12 {
            continue;
        }
        let t = (p - a).cross(e) / denom;
        let u = (p - a).cross(d) / denom;
        if (0.0..=1.0).contains(&t) && (0.0..=1.0).contains(&u) {
            hits.push(to_scene * (a + d * t));
        }
    }
    hits
}

fn nearest(points: Vec<Point>, to: Point) -> Option<Point> {
    points.into_iter().min_by(|a, b| a.distance_squared(to).total_cmp(&b.distance_squared(to)))
}

/// Scene coordinates of a linear element's points.
pub(crate) fn global_points(el: &Element) -> Vec<Point> {
    let to_scene = el.transform();
    el.local_points().into_iter().map(|p| to_scene * p).collect()
}

/// Moves a linear element's points to the scene positions `points`,
/// keeping its angle and Excalidraw's convention that the first point is
/// the element's origin.
pub(crate) fn set_global_points(el: &mut Element, points: &[Point]) {
    let Some(&first) = points.first() else {
        return;
    };
    // Undo the rotation about the scene origin, then find where the true
    // rotation center (the middle of the points' box) has to be.
    let unrotate = kurbo::Affine::rotate(-el.angle);
    let flat: Vec<Point> = points.iter().map(|&p| unrotate * p).collect();
    let bounds = flat[1..].iter().fold(kurbo::Rect::from_points(flat[0], flat[0]), |r, &p| r.union_pt(p));
    let center = kurbo::Affine::rotate(el.angle) * bounds.center();
    let origin = center + (flat[0] - bounds.center());
    el.points = flat.iter().map(|p| [p.x - flat[0].x, p.y - flat[0].y]).collect();
    (el.x, el.y) = (origin.x, origin.y);
    (el.width, el.height) = (bounds.width(), bounds.height());
    debug_assert!(el.angle != 0.0 || first.distance(origin) < 1e-6);
}

/// Where the `end` of `arrow` belongs given its binding to `shape`.
/// `points` are the arrow's current scene points and `other` the binding
/// and shape of its opposite end, if bound. Port of `updateBoundPoint`.
pub(crate) fn bound_point(
    arrow: &Element,
    points: &[Point],
    end: End,
    binding: &Binding,
    shape: &Element,
    other: Option<(&Binding, &Element)>,
) -> Point {
    let focus = global_fixed_point(shape, binding.fixed_point);
    if binding.inside || points.len() < 2 {
        return focus;
    }
    let other_arrow_point = points[end.indices(points.len()).1];
    let other_focus = other.map(|(b, s)| global_fixed_point(s, b.fixed_point));
    // A two-point arrow between two shapes runs from focus to focus.
    let aim = other_focus.filter(|_| points.len() == 2).unwrap_or(other_arrow_point);

    let outline_point = nearest(intersections(shape, focus, aim, gap(shape)), aim);
    let other_outline_point = other.and_then(|(_, s)| nearest(intersections(s, focus, aim, gap(s)), focus));

    let has_head = |e: End| match e {
        End::Start => arrow.start_arrowhead.is_some(),
        End::End => arrow.end_arrowhead.is_some(),
    };
    let target = if has_head(end) || !has_head(end.other()) { focus } else { outline_point.unwrap_or(focus) };

    if let (Some((_, other_shape)), Some(at)) = (other, outline_point) {
        // Our outline point falls inside the other shape (overlapping
        // shapes): snapping to it would turn the arrow around.
        let area = |e: &Element| e.width * e.height;
        let covered = contains(other_shape, at) || distance(other_shape, at) <= gap(other_shape);
        if covered && area(other_shape) < area(shape) * 2.0 {
            return target;
        }
    }

    let other_target = match other {
        Some(_) => other_outline_point.or(other_focus).unwrap_or(other_arrow_point),
        None => other_arrow_point,
    };
    let too_short = other_target.distance(outline_point.unwrap_or(focus)) <= MIN_ARROW_LENGTH;
    match (other, too_short) {
        (None, true) => focus,
        (Some(_), true) => target,
        _ => outline_point.unwrap_or(focus),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f64::consts::FRAC_PI_2;

    fn rect(x: f64, y: f64) -> Element {
        Element { id: format!("r{x}"), x, y, width: 100.0, height: 100.0, ..Element::default() }
    }

    fn arrow(points: &[(f64, f64)]) -> Element {
        let mut el = Element { kind: Kind::Arrow, end_arrowhead: Some("arrow".into()), ..Element::default() };
        set_global_points(&mut el, &points.iter().map(|&p| Point::from(p)).collect::<Vec<_>>());
        el
    }

    fn center(shape: &Element) -> Binding {
        Binding { element_id: shape.id.clone(), fixed_point: normalize([0.5, 0.5]), inside: false }
    }

    #[test]
    fn bindings_round_trip_through_json() {
        let mut a = arrow(&[(0.0, 0.0), (50.0, 0.0)]);
        assert_eq!(read(&a, End::Start), None);
        let b = Binding { element_id: "s".into(), fixed_point: [0.25, 0.5001], inside: true };
        write(&mut a, End::End, Some(&b));
        assert_eq!(read(&a, End::End), Some(b));
        assert_eq!(a.extra["endBinding"]["mode"], "inside");
        write(&mut a, End::End, None);
        assert!(a.extra["endBinding"].is_null() && read(&a, End::End).is_none());
    }

    #[test]
    fn fixed_points_follow_the_shape() {
        let mut shape = Element { angle: 0.4, ..rect(200.0, 100.0) };
        let at = Point::new(230.0, 180.0);
        let fixed = fixed_point_for(&shape, at);
        assert!(global_fixed_point(&shape, fixed).distance(at) < 1e-9);
        shape.x += 70.0;
        shape.width *= 2.0;
        let moved = global_fixed_point(&shape, fixed);
        assert_eq!(fixed_point_for(&shape, moved).map(|r| (r * 1e6).round()), fixed.map(|r| (r * 1e6).round()));
        assert_eq!(normalize([0.5, 99.0]), [0.5001, 10.0]);
    }

    #[test]
    fn segments_cross_the_gap_outline() {
        let shape = rect(0.0, 0.0);
        let hits = intersections(&shape, Point::new(50.0, 50.0), Point::new(300.0, 50.0), 6.0);
        assert_eq!(hits.len(), 1);
        assert!(hits[0].distance(Point::new(106.0, 50.0)) < 1e-6);
        assert_eq!(intersections(&shape, Point::new(-50.0, 50.0), Point::new(300.0, 50.0), 6.0).len(), 2);
        assert!(intersections(&shape, Point::new(200.0, 0.0), Point::new(300.0, 50.0), 6.0).is_empty());

        // A diamond's slanted edges end up the same gap away.
        let diamond = Element { kind: Kind::Diamond, ..rect(0.0, 0.0) };
        let hit = intersections(&diamond, Point::new(50.0, 50.0), Point::new(150.0, 150.0), 6.0)[0];
        assert!((hit.distance(Point::new(75.0, 75.0)) - 6.0).abs() < 0.01, "{hit:?}");

        let turned = Element { angle: FRAC_PI_2, width: 200.0, ..rect(0.0, 0.0) };
        let hit = intersections(&turned, Point::new(100.0, 50.0), Point::new(100.0, 400.0), 0.0)[0];
        assert!(hit.distance(Point::new(100.0, 150.0)) < 1e-6);
    }

    #[test]
    fn candidates_include_hollow_interiors_and_nearby_points() {
        let shapes = [rect(0.0, 0.0), rect(300.0, 0.0), arrow(&[(0.0, 0.0), (400.0, 0.0)])];
        assert_eq!(candidate(&shapes, Point::new(50.0, 50.0), 1.0), Some(0));
        assert_eq!(candidate(&shapes, Point::new(290.0, 50.0), 1.0), Some(1));
        assert_eq!(candidate(&shapes, Point::new(200.0, 50.0), 1.0), None);
        assert_eq!(candidate(&shapes, Point::new(200.0, 0.0), 1.0), None, "arrows are not bindable");
    }

    #[test]
    fn orbit_ends_stop_at_the_outline_facing_the_other_shape() {
        let (left, right) = (rect(0.0, 0.0), rect(300.0, 0.0));
        let a = arrow(&[(50.0, 50.0), (350.0, 50.0)]);
        let points = global_points(&a);
        let (from, to) = (center(&left), center(&right));
        let start = bound_point(&a, &points, End::Start, &from, &left, Some((&to, &right)));
        let end = bound_point(&a, &points, End::End, &to, &right, Some((&from, &left)));
        assert!(start.distance(Point::new(106.0, 50.0)) < 0.05, "{start:?}");
        assert!(end.distance(Point::new(294.0, 50.0)) < 0.05, "{end:?}");

        // Unbound at the other end, it aims at the neighboring arrow point.
        let free = bound_point(&a, &[Point::new(50.0, 300.0), points[1]], End::End, &to, &right, None);
        assert!((free.x - 294.0).abs() < 0.05 && free.y > 50.0, "{free:?}");
    }

    #[test]
    fn inside_and_too_short_arrows_aim_at_the_fixed_point() {
        let shape = rect(0.0, 0.0);
        let a = arrow(&[(50.0, 50.0), (110.0, 50.0)]);
        let points = global_points(&a);
        let inside = Binding { inside: true, ..center(&shape) };
        assert!(bound_point(&a, &points, End::Start, &inside, &shape, None).distance(Point::new(50.0, 50.0)) < 0.02);
        // The free end is 4px from where the outline point would be.
        let near = bound_point(&a, &points, End::Start, &center(&shape), &shape, None);
        assert!(near.distance(Point::new(50.0, 50.0)) < 0.02, "{near:?}");
    }

    #[test]
    fn global_points_survive_rotation() {
        let mut a = Element { angle: 0.8, ..arrow(&[(0.0, 0.0), (10.0, 0.0)]) };
        let target = [Point::new(120.0, 40.0), Point::new(200.0, 90.0), Point::new(260.0, 20.0)];
        set_global_points(&mut a, &target);
        assert_eq!(a.points[0], [0.0, 0.0]);
        for (got, want) in global_points(&a).iter().zip(target) {
            assert!(got.distance(want) < 1e-9);
        }
    }

    #[test]
    fn legacy_bindings_keep_the_arrow_where_it_is() {
        let shape = rect(300.0, 0.0);
        let mut a = arrow(&[(50.0, 50.0), (294.0, 50.0)]);
        a.extra.insert("endBinding".into(), json!({"elementId": shape.id, "focus": 0.0, "gap": 6.0}));
        assert!(is_legacy(&a, End::End) && !is_legacy(&a, End::Start));
        let binding = migrate(&a, End::End, &shape);
        assert!(!binding.inside);
        let points = global_points(&a);
        let end = bound_point(&a, &points, End::End, &binding, &shape, None);
        assert!(end.distance(points[1]) < 0.5, "{end:?}");
    }
}
