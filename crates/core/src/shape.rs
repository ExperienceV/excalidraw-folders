//! Turns an [`Element`] into draw ops, in element-local coordinates
//! (origin at the element's `x`/`y`, before rotation).

use kurbo::{BezPath, Ellipse, ParamCurve, PathEl, Point, Shape, Vec2};

use crate::draw::{Color, Crop, DrawOp, Style};
use crate::element::{Element, FillStyle, Kind, Roundness, StrokeStyle};
use crate::freehand;
use crate::rough::{Options, Rough, Seg};
use crate::text::text_path;

const DEFAULT_ADAPTIVE_RADIUS: f64 = 32.0;
const PROPORTIONAL_RADIUS: f64 = 0.25;
/// Direction of hachure lines, in screen degrees (leans like "/").
const HACHURE_ANGLE: f64 = -49.0;
/// Shown in place of an image whose data is missing or unreadable.
const IMAGE_PLACEHOLDER: Color = Color::rgba(222, 226, 230, 255);

/// Corner radius for a side of length `x`, following Excalidraw's rules.
fn corner_radius(x: f64, roundness: Option<&Roundness>) -> f64 {
    match roundness {
        None => 0.0,
        Some(r) if r.kind == Roundness::ADAPTIVE => {
            let fixed = r.value.unwrap_or(DEFAULT_ADAPTIVE_RADIUS);
            if x <= fixed / PROPORTIONAL_RADIUS {
                x * PROPORTIONAL_RADIUS
            } else {
                fixed
            }
        }
        Some(_) => x * PROPORTIONAL_RADIUS,
    }
}

/// A closed polygon whose corners are cut back by `cut(edge_len)` and
/// replaced with quadratic curves.
fn rounded_polygon(pts: &[Point], cut: impl Fn(f64) -> f64) -> Vec<Seg> {
    let n = pts.len();
    let toward = |from: Point, to: Point| {
        let len = (to - from).hypot();
        let c = cut(len).min(len / 2.0);
        if len > 0.0 { from + (to - from) * (c / len) } else { from }
    };
    let mut segs = Vec::with_capacity(n * 2);
    for i in 0..n {
        let (prev, v, next) = (pts[(i + n - 1) % n], pts[i], pts[(i + 1) % n]);
        let (enter, leave) = (toward(v, prev), toward(v, next));
        if enter != v || leave != v {
            segs.push(Seg::Quad(enter, v, leave));
        }
        segs.push(Seg::Line(leave, toward(next, v)));
    }
    segs
}

fn segs_to_path(segs: &[Seg]) -> BezPath {
    let mut path = BezPath::new();
    for (i, seg) in segs.iter().enumerate() {
        let (start, el) = match *seg {
            Seg::Line(a, b) => (a, PathEl::LineTo(b)),
            Seg::Quad(a, c, b) => (a, PathEl::QuadTo(c, b)),
        };
        if i == 0 {
            path.move_to(start);
        }
        path.push(el);
    }
    path.close_path();
    path
}

fn box_segs(el: &Element) -> Vec<Seg> {
    let (w, h) = (el.width, el.height);
    let roundness = el.roundness.as_ref();
    if el.kind == Kind::Diamond {
        let pts = [(w / 2.0, 0.0), (w, h / 2.0), (w / 2.0, h), (0.0, h / 2.0)].map(Point::from);
        rounded_polygon(&pts, |len| corner_radius(len, roundness))
    } else {
        let pts = [(0.0, 0.0), (w, 0.0), (w, h), (0.0, h)].map(Point::from);
        let r = corner_radius(w.min(h), roundness);
        rounded_polygon(&pts, |_| r)
    }
}

