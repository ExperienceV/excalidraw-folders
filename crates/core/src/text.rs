//! Text measuring, wrapping and glyph outlines.
//!
//! Glyphs come out as plain paths, so every renderer (GPU, SVG) draws text
//! with the same fill primitive it uses for shapes. Metrics and wrapping
//! follow Excalidraw's `font-metadata.ts`, `textMeasurements.ts` and
//! `textWrapping.ts`.
//!
//! Known gaps: no kerning or complex shaping (advances are summed per
//! character), and line breaking handles spaces, hyphens and CJK but not
//! Excalidraw's full emoji/punctuation rules.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

use kurbo::{Affine, BezPath, Point};
use skrifa::instance::{Location, Size};
use skrifa::outline::{DrawSettings, OutlinePen};
use skrifa::{FontRef, MetadataProvider};

use crate::element::Element;
use crate::font_data::FAMILIES;

pub const DEFAULT_FONT_FAMILY: i64 = 5;
pub const DEFAULT_FONT_SIZE: f64 = 20.0;
/// Families tried, in order, for characters the requested family lacks.
const FALLBACK_FAMILIES: [i64; 2] = [6, 9];
/// Advance, in em, of a character no bundled font can draw.
const MISSING_ADVANCE: f64 = 0.6;
const TAB_SPACES: f64 = 8.0;

/// Vertical metrics Excalidraw hardcodes per family (`FONT_METADATA`).
#[derive(Clone, Copy, Debug)]
pub struct Metrics {
    pub units_per_em: f64,
    pub ascender: f64,
    pub descender: f64,
    /// Default unitless line height for new text.
    pub line_height: f64,
}

pub fn metrics(family: i64) -> Metrics {
    let (units_per_em, ascender, descender, line_height) = match family {
        1 => (1000.0, 886.0, -374.0, 1.25),
        2 => (2048.0, 1577.0, -471.0, 1.15),
        3 => (2048.0, 1900.0, -480.0, 1.2),
        6 => (1000.0, 1011.0, -353.0, 1.25),
        7 => (1000.0, 923.0, -220.0, 1.15),
        8 => (1000.0, 750.0, -250.0, 1.25),
        9 => (2048.0, 1854.0, -434.0, 1.15),
        _ => (1000.0, 886.0, -374.0, 1.25),
    };
    Metrics { units_per_em, ascender, descender, line_height }
}

/// Distance from the top of a line box to its alphabetic baseline.
pub fn vertical_offset(family: i64, font_size: f64, line_height_px: f64) -> f64 {
    let m = metrics(family);
    let em = font_size / m.units_per_em;
    let line_gap = (line_height_px - em * m.ascender + em * m.descender) / 2.0;
    em * m.ascender + line_gap
}

struct Face {
    /// Decoded OpenType data.
    data: Vec<u8>,
    units_per_em: f64,
    /// Regular weight, for variable fonts.
    location: Location,
}

#[derive(Clone)]
struct Glyph {
    /// Outline in em units, y pointing down, origin on the baseline.
    path: Arc<BezPath>,
    /// Advance width in em.
    advance: f64,
}

struct Family {
    faces: Vec<Face>,
    glyphs: Mutex<HashMap<char, Option<Glyph>>>,
}

struct PathPen<'a> {
    path: &'a mut BezPath,
    scale: f64,
}

impl PathPen<'_> {
    fn pt(&self, x: f32, y: f32) -> Point {
        Point::new(f64::from(x) * self.scale, -f64::from(y) * self.scale)
    }
}

impl OutlinePen for PathPen<'_> {
    fn move_to(&mut self, x: f32, y: f32) {
        self.path.move_to(self.pt(x, y));
    }
    fn line_to(&mut self, x: f32, y: f32) {
        self.path.line_to(self.pt(x, y));
    }
    fn quad_to(&mut self, cx0: f32, cy0: f32, x: f32, y: f32) {
        self.path.quad_to(self.pt(cx0, cy0), self.pt(x, y));
    }
    fn curve_to(&mut self, cx0: f32, cy0: f32, cx1: f32, cy1: f32, x: f32, y: f32) {
        self.path.curve_to(self.pt(cx0, cy0), self.pt(cx1, cy1), self.pt(x, y));
    }
    fn close(&mut self) {
        self.path.close_path();
    }
}

