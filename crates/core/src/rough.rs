//! Hand-drawn stroke generation, adapted from the algorithms of rough.js
//! (MIT, © Preet Shihn), which Excalidraw uses for its sketchy look.
//!
//! Output is deterministic for a given seed, so a shape keeps its wobble
//! between frames and between sessions.

use std::f64::consts::PI;

use kurbo::{BezPath, Point, Vec2};

/// The seeded generator rough.js uses (Park–Miller over 32-bit ints).
#[derive(Clone, Debug)]
pub struct Random {
    seed: i32,
}

impl Random {
    pub fn new(seed: i64) -> Self {
        // rough.js falls back to Math.random for seed 0; we stay deterministic.
        let seed = seed as i32;
        Random { seed: if seed == 0 { 1 } else { seed } }
    }

    /// Uniform in `[0, 1)`.
    pub fn next(&mut self) -> f64 {
        self.seed = self.seed.wrapping_mul(48271);
        f64::from(self.seed & i32::MAX) / 2_147_483_648.0
    }
}

#[derive(Clone, Debug)]
pub struct Options {
    pub roughness: f64,
    pub bowing: f64,
    pub max_randomness_offset: f64,
    /// 1.0 follows the ideal ellipse exactly; lower values let radii drift.
    pub curve_fitting: f64,
    pub curve_step_count: f64,
    /// Draw each stroke once instead of twice (used for dashed lines).
    pub disable_multi_stroke: bool,
    /// Keep segment endpoints exactly on the given vertices.
    pub preserve_vertices: bool,
}

impl Default for Options {
    fn default() -> Self {
        Options {
            roughness: 1.0,
            bowing: 1.0,
            max_randomness_offset: 2.0,
            curve_fitting: 0.95,
            curve_step_count: 9.0,
            disable_multi_stroke: false,
            preserve_vertices: false,
        }
    }
}

/// One piece of an outline: a straight edge or a quadratic corner.
#[derive(Clone, Copy, Debug)]
pub enum Seg {
    Line(Point, Point),
    Quad(Point, Point, Point),
}

pub struct Rough {
    pub opts: Options,
    rng: Random,
}

impl Rough {
    pub fn new(opts: Options, seed: i64) -> Self {
        Rough { opts, rng: Random::new(seed) }
    }

    fn offset(&mut self, min: f64, max: f64, gain: f64) -> f64 {
        self.opts.roughness * gain * (self.rng.next() * (max - min) + min)
    }

    fn offset_opt(&mut self, x: f64, gain: f64) -> f64 {
        self.offset(-x, x, gain)
    }

    fn jitter(&mut self, p: Point, amount: f64) -> Point {
        Point::new(p.x + self.offset_opt(amount, 1.0), p.y + self.offset_opt(amount, 1.0))
    }

    /// One wobbly pass over the segment `a`–`b`. `overlay` is the second,
    /// tighter pass of a double stroke.
    fn line_pass(&mut self, a: Point, b: Point, path: &mut BezPath, overlay: bool) {
        let d = b - a;
        let length_sq = d.hypot2();
        let length = length_sq.sqrt();
        let gain = if length < 200.0 {
            1.0
        } else if length > 500.0 {
            0.4
        } else {
            -0.001_666_8 * length + 1.233_334
        };

        let mut offset = self.opts.max_randomness_offset;
        if offset * offset * 100.0 > length_sq {
            offset = length / 10.0;
        }
        let amount = if overlay { offset / 2.0 } else { offset };

        let diverge = 0.2 + self.rng.next() * 0.2;
        let bow = self.opts.bowing * self.opts.max_randomness_offset / 200.0;
        let mid = Vec2::new(self.offset_opt(bow * d.y, gain), self.offset_opt(-bow * d.x, gain));

        let keep = self.opts.preserve_vertices;
        let end = |this: &mut Self, p: Point| {
            if keep {
                p
            } else {
                Point::new(p.x + this.offset_opt(amount, gain), p.y + this.offset_opt(amount, gain))
            }
        };
        let start = end(self, a);
        path.move_to(start);
        let ctrl = |this: &mut Self, t: f64| {
            let p = a + mid + d * t;
            Point::new(p.x + this.offset_opt(amount, gain), p.y + this.offset_opt(amount, gain))
        };
        let c1 = ctrl(self, diverge);
        let c2 = ctrl(self, 2.0 * diverge);
        let to = end(self, b);
        path.curve_to(c1, c2, to);
    }

    /// A sketchy straight line, drawn twice unless multi-stroke is disabled.
    pub fn line(&mut self, a: Point, b: Point, path: &mut BezPath) {
        self.line_pass(a, b, path, false);
        if !self.opts.disable_multi_stroke {
            self.line_pass(a, b, path, true);
        }
    }

    /// A sketchy polyline through `points`.
    pub fn linear_path(&mut self, points: &[Point], close: bool, path: &mut BezPath) {
        for w in points.windows(2) {
            self.line(w[0], w[1], path);
        }
        if close && points.len() > 2 {
            self.line(points[points.len() - 1], points[0], path);
        }
    }