/// Smooth curve through the points, matching the sketchy version's spline.
fn smooth_path(pts: &[Point]) -> BezPath {
    let mut path = BezPath::new();
    let n = pts.len();
    if n < 2 {
        return path;
    }
    path.move_to(pts[0]);
    for i in 0..n - 1 {
        let p0 = pts[i.saturating_sub(1)];
        let (p1, p2) = (pts[i], pts[i + 1]);
        let p3 = pts[(i + 2).min(n - 1)];
        path.curve_to(p1 + (p2 - p0) / 6.0, p2 + (p1 - p3) / 6.0, p2);
    }
    path
}

fn polyline(pts: &[Point]) -> BezPath {
    let mut path = BezPath::new();
    for (i, &p) in pts.iter().enumerate() {
        if i == 0 {
            path.move_to(p);
        } else {
            path.line_to(p);
        }
    }
    path
}

fn freedraw_contour(el: &Element) -> BezPath {
    let simulate = el.simulate_pressure.unwrap_or_else(|| el.pressures.is_empty());
    let pts: Vec<(Point, Option<f64>)> = el
        .points
        .iter()
        .enumerate()
        .map(|(i, p)| (Point::new(p[0], p[1]), el.pressures.get(i).copied()))
        .collect();
    let outline = freehand::get_stroke(&pts, &freehand::excalidraw_options(el.stroke_width, simulate));
    freehand::stroke_to_path(&outline)
}

/// Freehand stroke centerline: quadratic segments through the midpoints of the samples.
fn freedraw_path(pts: &[Point]) -> BezPath {
    let mut path = BezPath::new();
    let Some(&first) = pts.first() else {
        return path;
    };
    path.move_to(first);
    if pts.len() == 1 {
        // A dot: zero-length segment, the round caps give it a body.
        path.line_to(first);
        return path;
    }
    for w in pts[1..].windows(2) {
        path.quad_to(w[0], w[0].midpoint(w[1]));
    }
    path.line_to(pts[pts.len() - 1]);
    path
}

/// The ideal (non-sketchy) geometry of the element, used for filling,
/// hit-testing and selection.
pub fn outline(el: &Element) -> BezPath {
    match el.kind {
        Kind::Rectangle | Kind::Diamond => segs_to_path(&box_segs(el)),
        Kind::Ellipse => {
            let center = Point::new(el.width / 2.0, el.height / 2.0);
            Ellipse::new(center, (el.width / 2.0, el.height / 2.0), 0.0).to_path(0.1)
        }
        Kind::Line | Kind::Arrow => {
            let pts = el.local_points();
            if el.roundness.is_some() { smooth_path(&pts) } else { polyline(&pts) }
        }
        Kind::Freedraw => freedraw_path(&el.local_points()),
        Kind::Text | Kind::Image | Kind::Other(_) => el.local_bounds().to_path(0.1),
    }
}

/// Flattens a path into the vertices of a single polygon.
pub fn flatten(path: &BezPath, tolerance: f64) -> Vec<Point> {
    let mut pts = Vec::new();
    kurbo::flatten(path, tolerance, |el| match el {
        PathEl::MoveTo(p) | PathEl::LineTo(p) => pts.push(p),
        _ => {}
    });
    pts
}

/// Whether a line's ends meet, making it a fillable polygon.
pub fn is_closed(el: &Element) -> bool {
    el.kind == Kind::Line
        && el.points.len() > 2
        && el.points.first().zip(el.points.last()).is_some_and(|(a, b)| {
            (a[0] - b[0]).hypot(a[1] - b[1]) < 1.0
        })
}

/// Excalidraw tones roughness down on small shapes so they stay legible.
fn adjusted_roughness(el: &Element) -> f64 {
    let b = el.local_bounds();
    let (max, min) = (b.width().max(b.height()), b.width().min(b.height()));
    let linear = matches!(el.kind, Kind::Line | Kind::Arrow);
    if (min >= 20.0 && max >= 50.0)
        || (min >= 15.0 && el.roundness.is_some() && !linear)
        || (linear && max >= 50.0)
    {
        return el.roughness;
    }
    (el.roughness / if max < 10.0 { 3.0 } else { 2.0 }).min(2.5)
}

