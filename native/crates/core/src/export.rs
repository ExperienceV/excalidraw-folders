//! Export of a whole document, without editor chrome. Uses the same draw
//! ops as the on-screen renderer, so the output matches what the window shows.

use std::fmt::Write;
use std::rc::Rc;

use kurbo::{Affine, Rect, Shape, Size};

use crate::draw::{Color, Crop, Frame, Item, Style};
use crate::element::{Document, Element};
use crate::shape::element_ops;

const PADDING: f64 = 10.0;

fn paint(c: Color) -> String {
    format!("rgb({},{},{})", c.r, c.g, c.b)
}

fn live(doc: &Document) -> impl Iterator<Item = &Element> {
    doc.elements.iter().filter(|e| !e.is_deleted)
}

/// Scene area covered by the drawing, with Excalidraw's export padding.
fn bounds(doc: &Document) -> Rect {
    live(doc)
        .map(Element::scene_bounds)
        .reduce(|a, b| a.union(b))
        .unwrap_or(Rect::ZERO)
        .inflate(PADDING, PADDING)
}

pub fn background(doc: &Document) -> Color {
    doc.app_state["viewBackgroundColor"].as_str().and_then(Color::parse).unwrap_or(Color::WHITE)
}

/// A display list of the whole drawing at `scale`, plus its pixel size,
/// for raster export.
pub fn to_frame(doc: &Document, scale: f64) -> (Frame, u32, u32) {
    let bounds = bounds(doc);
    let view = Affine::scale(scale) * Affine::translate((-bounds.x0, -bounds.y0));
    let items = live(doc)
        .map(|el| Item {
            transform: view * el.transform(),
            opacity: (el.opacity / 100.0) as f32,
            ops: Rc::new(element_ops(el)),
        })
        .filter(|item| !item.ops.is_empty())
        .collect();
    let pixels = |extent: f64| (extent * scale).ceil().max(1.0) as u32;
    (Frame { background: background(doc), items }, pixels(bounds.width()), pixels(bounds.height()))
}

/// Writes an image element: the embedded bitmap stretched to `size`, or a
/// placeholder box if the document does not hold its data.
fn image_svg(out: &mut String, href: Option<&str>, size: Size, crop: Option<Crop>, flip: (bool, bool), placeholder: Color) {
    let (w, h) = (size.width, size.height);
    let Some(href) = href else {
        let _ = write!(out, "\n  <rect width=\"{w}\" height=\"{h}\" fill=\"{}\"/>", paint(placeholder));
        return;
    };
    let mirror = |flipped: bool, extent: f64| if flipped { (-1, extent) } else { (1, 0.0) };
    let ((sx, tx), (sy, ty)) = (mirror(flip.0, w), mirror(flip.1, h));
    let _ = write!(out, "\n  <g transform=\"matrix({sx} 0 0 {sy} {tx} {ty})\">");
    match crop {
        Some(c) => {
            let _ = write!(
                out,
                "<svg width=\"{w}\" height=\"{h}\" viewBox=\"{} {} {} {}\" preserveAspectRatio=\"none\">\
                 <image href=\"{href}\" width=\"{}\" height=\"{}\" preserveAspectRatio=\"none\"/></svg>",
                c.x, c.y, c.width, c.height, c.natural_width, c.natural_height
            );
        }
        None => {
            let _ = write!(out, "<image href=\"{href}\" width=\"{w}\" height=\"{h}\" preserveAspectRatio=\"none\"/>");
        }
    }
    out.push_str("</g>");
}