    /// A sketchy outline made of straight edges and quadratic corners.
    pub fn segments(&mut self, segs: &[Seg], path: &mut BezPath) {
        let amount = 1.0 + self.opts.roughness * 0.2;
        let passes = if self.opts.disable_multi_stroke { 1 } else { 2 };
        for seg in segs {
            match *seg {
                Seg::Line(a, b) => self.line(a, b, path),
                Seg::Quad(a, c, b) => {
                    for _ in 0..passes {
                        let keep = self.opts.preserve_vertices;
                        let from = if keep { a } else { self.jitter(a, amount) };
                        path.move_to(from);
                        let ctrl = self.jitter(c, amount);
                        let to = if keep { b } else { self.jitter(b, amount) };
                        path.quad_to(ctrl, to);
                    }
                }
            }
        }
    }

    /// A sketchy ellipse centered on `center`.
    pub fn ellipse(&mut self, center: Point, width: f64, height: f64, path: &mut BezPath) {
        let (hw, hh) = (width / 2.0, height / 2.0);
        let psq = (PI * 2.0 * ((hw * hw + hh * hh) / 2.0).sqrt()).sqrt();
        let steps = self.opts.curve_step_count;
        let step_count = steps.max(steps / 200f64.sqrt() * psq).ceil();
        let increment = PI * 2.0 / step_count;
        let fit = 1.0 - self.opts.curve_fitting;
        let rx = hw.abs() + self.offset_opt(hw.abs() * fit, 1.0);
        let ry = hh.abs() + self.offset_opt(hh.abs() * fit, 1.0);

        let inner = self.offset(0.4, 1.0, 1.0);
        let overlap = increment * self.offset(0.1, inner, 1.0);
        let first = self.ellipse_points(increment, center, rx, ry, 1.0, overlap);
        curve_through(&first, path);
        if !self.opts.disable_multi_stroke && self.opts.roughness != 0.0 {
            let second = self.ellipse_points(increment, center, rx, ry, 1.5, 0.0);
            curve_through(&second, path);
        }
    }

    fn ellipse_points(
        &mut self,
        increment: f64,
        c: Point,
        rx: f64,
        ry: f64,
        offset: f64,
        overlap: f64,
    ) -> Vec<Point> {
        let at = |k: f64, angle: f64| Point::new(c.x + k * rx * angle.cos(), c.y + k * ry * angle.sin());
        let mut pts = Vec::new();
        if self.opts.roughness == 0.0 {
            let inc = increment / 4.0;
            pts.push(at(1.0, -inc));
            let mut angle = 0.0;
            while angle <= PI * 2.0 {
                pts.push(at(1.0, angle));
                angle += inc;
            }
            pts.push(at(1.0, 0.0));
            pts.push(at(1.0, inc));
        } else {
            let rad_offset = self.offset_opt(0.5, 1.0) - PI / 2.0;
            let p = at(0.9, rad_offset - increment);
            pts.push(self.jitter(p, offset));
            let end_angle = PI * 2.0 + rad_offset - 0.01;
            let mut angle = rad_offset;
            while angle < end_angle {
                let p = at(1.0, angle);
                pts.push(self.jitter(p, offset));
                angle += increment;
            }
            for (k, a) in [
                (1.0, rad_offset + PI * 2.0 + overlap * 0.5),
                (0.98, rad_offset + overlap),
                (0.9, rad_offset + overlap * 0.5),
            ] {
                let p = at(k, a);
                pts.push(self.jitter(p, offset));
            }
        }
        pts
    }

    /// A sketchy smooth curve through `points`.
    pub fn curve(&mut self, points: &[Point], path: &mut BezPath) {
        let r = self.opts.roughness;
        self.curve_pass(points, 1.0 + r * 0.2, path);
        if !self.opts.disable_multi_stroke {
            self.curve_pass(points, 1.5 * (1.0 + r * 0.22), path);
        }
    }

    fn curve_pass(&mut self, points: &[Point], offset: f64, path: &mut BezPath) {
        let (Some(&first), Some(&last)) = (points.first(), points.last()) else {
            return;
        };
        // The spline needs a phantom point before the first and after the last.
        let mut pts = Vec::with_capacity(points.len() + 2);
        let start = self.jitter(first, offset);
        pts.push(start);
        pts.push(start);
        for &p in &points[1..] {
            pts.push(self.jitter(p, offset));
        }
        pts.push(self.jitter(last, offset));
        curve_through(&pts, path);
    }

    /// Parallel hachure lines clipped to `polygon`, `gap` apart, running at
    /// `angle_deg` (screen degrees; negative leans like a forward slash).
    /// With `zigzag`, consecutive lines are joined end to start.
    pub fn hachure(
        &mut self,
        polygon: &[Point],
        gap: f64,
        angle_deg: f64,
        zigzag: bool,
        path: &mut BezPath,
    ) {
        let lines = hachure_lines(polygon, gap.max(0.5), angle_deg);
        for (i, &(a, b)) in lines.iter().enumerate() {
            self.line(a, b, path);
            if zigzag {
                if let Some(&(next, _)) = lines.get(i + 1) {
                    self.line(b, next, path);
                }
            }
        }
    }
}

