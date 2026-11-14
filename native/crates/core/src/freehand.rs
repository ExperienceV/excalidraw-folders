//! Variable-width freehand stroke generation, ported from `perfect-freehand`
//! with Excalidraw's tuning constants.

use std::f64::consts::{FRAC_PI_2, PI};

use kurbo::{BezPath, Point, Vec2};

const RATE_OF_PRESSURE_CHANGE: f64 = 0.275;
const FIXED_PI: f64 = PI + 0.0001;

#[derive(Clone, Copy, Debug)]
pub struct TaperOptions {
    pub cap: bool,
    pub taper: f64,
    pub easing: fn(f64) -> f64,
}

#[derive(Clone, Copy, Debug)]
pub struct Options {
    pub size: f64,
    pub thinning: f64,
    pub smoothing: f64,
    pub streamline: f64,
    pub simulate_pressure: bool,
    pub last: bool,
    pub easing: fn(f64) -> f64,
    pub start: TaperOptions,
    pub end: TaperOptions,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            size: 16.0,
            thinning: 0.5,
            smoothing: 0.5,
            streamline: 0.5,
            simulate_pressure: true,
            last: false,
            easing: |t| t,
            start: TaperOptions { cap: true, taper: 0.0, easing: |t| t * (2.0 - t) },
            end: TaperOptions { cap: true, taper: 0.0, easing: |t| (t - 1.0).powi(3) + 1.0 },
        }
    }
}

/// Excalidraw's empirical parameters for variable-width freedraw strokes.
pub fn excalidraw_options(stroke_width: f64, simulate_pressure: bool) -> Options {
    Options {
        size: stroke_width * 4.25,
        thinning: 0.6,
        smoothing: 0.5,
        streamline: 0.5,
        simulate_pressure,
        last: true,
        easing: |t| (t * FRAC_PI_2).sin(),
        ..Options::default()
    }
}

#[derive(Clone, Copy, Debug)]
struct StrokePoint {
    point: Point,
    pressure: f64,
    vector: Vec2,
    distance: f64,
    running_length: f64,
}

fn get_stroke_radius(size: f64, thinning: f64, pressure: f64, easing: fn(f64) -> f64) -> f64 {
    size * easing(0.5 - thinning * (0.5 - pressure))
}

fn perp(v: Vec2) -> Vec2 {
    Vec2::new(v.y, -v.x)
}

fn rot_around(p: Point, center: Point, r: f64) -> Point {
    let (s, c) = r.sin_cos();
    let d = p - center;
    Point::new(d.x * c - d.y * s + center.x, d.x * s + d.y * c + center.y)
}

fn get_stroke_points(points: &[(Point, Option<f64>)], opts: &Options) -> Vec<StrokePoint> {
    if points.is_empty() {
        return Vec::new();
    }

    let t = 0.15 + (1.0 - opts.streamline) * 0.85;
    let mut pts: Vec<(Point, Option<f64>)> = Vec::with_capacity(points.len().max(5));

    if points.len() == 2 {
        let (p0, pr0) = points[0];
        let (p1, pr1) = points[1];
        pts.push((p0, pr0));
        for i in 1..5 {
            let u = f64::from(i) / 4.0;
            let pr = match (pr0, pr1) {
                (Some(a), Some(b)) => Some(a + (b - a) * u),
                _ => None,
            };
            pts.push((p0.lerp(p1, u), pr));
        }
    } else {
        pts.extend_from_slice(points);
        if pts.len() == 1 {
            let (p0, pr0) = pts[0];
            pts.push((p0 + Vec2::new(1.0, 1.0), pr0));
        }
    }

    let p0_pressure = pts[0].1.filter(|&p| p >= 0.0).unwrap_or(0.25);
    let mut prev = StrokePoint {
        point: pts[0].0,
        pressure: p0_pressure,
        vector: Vec2::new(1.0, 1.0),
        distance: 0.0,
        running_length: 0.0,
    };
    let mut stroke_points = vec![prev];

    let mut has_reached_minimum_length = false;
    let mut running_length = 0.0;
    let max = pts.len() - 1;

    for (i, &(raw_pt, raw_pressure)) in pts.iter().enumerate().skip(1) {
        let pt = if opts.last && i == max { raw_pt } else { prev.point.lerp(raw_pt, t) };
        if pt == prev.point {
            continue;
        }
        let distance = pt.distance(prev.point);
        running_length += distance;
        if i < max && !has_reached_minimum_length {
            if running_length < opts.size {
                continue;
            }
            has_reached_minimum_length = true;
        }
        let diff = prev.point - pt;
        let vector = if distance > 0.0 { diff / distance } else { Vec2::ZERO };
        let pressure = raw_pressure.filter(|&p| p >= 0.0).unwrap_or(0.5);
        prev = StrokePoint { point: pt, pressure, vector, distance, running_length };
        stroke_points.push(prev);
    }

    stroke_points[0].vector = stroke_points.get(1).map_or(Vec2::ZERO, |p| p.vector);
    stroke_points
}