pub fn to_svg(doc: &Document) -> String {
    let bounds = bounds(doc);
    let background = background(doc);

    let (w, h) = (bounds.width(), bounds.height());
    let mut out = format!(
        r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {w} {h}" width="{w}" height="{h}">"#
    );
    let _ = write!(out, "\n<rect width=\"100%\" height=\"100%\" fill=\"{}\"/>", paint(background));

    let origin = Affine::translate((-bounds.x0, -bounds.y0));
    for el in live(doc) {
        let ops = element_ops(el);
        if ops.is_empty() {
            continue;
        }
        let [a, b, c, d, e, f] = (origin * el.transform()).as_coeffs();
        let _ = write!(out, "\n<g transform=\"matrix({a} {b} {c} {d} {e} {f})\"");
        if el.opacity < 100.0 {
            let _ = write!(out, " opacity=\"{}\"", el.opacity / 100.0);
        }
        out.push('>');
        for op in &ops {
            if let Style::Image { file_id, crop, flip_x, flip_y } = &op.style {
                let size = op.path.bounding_box().size();
                image_svg(&mut out, doc.files[file_id]["dataURL"].as_str(), size, *crop, (*flip_x, *flip_y), op.color);
                continue;
            }
            let _ = write!(out, "\n  <path d=\"{}\"", op.path.to_svg());
            let alpha = f64::from(op.color.a) / 255.0;
            match &op.style {
                Style::Fill => {
                    let _ = write!(out, " fill=\"{}\"", paint(op.color));
                    if op.color.a < 255 {
                        let _ = write!(out, " fill-opacity=\"{alpha}\"");
                    }
                }
                Style::Stroke { width, dash } => {
                    let _ = write!(
                        out,
                        " fill=\"none\" stroke=\"{}\" stroke-width=\"{width}\" stroke-linecap=\"round\" stroke-linejoin=\"round\"",
                        paint(op.color)
                    );
                    if op.color.a < 255 {
                        let _ = write!(out, " stroke-opacity=\"{alpha}\"");
                    }
                    if !dash.is_empty() {
                        let list: Vec<String> = dash.iter().map(f64::to_string).collect();
                        let _ = write!(out, " stroke-dasharray=\"{}\"", list.join(" "));
                    }
                }
                Style::Image { .. } => unreachable!("images are written above"),
            }
            out.push_str("/>");
        }
        out.push_str("\n</g>");
    }
    out.push_str("\n</svg>\n");
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::element::Kind;

    fn doc() -> Document {
        let rect = Element { x: 50.0, y: 70.0, width: 100.0, height: 40.0, opacity: 50.0, ..Element::default() };
        let gone = Element { x: -900.0, is_deleted: true, ..rect.clone() };
        let text = Element { kind: Kind::Text, ..rect.clone() };
        Document { elements: vec![rect, gone, text], ..Document::default() }
    }

    #[test]
    fn frame_is_cropped_to_live_content_with_padding() {
        let (frame, w, h) = to_frame(&doc(), 2.0);
        assert_eq!((w, h), (240, 120));
        assert_eq!(frame.items.len(), 1, "deleted and unrendered elements are skipped");
        assert_eq!(frame.items[0].transform.as_coeffs(), [2.0, 0.0, 0.0, 2.0, 20.0, 20.0]);
        assert_eq!(frame.items[0].opacity, 0.5);
    }

    #[test]
    fn svg_embeds_images_or_a_placeholder() {
        let mut image = Element { kind: Kind::Image, width: 80.0, height: 40.0, ..Element::default() };
        image.extra.insert("fileId".into(), "f1".into());
        image.extra.insert("scale".into(), serde_json::json!([1, -1]));
        let mut doc = Document { elements: vec![image], ..Document::default() };
        assert!(to_svg(&doc).contains(r#"<rect width="80" height="40" fill="rgb(222,226,230)"/>"#));

        doc.files = serde_json::json!({"f1": {"dataURL": "data:image/png;base64,AAAA"}});
        let svg = to_svg(&doc);
        assert!(svg.contains(r#"<g transform="matrix(1 0 0 -1 0 40)"><image href="data:image/png;base64,AAAA" width="80" height="40""#));
    }

    #[test]
    fn svg_has_one_group_per_drawn_element() {
        let svg = to_svg(&doc());
        assert!(svg.starts_with(r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 120 60""#));
        assert_eq!(svg.matches("<g ").count(), 1);
        assert!(svg.contains(r#"transform="matrix(1 0 0 1 10 10)" opacity="0.5""#));
    }
}