/// The point an arrowhead at the last of `pts` points away from. On a
/// curved arrow that is a point 30% back along the final curve segment, as
/// in Excalidraw, so the head follows the curve rather than the chord.
fn head_direction_point(pts: &[Point], round: bool) -> Point {
    let n = pts.len();
    if !round || n < 3 {
        return pts[n - 2];
    }
    // The last cubic of `smooth_path`.
    let (p0, p1, p2) = (pts[n - 3], pts[n - 2], pts[n - 1]);
    let (c1, c2) = (p1 + (p2 - p0) / 6.0, p2 + (p1 - p2) / 6.0);
    kurbo::CubicBez::new(p1, c1, c2, p2).eval(0.7)
}

fn arrowhead(rough: &mut Rough, kind: &str, tip: Point, from: Point, path: &mut BezPath) {
    let dir = tip - from;
    let len = dir.hypot();
    if len == 0.0 {
        return;
    }
    let (size, spread) = match kind {
        "bar" => (15f64, 90f64),
        "triangle" | "triangle_outline" => (15.0, 25.0),
        _ => (25.0, 20.0),
    };
    let back = -dir / len * size.min(len / 2.0);
    let wing = |deg: f64| {
        let (s, c) = deg.to_radians().sin_cos();
        tip + Vec2::new(back.x * c - back.y * s, back.x * s + back.y * c)
    };
    let (a, b) = (wing(spread), wing(-spread));
    match kind {
        "bar" => rough.line(a, b, path),
        "triangle" | "triangle_outline" => rough.linear_path(&[a, tip, b], true, path),
        _ => {
            rough.line(a, tip, path);
            rough.line(b, tip, path);
        }
    }
}

/// The single op of an image element: its file, crop and flips.
fn image_op(el: &Element) -> Option<DrawOp> {
    let file_id = el.extra.get("fileId")?.as_str()?.to_owned();
    let number = |value: &serde_json::Value, key: &str| value[key].as_f64();
    let crop = el.extra.get("crop").and_then(|c| {
        Some(Crop {
            x: number(c, "x")?,
            y: number(c, "y")?,
            width: number(c, "width")?,
            height: number(c, "height")?,
            natural_width: number(c, "naturalWidth")?,
            natural_height: number(c, "naturalHeight")?,
        })
    });
    // Excalidraw flips an image by giving it a negative scale.
    let flipped = |axis: usize| el.extra.get("scale").is_some_and(|s| s[axis].as_f64().is_some_and(|v| v < 0.0));
    Some(DrawOp {
        path: el.local_bounds().to_path(0.1),
        color: IMAGE_PLACEHOLDER,
        style: Style::Image { file_id, crop, flip_x: flipped(0), flip_y: flipped(1) },
    })
}