fn get_stroke_outline_points(points: &[StrokePoint], opts: &Options) -> Vec<Point> {
    if points.is_empty() || opts.size <= 0.0 {
        return Vec::new();
    }

    let total_length = points[points.len() - 1].running_length;
    let taper_start = opts.start.taper;
    let taper_end = opts.end.taper;
    let min_distance = (opts.size * opts.smoothing).powi(2);

    let mut left_pts: Vec<Point> = Vec::new();
    let mut right_pts: Vec<Point> = Vec::new();

    let mut prev_pressure = points.iter().take(10).fold(points[0].pressure, |acc, cur| {
        let mut pressure = cur.pressure;
        if opts.simulate_pressure {
            let sp = (cur.distance / opts.size).min(1.0);
            let rp = (1.0 - sp).min(1.0);
            pressure = (acc + (rp - acc) * (sp * RATE_OF_PRESSURE_CHANGE)).min(1.0);
        }
        (acc + pressure) / 2.0
    });

    let mut radius = get_stroke_radius(opts.size, opts.thinning, points[points.len() - 1].pressure, opts.easing);
    let mut first_radius: Option<f64> = None;

    let mut prev_vector = points[0].vector;
    let mut pl = points[0].point;
    let mut pr = pl;
    let mut tl = pl;
    let mut tr = pr;
    let mut is_prev_point_sharp_corner = false;

    for i in 0..points.len() {
        let mut pressure = points[i].pressure;
        let point = points[i].point;
        let vector = points[i].vector;
        let distance = points[i].distance;
        let running_length = points[i].running_length;

        if i < points.len() - 1 && total_length - running_length < 3.0 {
            continue;
        }

        if opts.thinning != 0.0 {
            if opts.simulate_pressure {
                let sp = (distance / opts.size).min(1.0);
                let rp = (1.0 - sp).min(1.0);
                pressure = (prev_pressure + (rp - prev_pressure) * (sp * RATE_OF_PRESSURE_CHANGE)).min(1.0);
            }
            radius = get_stroke_radius(opts.size, opts.thinning, pressure, opts.easing);
        } else {
            radius = opts.size / 2.0;
        }

        if first_radius.is_none() {
            first_radius = Some(radius);
        }

        let ts = if running_length < taper_start {
            (opts.start.easing)(running_length / taper_start)
        } else {
            1.0
        };
        let te = if total_length - running_length < taper_end {
            (opts.end.easing)((total_length - running_length) / taper_end)
        } else {
            1.0
        };
        radius = (radius * ts.min(te)).max(0.01);

        let next_vector = if i < points.len() - 1 { points[i + 1].vector } else { points[i].vector };
        let next_dpr = if i < points.len() - 1 { vector.dot(next_vector) } else { 1.0 };
        let prev_dpr = vector.dot(prev_vector);

        let is_point_sharp_corner = prev_dpr < 0.0 && !is_prev_point_sharp_corner;
        let is_next_point_sharp_corner = next_dpr < 0.0;

        if is_point_sharp_corner || is_next_point_sharp_corner {
            let offset = perp(prev_vector) * radius;
            let step = 1.0 / 13.0;
            let mut t = 0.0;
            while t <= 1.0 {
                tl = rot_around(point - offset, point, FIXED_PI * t);
                left_pts.push(tl);
                tr = rot_around(point + offset, point, FIXED_PI * -t);
                right_pts.push(tr);
                t += step;
            }
            pl = tl;
            pr = tr;
            if is_next_point_sharp_corner {
                is_prev_point_sharp_corner = true;
            }
            continue;
        }

        is_prev_point_sharp_corner = false;

        if i == points.len() - 1 {
            let offset = perp(vector) * radius;
            left_pts.push(point - offset);
            right_pts.push(point + offset);
            continue;
        }

        let offset = perp(next_vector.lerp(vector, next_dpr)) * radius;
        tl = point - offset;
        if i <= 1 || pl.distance_squared(tl) > min_distance {
            left_pts.push(tl);
            pl = tl;
        }
        tr = point + offset;
        if i <= 1 || pr.distance_squared(tr) > min_distance {
            right_pts.push(tr);
            pr = tr;
        }

        prev_pressure = pressure;
        prev_vector = vector;
    }

    let first_point = points[0].point;
    let last_point = if points.len() > 1 {
        points[points.len() - 1].point
    } else {
        points[0].point + Vec2::new(1.0, 1.0)
    };

    let mut start_cap: Vec<Point> = Vec::new();
    let mut end_cap: Vec<Point> = Vec::new();

    if points.len() == 1 {
        if (taper_start == 0.0 && taper_end == 0.0) || opts.last {
            let diff = first_point - last_point;
            let len = diff.hypot();
            let dir = if len > 0.0 { diff / len } else { Vec2::ZERO };
            let start = first_point + perp(dir) * -(first_radius.unwrap_or(radius));
            let mut dot_pts = Vec::new();
            let step = 1.0 / 13.0;
            let mut t = step;
            while t <= 1.0 {
                dot_pts.push(rot_around(start, first_point, FIXED_PI * 2.0 * t));
                t += step;
            }
            return dot_pts;
        }
    } else {
        if taper_start == 0.0 && !(taper_end != 0.0 && points.len() == 1) {
            if opts.start.cap {
                let step = 1.0 / 13.0;
                let mut t = step;
                while t <= 1.0 {
                    start_cap.push(rot_around(right_pts[0], first_point, FIXED_PI * t));
                    t += step;
                }
            } else {
                let corners_vector = left_pts[0] - right_pts[0];
                let offset_a = corners_vector * 0.5;
                let offset_b = corners_vector * 0.51;
                start_cap.push(first_point - offset_a);
                start_cap.push(first_point - offset_b);
                start_cap.push(first_point + offset_b);
                start_cap.push(first_point + offset_a);
            }
        }

        let direction = perp(-points[points.len() - 1].vector);
        if taper_end != 0.0 || (taper_start != 0.0 && points.len() == 1) {
            end_cap.push(last_point);
        } else if opts.end.cap {
            let start = last_point + direction * radius;
            let step = 1.0 / 29.0;
            let mut t = step;
            while t < 1.0 {
                end_cap.push(rot_around(start, last_point, FIXED_PI * 3.0 * t));
                t += step;
            }
        } else {
            end_cap.push(last_point + direction * radius);
            end_cap.push(last_point + direction * (radius * 0.99));
            end_cap.push(last_point - direction * (radius * 0.99));
            end_cap.push(last_point - direction * radius);
        }
    }

    let mut out = Vec::with_capacity(left_pts.len() + end_cap.len() + right_pts.len() + start_cap.len());
    out.extend(left_pts);
    out.extend(end_cap);
    out.extend(right_pts.into_iter().rev());
    out.extend(start_cap);
    out
}