impl Family {
    fn load(files: &[&[u8]]) -> Family {
        let faces = files
            .iter()
            .map(|woff2| {
                let data = wuff::decompress_woff2(woff2).expect("bundled font decodes");
                let font = FontRef::new(&data).expect("bundled font parses");
                let units_per_em = f64::from(font.metrics(Size::unscaled(), &Location::default()).units_per_em);
                let location = font.axes().location([("wght", 400.0)]);
                Face { data, units_per_em, location }
            })
            .collect();
        Family { faces, glyphs: Mutex::new(HashMap::new()) }
    }

    fn outline(&self, ch: char) -> Option<Glyph> {
        self.faces.iter().find_map(|face| {
            let font = FontRef::new(&face.data).ok()?;
            let id = font.charmap().map(ch).filter(|id| id.to_u32() != 0)?;
            let advance = font.glyph_metrics(Size::unscaled(), &face.location).advance_width(id)?;
            let mut path = BezPath::new();
            if let Some(glyph) = font.outline_glyphs().get(id) {
                let mut pen = PathPen { path: &mut path, scale: 1.0 / face.units_per_em };
                let settings = DrawSettings::unhinted(Size::unscaled(), &face.location);
                glyph.draw(settings, &mut pen).ok()?;
            }
            Some(Glyph { path: Arc::new(path), advance: f64::from(advance) / face.units_per_em })
        })
    }

    fn glyph(&self, ch: char) -> Option<Glyph> {
        let mut cache = self.glyphs.lock().unwrap_or_else(|e| e.into_inner());
        cache.entry(ch).or_insert_with(|| self.outline(ch)).clone()
    }
}

fn family(id: i64) -> &'static Family {
    static LOADED: [OnceLock<Family>; FAMILIES.len()] = [const { OnceLock::new() }; FAMILIES.len()];
    let slot = FAMILIES
        .iter()
        .position(|(ids, _)| ids.contains(&id))
        .or_else(|| FAMILIES.iter().position(|(ids, _)| ids.contains(&DEFAULT_FONT_FAMILY)))
        .expect("the default family is bundled");
    LOADED[slot].get_or_init(|| Family::load(FAMILIES[slot].1))
}

fn glyph(family_id: i64, ch: char) -> Option<Glyph> {
    family(family_id)
        .glyph(ch)
        .or_else(|| FALLBACK_FAMILIES.iter().find_map(|&id| family(id).glyph(ch)))
}

/// Advance of one character, in em.
fn advance(family_id: i64, ch: char) -> f64 {
    match ch {
        '\t' => TAB_SPACES * advance(family_id, ' '),
        '\r' | '\n' => 0.0,
        _ => glyph(family_id, ch).map_or(MISSING_ADVANCE, |g| g.advance),
    }
}

/// Width of a single line of text, in pixels.
pub fn line_width(text: &str, family_id: i64, font_size: f64) -> f64 {
    text.chars().map(|ch| advance(family_id, ch)).sum::<f64>() * font_size
}

/// Size of a block of text: widest line, and one line box per line (empty
/// lines included).
pub fn measure(text: &str, family_id: i64, font_size: f64, line_height: f64) -> (f64, f64) {
    let width = text.split('\n').map(|l| line_width(l, family_id, font_size)).fold(0.0, f64::max);
    (width, text.split('\n').count() as f64 * font_size * line_height)
}

/// Appends the outlines of `text` (one line) to `path`, with the pen
/// starting at `origin` on the baseline.
pub fn append_line(text: &str, family_id: i64, font_size: f64, origin: Point, path: &mut BezPath) {
    let mut x = origin.x;
    for ch in text.chars() {
        if let Some(g) = glyph(family_id, ch).filter(|_| !ch.is_control()) {
            let place = Affine::translate((x, origin.y)) * Affine::scale(font_size);
            path.extend(place * (*g.path).clone());
        }
        x += advance(family_id, ch) * font_size;
    }
}