/// Generates the element's draw ops, fill first. Deterministic in the
/// element's `seed`, so the result can be cached until the element changes.
pub fn element_ops(el: &Element) -> Vec<DrawOp> {
    let stroke = Color::parse(&el.stroke_color).unwrap_or(Color::rgba(30, 30, 30, 255));
    let bg = Color::parse(&el.background_color).unwrap_or(Color::TRANSPARENT);
    let solid = el.stroke_style == StrokeStyle::Solid;
    let sw = el.stroke_width;
    let width = if solid { sw } else { sw + 0.5 };
    let dash = match el.stroke_style {
        StrokeStyle::Solid => Vec::new(),
        StrokeStyle::Dashed => vec![8.0, 8.0 + sw],
        StrokeStyle::Dotted => vec![1.5, 6.0 + sw],
    };
    let mut rough = Rough::new(
        Options {
            roughness: adjusted_roughness(el),
            disable_multi_stroke: !solid,
            preserve_vertices: el.roughness < 2.0,
            curve_fitting: if el.kind == Kind::Ellipse { 1.0 } else { 0.95 },
            ..Options::default()
        },
        el.seed,
    );

    let mut strokes = BezPath::new();
    let mut heads = BezPath::new();
    let mut fill_area = None;
    match el.kind {
        Kind::Rectangle | Kind::Diamond => {
            rough.segments(&box_segs(el), &mut strokes);
            fill_area = Some(outline(el));
        }
        Kind::Ellipse => {
            let center = Point::new(el.width / 2.0, el.height / 2.0);
            rough.ellipse(center, el.width, el.height, &mut strokes);
            fill_area = Some(outline(el));
        }
        Kind::Line | Kind::Arrow => {
            let pts = el.local_points();
            if pts.len() < 2 {
                return Vec::new();
            }
            if el.roundness.is_some() {
                rough.curve(&pts, &mut strokes);
            } else {
                rough.linear_path(&pts, false, &mut strokes);
            }
            if is_closed(el) {
                let mut area = outline(el);
                area.close_path();
                fill_area = Some(area);
            }
            if el.kind == Kind::Arrow {
                let round = el.roundness.is_some();
                if let Some(kind) = &el.end_arrowhead {
                    arrowhead(&mut rough, kind, pts[pts.len() - 1], head_direction_point(&pts, round), &mut heads);
                }
                if let Some(kind) = &el.start_arrowhead {
                    let reversed: Vec<Point> = pts.iter().rev().copied().collect();
                    arrowhead(&mut rough, kind, pts[0], head_direction_point(&reversed, round), &mut heads);
                }
            }
        }
        Kind::Freedraw => {
            if stroke.is_transparent() {
                return Vec::new();
            }
            let path = freedraw_contour(el);
            return if path.is_empty() { Vec::new() } else { vec![DrawOp::fill(path, stroke)] };
        }
        Kind::Text => {
            let glyphs = text_path(el);
            return if glyphs.is_empty() { Vec::new() } else { vec![DrawOp::fill(glyphs, stroke)] };
        }
        Kind::Image => return image_op(el).into_iter().collect(),
        Kind::Other(_) => return Vec::new(),
    }

    let mut ops = Vec::new();
    if let Some(area) = fill_area.filter(|_| !bg.is_transparent()) {
        if el.fill_style == FillStyle::Solid {
            ops.push(DrawOp::fill(area, bg));
        } else {
            let polygon = flatten(&area, 0.5);
            let gap = sw * 4.0;
            let mut lines = BezPath::new();
            let zigzag = el.fill_style == FillStyle::Zigzag;
            rough.hachure(&polygon, gap, HACHURE_ANGLE, zigzag, &mut lines);
            if el.fill_style == FillStyle::CrossHatch {
                rough.hachure(&polygon, gap, HACHURE_ANGLE + 90.0, false, &mut lines);
            }
            ops.push(DrawOp::stroke(lines, bg, sw / 2.0));
        }
    }
    if !stroke.is_transparent() {
        ops.push(DrawOp { path: strokes, color: stroke, style: Style::Stroke { width, dash } });
        if !heads.is_empty() {
            ops.push(DrawOp::stroke(heads, stroke, width));
        }
    }
    ops
}

#[cfg(test)]
mod tests {
    use super::*;

    fn el(kind: Kind) -> Element {
        Element { kind, width: 200.0, height: 100.0, seed: 3, ..Element::default() }
    }

    #[test]
    fn adaptive_radius_caps_at_fixed_value() {
        let r = Roundness { kind: Roundness::ADAPTIVE, value: None };
        assert_eq!(corner_radius(40.0, Some(&r)), 10.0);
        assert_eq!(corner_radius(400.0, Some(&r)), 32.0);
        assert_eq!(corner_radius(400.0, None), 0.0);
    }