/// Catmull-Rom-style spline through `pts[1..len-1]`; the first and last
/// points only shape the end tangents.
fn curve_through(pts: &[Point], path: &mut BezPath) {
    if pts.len() < 4 {
        if let [a, .., b] = pts {
            path.move_to(*a);
            path.line_to(*b);
        }
        return;
    }
    path.move_to(pts[1]);
    for i in 1..pts.len() - 2 {
        let (p0, p1, p2, p3) = (pts[i - 1], pts[i], pts[i + 1], pts[i + 2]);
        path.curve_to(p1 + (p2 - p0) / 6.0, p2 + (p1 - p3) / 6.0, p2);
    }
}

/// Scanline fill: rotate the polygon so the hachure direction is horizontal,
/// intersect with evenly spaced rows (even-odd), and rotate the spans back.
fn hachure_lines(polygon: &[Point], gap: f64, angle_deg: f64) -> Vec<(Point, Point)> {
    if polygon.len() < 3 {
        return Vec::new();
    }
    let (sin, cos) = angle_deg.to_radians().sin_cos();
    let fwd = |p: Point| Point::new(p.x * cos + p.y * sin, -p.x * sin + p.y * cos);
    let back = |p: Point| Point::new(p.x * cos - p.y * sin, p.x * sin + p.y * cos);
    let pts: Vec<Point> = polygon.iter().map(|&p| fwd(p)).collect();

    let min_y = pts.iter().map(|p| p.y).fold(f64::INFINITY, f64::min);
    let max_y = pts.iter().map(|p| p.y).fold(f64::NEG_INFINITY, f64::max);
    let mut lines = Vec::new();
    let mut xs = Vec::new();
    let mut y = min_y + gap / 2.0;
    while y < max_y {
        xs.clear();
        for i in 0..pts.len() {
            let (a, b) = (pts[i], pts[(i + 1) % pts.len()]);
            if (a.y <= y) != (b.y <= y) {
                xs.push(a.x + (y - a.y) / (b.y - a.y) * (b.x - a.x));
            }
        }
        xs.sort_by(f64::total_cmp);
        for span in xs.chunks_exact(2) {
            if span[1] - span[0] > 0.5 {
                lines.push((back(Point::new(span[0], y)), back(Point::new(span[1], y))));
            }
        }
        y += gap;
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;
    use kurbo::Shape;

    #[test]
    fn random_matches_rough_js_sequence() {
        // Math.imul(48271, 1) = 48271; (2**31 - 1 & 48271) / 2**31
        let mut r = Random::new(1);
        assert_eq!(r.next(), 48271.0 / 2_147_483_648.0);
        assert_eq!(r.next(), f64::from(48271i32.wrapping_mul(48271) & i32::MAX) / 2_147_483_648.0);
    }

    #[test]
    fn same_seed_same_shape() {
        let draw = |seed| {
            let mut path = BezPath::new();
            let mut rough = Rough::new(Options::default(), seed);
            rough.ellipse(Point::new(50.0, 40.0), 100.0, 80.0, &mut path);
            rough.line(Point::ZERO, Point::new(100.0, 0.0), &mut path);
            path
        };
        assert_eq!(draw(7), draw(7));
        assert_ne!(draw(7), draw(8));
    }

    #[test]
    fn strokes_stay_near_the_ideal_shape() {
        let mut path = BezPath::new();
        let mut rough = Rough::new(Options::default(), 42);
        rough.ellipse(Point::new(0.0, 0.0), 200.0, 100.0, &mut path);
        let b = path.bounding_box();
        assert!((b.width() - 200.0).abs() < 20.0 && (b.height() - 100.0).abs() < 20.0, "{b:?}");

        let mut path = BezPath::new();
        rough.line(Point::new(0.0, 0.0), Point::new(300.0, 0.0), &mut path);
        let b = path.bounding_box();
        assert!(b.height() < 12.0 && (b.width() - 300.0).abs() < 8.0, "{b:?}");
    }

    #[test]
    fn hachure_fills_a_square() {
        let square = [(0.0, 0.0), (100.0, 0.0), (100.0, 100.0), (0.0, 100.0)].map(Point::from);
        let lines = hachure_lines(&square, 10.0, 0.0);
        assert_eq!(lines.len(), 10);
        assert!(lines.iter().all(|(a, b)| a.x.abs() < 1e-9 && (b.x - 100.0).abs() < 1e-9));

        let slanted = hachure_lines(&square, 8.0, -49.0);
        assert!(slanted.len() > 12);
        for (a, b) in slanted {
            // Leaning like "/": x grows while y shrinks, and spans stay inside.
            assert!((b.x - a.x) * (b.y - a.y) < 0.0);
            for p in [a, b] {
                assert!((-1e-6..=100.0 + 1e-6).contains(&p.x) && (-1e-6..=100.0 + 1e-6).contains(&p.y));
            }
        }
    }
}