/// A visual line: byte offsets into the source text, excluding the line
/// break (and any whitespace trimmed at a soft wrap).
#[derive(Clone, Debug, PartialEq)]
pub struct Line {
    pub start: usize,
    pub end: usize,
}

fn is_cjk(ch: char) -> bool {
    matches!(u32::from(ch), 0x2E80..=0x9FFF | 0xAC00..=0xD7AF | 0xF900..=0xFAFF | 0xFF00..=0xFFEF)
}

/// Splits a hard line into the pieces a line may break between: each
/// whitespace and CJK character alone, and words (breakable after a hyphen).
fn tokens(line: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut start = 0;
    for (i, ch) in line.char_indices() {
        let next = i + ch.len_utf8();
        if ch.is_whitespace() || is_cjk(ch) {
            if start < i {
                out.push(&line[start..i]);
            }
            out.push(&line[i..next]);
            start = next;
        } else if ch == '-' {
            out.push(&line[start..next]);
            start = next;
        }
    }
    if start < line.len() {
        out.push(&line[start..]);
    }
    out
}

/// Breaks `text` into visual lines no wider than `max_width` (hard line
/// breaks only when `None`). Port of Excalidraw's `getWrappedTextLines`.
pub fn wrap(text: &str, family_id: i64, font_size: f64, max_width: Option<f64>) -> Vec<Line> {
    let width = |s: &str| line_width(s, family_id, font_size);
    let max_width = max_width.filter(|w| w.is_finite() && *w >= 0.0);
    let mut lines = Vec::new();
    let mut offset = 0;
    for hard in text.split('\n') {
        let hard_start = offset;
        offset += hard.len() + 1;
        let Some(max) = max_width.filter(|max| width(hard) > *max) else {
            lines.push(Line { start: hard_start, end: hard_start + hard.len() });
            continue;
        };

        // Current visual line, as offsets into `hard`.
        let (mut start, mut end) = (0, 0);
        let mut pos = 0;
        let soft_break = |start: usize, end: usize| {
            let kept = hard[start..end].trim_end().len();
            Line { start: hard_start + start, end: hard_start + start + kept }
        };
        let mut pending = tokens(hard).into_iter().peekable();
        while let Some(&token) = pending.peek() {
            let token_end = pos + token.len();
            let is_space = token.chars().all(char::is_whitespace);
            if is_space || width(&hard[start..token_end]) <= max {
                // Trailing whitespace never forces a break.
                end = token_end;
            } else if start == end {
                // A single word wider than the line: break it by characters.
                for (i, ch) in token.char_indices() {
                    let ch_end = pos + i + ch.len_utf8();
                    if width(&hard[start..ch_end]) > max && start < pos + i {
                        lines.push(Line { start: hard_start + start, end: hard_start + pos + i });
                        start = pos + i;
                    }
                    end = ch_end;
                }
            } else {
                lines.push(soft_break(start, end));
                (start, end) = (pos, pos);
                continue;
            }
            pos = token_end;
            pending.next();
        }
        if start < end {
            // Keep trailing whitespace only while it still fits.
            let mut kept = hard[start..end].trim_end().len();
            for ch in hard[start + kept..end].chars() {
                if width(&hard[start..start + kept + ch.len_utf8()]) > max {
                    break;
                }
                kept += ch.len_utf8();
            }
            lines.push(Line { start: hard_start + start, end: hard_start + start + kept });
        }
    }
    lines
}

/// `text` with soft line breaks inserted, as stored in an element's `text`.
pub fn wrap_text(text: &str, family_id: i64, font_size: f64, max_width: f64) -> String {
    let lines = wrap(text, family_id, font_size, Some(max_width));
    lines.iter().map(|l| &text[l.start..l.end]).collect::<Vec<_>>().join("\n")
}

/// Left edge of a line of `width` inside a text element, per `textAlign`.
pub fn line_x(el: &Element, width: f64) -> f64 {
    match el.text_align.as_deref() {
        Some("center") => (el.width - width) / 2.0,
        Some("right") => el.width - width,
        _ => 0.0,
    }
}