    #[test]
    fn outlines_cover_the_element_box() {
        for kind in [Kind::Rectangle, Kind::Diamond, Kind::Ellipse] {
            for roundness in [None, Some(Roundness { kind: 3, value: None })] {
                // Rounded diamond corners stop short of the box, as in Excalidraw.
                let slack = if kind == Kind::Diamond && roundness.is_some() { 0.15 } else { 0.003 };
                let e = Element { roundness, ..el(kind.clone()) };
                let b = outline(&e).bounding_box();
                assert!(b.x0 >= -0.01 && b.y0 >= -0.01 && b.x1 <= 200.01 && b.y1 <= 100.01, "{kind:?} {b:?}");
                assert!(b.width() >= 200.0 * (1.0 - slack) && b.height() >= 100.0 * (1.0 - slack), "{kind:?} {b:?}");
                assert!(outline(&e).contains(Point::new(100.0, 50.0)));
            }
        }
    }

    #[test]
    fn fill_styles_produce_expected_ops() {
        let mut e = el(Kind::Rectangle);
        assert_eq!(element_ops(&e).len(), 1, "transparent background: stroke only");

        e.background_color = "#ffc9c9".into();
        let ops = element_ops(&e);
        assert_eq!(ops.len(), 2);
        assert_eq!(ops[0].style, Style::Fill);

        e.fill_style = FillStyle::Hachure;
        let ops = element_ops(&e);
        assert!(matches!(ops[0].style, Style::Stroke { width, .. } if width == 1.0));
        let b = ops[0].path.bounding_box();
        assert!(b.x0 > -6.0 && b.y0 > -6.0 && b.x1 < 206.0 && b.y1 < 106.0, "{b:?}");
    }

    #[test]
    fn arrow_has_a_head_and_dashes_only_on_the_shaft() {
        let e = Element {
            points: vec![[0.0, 0.0], [120.0, 60.0]],
            end_arrowhead: Some("arrow".into()),
            stroke_style: StrokeStyle::Dashed,
            ..el(Kind::Arrow)
        };
        let ops = element_ops(&e);
        assert_eq!(ops.len(), 2);
        assert!(matches!(&ops[0].style, Style::Stroke { dash, .. } if !dash.is_empty()));
        assert!(matches!(&ops[1].style, Style::Stroke { dash, .. } if dash.is_empty()));
    }

    #[test]
    fn image_op_carries_file_crop_and_flips() {
        let mut e = el(Kind::Image);
        assert!(element_ops(&e).is_empty(), "no file, nothing to draw");
        e.extra.insert("fileId".into(), "abc".into());
        e.extra.insert("scale".into(), serde_json::json!([-1, 1]));
        let crop = serde_json::json!({"x": 10, "y": 20, "width": 300, "height": 150, "naturalWidth": 800, "naturalHeight": 600});
        e.extra.insert("crop".into(), crop);
        let ops = element_ops(&e);
        assert_eq!(ops[0].path.bounding_box(), kurbo::Rect::new(0.0, 0.0, 200.0, 100.0));
        let Style::Image { file_id, crop: Some(crop), flip_x: true, flip_y: false } = &ops[0].style else {
            panic!("{:?}", ops[0].style)
        };
        assert_eq!((file_id.as_str(), crop.x, crop.width, crop.natural_height), ("abc", 10.0, 300.0, 600.0));
    }

    #[test]
    fn curved_arrow_heads_follow_the_curve() {
        let pts = [(0.0, 0.0), (100.0, 0.0), (100.0, 100.0)].map(Point::from);
        assert_eq!(head_direction_point(&pts, false), pts[1]);
        assert_eq!(head_direction_point(&pts[..2], true), pts[0]);
        // The curve bulges outward around the corner, so it arrives from the right of the chord.
        let from = head_direction_point(&pts, true);
        assert!(from.x > 100.0 && from.y > 50.0 && from.y < 100.0, "{from:?}");
    }

    #[test]
    fn closed_line_is_fillable() {
        let mut e = Element {
            points: vec![[0.0, 0.0], [100.0, 0.0], [50.0, 80.0], [0.0, 0.0]],
            background_color: "#a5d8ff".into(),
            ..el(Kind::Line)
        };
        assert!(is_closed(&e));
        assert_eq!(element_ops(&e)[0].style, Style::Fill);
        e.points.pop();
        assert!(!is_closed(&e));
    }
}
