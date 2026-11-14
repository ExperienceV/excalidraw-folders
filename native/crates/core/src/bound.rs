//! Geometry of text bound to a container (a label inside a rectangle,
//! diamond or ellipse). Ported from Excalidraw's `textElement.ts`.

use std::f64::consts::SQRT_2;

use kurbo::{Affine, Point};

use crate::element::{Element, Kind};

/// Space kept between a container's edge and its label.
const PADDING: f64 = 5.0;

/// Whether an element of this kind can hold a label.
pub(crate) fn is_container(kind: &Kind) -> bool {
    matches!(kind, Kind::Rectangle | Kind::Diamond | Kind::Ellipse)
}

/// Largest extent a label may have along a container side of `extent`:
/// the side of the biggest rectangle inscribed in the shape, minus padding.
fn inner_extent(extent: f64, kind: &Kind) -> f64 {
    let inscribed = match kind {
        Kind::Ellipse => (extent / 2.0 * SQRT_2).round(),
        Kind::Diamond => (extent / 2.0).round(),
        _ => extent,
    };
    inscribed - PADDING * 2.0
}

pub(crate) fn max_width(container: &Element) -> f64 {
    inner_extent(container.width, &container.kind)
}

pub(crate) fn max_height(container: &Element) -> f64 {
    inner_extent(container.height, &container.kind)
}

/// Container side needed for a label that is `extent` long on that side.
pub(crate) fn container_size_for(extent: f64, kind: &Kind) -> f64 {
    let padded = extent.ceil() + PADDING * 2.0;
    match kind {
        Kind::Ellipse => (padded / SQRT_2 * 2.0).round(),
        Kind::Diamond => 2.0 * padded,
        _ => padded,
    }
}

/// Offset from a container's corner to the corner of its label area.
fn inset(extent: f64, kind: &Kind) -> f64 {
    PADDING
        + match kind {
            Kind::Ellipse => extent / 2.0 * (1.0 - SQRT_2 / 2.0),
            Kind::Diamond => extent / 4.0,
            _ => 0.0,
        }
}

/// Scene position (`x`, `y`) of a label inside its container, following the
/// label's alignment and the container's rotation.
pub(crate) fn bound_position(container: &Element, label: &Element) -> (f64, f64) {
    let area_x = container.x + inset(container.width, &container.kind);
    let area_y = container.y + inset(container.height, &container.kind);
    let (area_w, area_h) = (max_width(container), max_height(container));
    let x = area_x
        + match label.text_align.as_deref() {
            Some("left") => 0.0,
            Some("right") => area_w - label.width,
            _ => (area_w - label.width) / 2.0,
        };
    let y = area_y
        + match label.vertical_align.as_deref() {
            Some("top") => 0.0,
            Some("bottom") => area_h - label.height,
            _ => (area_h - label.height) / 2.0,
        };
    if container.angle == 0.0 {
        return (x, y);
    }
    // The label rotates with the container, about the label area's center.
    let pivot = Point::new(area_x + area_w / 2.0, area_y + area_h / 2.0);
    let center = Point::new(x + label.width / 2.0, y + label.height / 2.0);
    let rotated = Affine::rotate_about(container.angle, pivot) * center;
    (rotated.x - label.width / 2.0, rotated.y - label.height / 2.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn container(kind: Kind) -> Element {
        Element { kind, x: 100.0, y: 50.0, width: 200.0, height: 100.0, ..Element::default() }
    }

    fn label(align: &str, vertical: &str) -> Element {
        Element {
            kind: Kind::Text,
            width: 60.0,
            height: 25.0,
            text_align: Some(align.into()),
            vertical_align: Some(vertical.into()),
            ..Element::default()
        }
    }

    #[test]
    fn label_area_matches_excalidraw() {
        assert_eq!((max_width(&container(Kind::Rectangle)), max_height(&container(Kind::Rectangle))), (190.0, 90.0));
        assert_eq!((max_width(&container(Kind::Ellipse)), max_height(&container(Kind::Ellipse))), (131.0, 61.0));
        assert_eq!((max_width(&container(Kind::Diamond)), max_height(&container(Kind::Diamond))), (90.0, 40.0));
    }

    #[test]
    fn a_grown_container_fits_its_label() {
        for kind in [Kind::Rectangle, Kind::Ellipse, Kind::Diamond] {
            let mut c = container(kind.clone());
            c.height = container_size_for(123.4, &kind);
            assert!((124.0..125.5).contains(&max_height(&c)), "{kind:?}: {}", max_height(&c));
        }
    }

    #[test]
    fn positions_follow_alignment() {
        let c = container(Kind::Rectangle);
        assert_eq!(bound_position(&c, &label("center", "middle")), (170.0, 87.5));
        assert_eq!(bound_position(&c, &label("left", "top")), (105.0, 55.0));
        assert_eq!(bound_position(&c, &label("right", "bottom")), (235.0, 120.0));
        // Centered labels share the container's center whatever its shape or
        // angle, up to the rounding Excalidraw applies to the label area.
        for kind in [Kind::Ellipse, Kind::Diamond] {
            let c = Element { angle: 0.7, ..container(kind) };
            let (x, y) = bound_position(&c, &label("center", "middle"));
            assert!(Point::new(x + 30.0, y + 12.5).distance(Point::new(200.0, 100.0)) < 0.5);
        }
    }
}