/// Outlines of a text element's glyphs, in element-local coordinates.
pub fn text_path(el: &Element) -> BezPath {
    let family_id = el.font_family.unwrap_or(DEFAULT_FONT_FAMILY);
    let size = el.font_size.unwrap_or(DEFAULT_FONT_SIZE);
    let line_height = size * el.line_height.unwrap_or(metrics(family_id).line_height);
    let baseline = vertical_offset(family_id, size, line_height);
    let mut path = BezPath::new();
    for (i, line) in el.text.as_deref().unwrap_or("").split('\n').enumerate() {
        let x = line_x(el, line_width(line, family_id, size));
        append_line(line, family_id, size, Point::new(x, i as f64 * line_height + baseline), &mut path);
    }
    path
}

#[cfg(test)]
mod tests {
    use super::*;
    use kurbo::Shape;

    #[test]
    fn every_bundled_family_draws_latin_text() {
        for id in [1, 2, 3, 5, 6, 7, 8, 9] {
            let w = line_width("Hola", id, 20.0);
            assert!((20.0..70.0).contains(&w), "family {id}: width {w}");
            let mut path = BezPath::new();
            append_line("Hola", id, 20.0, Point::new(0.0, 0.0), &mut path);
            let b = path.bounding_box();
            // Glyphs sit on the baseline and rise above it (y points down).
            assert!(b.y0 < -8.0 && b.y1 < 2.0 && b.width() > 15.0, "family {id}: {b:?}");
        }
    }

    #[test]
    fn accents_and_fallback_characters_have_glyphs() {
        for ch in "áéíóúñ¿¡".chars() {
            assert!(glyph(5, ch).is_some_and(|g| !g.path.is_empty()), "Excalifont lacks {ch}");
            assert!(glyph(6, ch).is_some_and(|g| !g.path.is_empty()), "Nunito lacks {ch}");
        }
        assert_eq!(advance(5, '\u{10FFFF}'), MISSING_ADVANCE);
        assert_eq!(advance(5, '\t'), 8.0 * advance(5, ' '));
    }

    #[test]
    fn measure_counts_empty_lines() {
        let (w, h) = measure("ab\n\nabcd", 5, 20.0, 1.25);
        assert_eq!(w, line_width("abcd", 5, 20.0));
        assert_eq!(h, 75.0);
        assert_eq!(vertical_offset(5, 20.0, 25.0), 20.0 * 0.886 + (25.0 - 20.0 * 1.26) / 2.0);
    }

    fn wrapped(text: &str, max: f64) -> Vec<&str> {
        wrap(text, 5, 20.0, Some(max)).iter().map(|l| &text[l.start..l.end]).collect()
    }

    #[test]
    fn wraps_at_spaces_and_trims_them() {
        let max = line_width("hello world", 5, 20.0) + 1.0;
        assert_eq!(wrapped("hello world again and again", max), ["hello world", "again and", "again"]);
        assert_eq!(wrapped("short\nhello world again", max), ["short", "hello world", "again"]);
        // Offsets skip the trimmed space and the hard break.
        let lines = wrap("hello world again", 5, 20.0, Some(max));
        assert_eq!(lines, [Line { start: 0, end: 11 }, Line { start: 12, end: 17 }]);
    }

    #[test]
    fn breaks_long_words_and_after_hyphens() {
        let max = line_width("abcd", 5, 20.0) + 0.1;
        let lines = wrapped("abcdefghij", max);
        assert!(lines.len() >= 3 && lines.concat() == "abcdefghij", "{lines:?}");
        assert!(lines.iter().all(|l| line_width(l, 5, 20.0) <= max));

        let max = line_width("well-", 5, 20.0).max(line_width("known", 5, 20.0)) + 1.0;
        assert_eq!(wrapped("well-known", max), ["well-", "known"]);
    }

    #[test]
    fn without_a_limit_only_hard_breaks_split() {
        let lines = wrap("a b c\n\nd", 5, 20.0, None);
        assert_eq!(lines, [Line { start: 0, end: 5 }, Line { start: 6, end: 6 }, Line { start: 7, end: 8 }]);
        assert_eq!(wrap_text("a b", 5, 20.0, 1e9), "a b");
    }
}