/// Computes the closed variable-width outline polygon for a freehand stroke.
pub fn get_stroke(points: &[(Point, Option<f64>)], opts: &Options) -> Vec<Point> {
    let stroke_points = get_stroke_points(points, opts);
    get_stroke_outline_points(&stroke_points, opts)
}

/// Converts a `perfect-freehand` outline polygon into a smooth closed `BezPath`,
/// matching Excalidraw's `getSvgPathFromStroke`.
pub fn stroke_to_path(outline: &[Point]) -> BezPath {
    if outline.is_empty() {
        return BezPath::new();
    }
    let mut path = BezPath::new();
    path.move_to(outline[0]);
    let n = outline.len();
    for i in 0..n {
        let next = outline[(i + 1) % n];
        path.quad_to(outline[i], outline[i].midpoint(next));
    }
    path.line_to(outline[0]);
    path.close_path();
    path
}

#[cfg(test)]
mod tests {
    use super::*;
    use kurbo::Shape;

    #[test]
    fn empty_input_produces_empty_outline() {
        assert!(get_stroke(&[], &excalidraw_options(2.0, true)).is_empty());
    }

    #[test]
    fn single_point_produces_closed_dot() {
        let outline = get_stroke(&[(Point::new(10.0, 20.0), None)], &excalidraw_options(2.0, true));
        assert!(outline.len() >= 10);
        let path = stroke_to_path(&outline);
        let bounds = path.bounding_box();
        assert!(bounds.contains(Point::new(10.0, 20.0)));
        assert!(bounds.width() > 2.0 && bounds.height() > 2.0);
    }

    #[test]
    fn pressure_varies_stroke_thickness() {
        let pts: Vec<Point> = (0..12).map(|i| Point::new(f64::from(i) * 10.0, 0.0)).collect();
        let light: Vec<_> = pts.iter().map(|&p| (p, Some(0.1))).collect();
        let heavy: Vec<_> = pts.iter().map(|&p| (p, Some(0.95))).collect();
        let opts = excalidraw_options(2.0, false);
        let light_h = stroke_to_path(&get_stroke(&light, &opts)).bounding_box().height();
        let heavy_h = stroke_to_path(&get_stroke(&heavy, &opts)).bounding_box().height();
        assert!(heavy_h > light_h * 1.5, "heavy={heavy_h} should be much wider than light={light_h}");
    }
}
