//! Renderer-agnostic drawing primitives. The editor emits these; a platform
//! shell (vello, SVG, ...) only has to know how to fill and stroke paths.

use std::rc::Rc;

use kurbo::{Affine, BezPath};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Color {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

impl Color {
    pub const TRANSPARENT: Color = Color::rgba(0, 0, 0, 0);
    pub const WHITE: Color = Color::rgba(255, 255, 255, 255);

    pub const fn rgba(r: u8, g: u8, b: u8, a: u8) -> Self {
        Color { r, g, b, a }
    }

    /// Parses the color strings Excalidraw writes: `transparent`, `#rgb`,
    /// `#rrggbb` and `#rrggbbaa`.
    pub fn parse(s: &str) -> Option<Color> {
        let s = s.trim();
        if s.eq_ignore_ascii_case("transparent") {
            return Some(Color::TRANSPARENT);
        }
        let hex = s.strip_prefix('#')?;
        if !hex.is_ascii() {
            return None;
        }
        let byte = |i: usize| u8::from_str_radix(&hex[i..i + 2], 16).ok();
        let nibble = |i: usize| u8::from_str_radix(&hex[i..i + 1], 16).ok().map(|v| v * 17);
        match hex.len() {
            3 => Some(Color::rgba(nibble(0)?, nibble(1)?, nibble(2)?, 255)),
            6 => Some(Color::rgba(byte(0)?, byte(2)?, byte(4)?, 255)),
            8 => Some(Color::rgba(byte(0)?, byte(2)?, byte(4)?, byte(6)?)),
            _ => None,
        }
    }

    pub fn is_transparent(self) -> bool {
        self.a == 0
    }
}

/// The part of a bitmap an image element shows, in source pixels, and the
/// bitmap's full size.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Crop {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
    pub natural_width: f64,
    pub natural_height: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Style {
    Fill,
    /// Round caps and joins are implied. An empty `dash` means a solid line.
    Stroke { width: f64, dash: Vec<f64> },
    /// The bitmap stored under `file_id` in the document's `files`,
    /// stretched over the path's bounding box. The op's color is what to
    /// paint when the bitmap cannot be shown.
    Image { file_id: String, crop: Option<Crop>, flip_x: bool, flip_y: bool },
}

#[derive(Clone, Debug, PartialEq)]
pub struct DrawOp {
    pub path: BezPath,
    pub color: Color,
    pub style: Style,
}

impl DrawOp {
    pub fn fill(path: BezPath, color: Color) -> Self {
        DrawOp { path, color, style: Style::Fill }
    }

    pub fn stroke(path: BezPath, color: Color, width: f64) -> Self {
        DrawOp { path, color, style: Style::Stroke { width, dash: Vec::new() } }
    }
}

/// A group of ops sharing a transform and a group opacity.
#[derive(Clone, Debug)]
pub struct Item {
    pub transform: Affine,
    pub opacity: f32,
    pub ops: Rc<Vec<DrawOp>>,
}

/// Everything needed to paint one frame, back to front, in logical pixels.
#[derive(Clone, Debug)]
pub struct Frame {
    pub background: Color,
    pub items: Vec<Item>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_excalidraw_colors() {
        assert_eq!(Color::parse("#1e1e1e"), Some(Color::rgba(30, 30, 30, 255)));
        assert_eq!(Color::parse("#fff"), Some(Color::WHITE));
        assert_eq!(Color::parse("#ff000080"), Some(Color::rgba(255, 0, 0, 128)));
        assert_eq!(Color::parse("transparent"), Some(Color::TRANSPARENT));
        assert_eq!(Color::parse("#12"), None);
        assert_eq!(Color::parse("#ññ"), None);
    }
}
