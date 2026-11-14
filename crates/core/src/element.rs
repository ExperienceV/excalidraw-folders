//! Scene model, kept wire-compatible with the `.excalidraw` JSON format.
//!
//! Fields the engine does not understand yet are preserved in `extra`, so
//! loading and saving a file made by Excalidraw does not drop data.

use kurbo::{Affine, Point, Rect};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(from = "String", into = "String")]
pub enum Kind {
    Rectangle,
    Diamond,
    Ellipse,
    Line,
    Arrow,
    Freedraw,
    Text,
    Image,
    /// A type this engine does not know; kept verbatim for round-tripping.
    Other(String),
}

impl From<String> for Kind {
    fn from(s: String) -> Self {
        match s.as_str() {
            "rectangle" => Kind::Rectangle,
            "diamond" => Kind::Diamond,
            "ellipse" => Kind::Ellipse,
            "line" => Kind::Line,
            "arrow" => Kind::Arrow,
            "freedraw" => Kind::Freedraw,
            "text" => Kind::Text,
            "image" => Kind::Image,
            _ => Kind::Other(s),
        }
    }
}

impl From<Kind> for String {
    fn from(k: Kind) -> Self {
        match k {
            Kind::Rectangle => "rectangle",
            Kind::Diamond => "diamond",
            Kind::Ellipse => "ellipse",
            Kind::Line => "line",
            Kind::Arrow => "arrow",
            Kind::Freedraw => "freedraw",
            Kind::Text => "text",
            Kind::Image => "image",
            Kind::Other(s) => return s,
        }
        .to_owned()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum FillStyle {
    Hachure,
    CrossHatch,
    Solid,
    Zigzag,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum StrokeStyle {
    Solid,
    Dashed,
    Dotted,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Roundness {
    /// 1 = legacy, 2 = proportional radius, 3 = adaptive radius.
    #[serde(rename = "type")]
    pub kind: u8,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<f64>,
}

impl Roundness {
    pub const PROPORTIONAL: u8 = 2;
    pub const ADAPTIVE: u8 = 3;
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Element {
    pub id: String,
    #[serde(rename = "type")]
    pub kind: Kind,
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
    /// Radians, clockwise, about the center of the element's bounds.
    pub angle: f64,
    pub stroke_color: String,
    pub background_color: String,
    pub fill_style: FillStyle,
    pub stroke_width: f64,
    pub stroke_style: StrokeStyle,
    pub roughness: f64,
    /// 0..=100.
    pub opacity: f64,
    pub roundness: Option<Roundness>,
    pub seed: i64,
    pub version: i64,
    pub version_nonce: i64,
    pub is_deleted: bool,
    pub group_ids: Vec<String>,
    /// Line, arrow and freedraw vertices, relative to (`x`, `y`).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub points: Vec<[f64; 2]>,
    /// Per-point stylus pressure (0.0..=1.0) for freedraw strokes.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub pressures: Vec<f64>,
    /// Whether freedraw pressure is simulated from pointer velocity.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub simulate_pressure: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub start_arrowhead: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub end_arrowhead: Option<String>,
    /// Text as displayed, with soft line breaks already inserted.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    /// Text as typed, before wrapping.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub original_text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub font_size: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub font_family: Option<i64>,
    /// `left`, `center` or `right`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text_align: Option<String>,
    /// `top`, `middle` or `bottom`; only meaningful inside a container.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub vertical_align: Option<String>,
    /// Unitless multiple of `font_size`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub line_height: Option<f64>,
    /// The shape this text is the label of.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub container_id: Option<String>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

impl Default for Element {
    fn default() -> Self {
        Element {
            id: String::new(),
            kind: Kind::Rectangle,
            x: 0.0,
            y: 0.0,
            width: 0.0,
            height: 0.0,
            angle: 0.0,
            stroke_color: "#1e1e1e".to_owned(),
            background_color: "transparent".to_owned(),
            fill_style: FillStyle::Solid,
            stroke_width: 2.0,
            stroke_style: StrokeStyle::Solid,
            roughness: 1.0,
            opacity: 100.0,
            roundness: None,
            seed: 1,
            version: 1,
            version_nonce: 0,
            is_deleted: false,
            group_ids: Vec::new(),
            points: Vec::new(),
            pressures: Vec::new(),
            simulate_pressure: None,
            start_arrowhead: None,
            end_arrowhead: None,
            text: None,
            original_text: None,
            font_size: None,
            font_family: None,
            text_align: None,
            vertical_align: None,
            line_height: None,
            container_id: None,
            extra: Map::new(),
        }
    }
}

impl Element {
    /// Whether the geometry lives in `points` rather than `width`/`height`.
    pub fn is_linear(&self) -> bool {
        matches!(self.kind, Kind::Line | Kind::Arrow | Kind::Freedraw)
    }

    pub fn local_points(&self) -> Vec<Point> {
        self.points.iter().map(|p| Point::new(p[0], p[1])).collect()
    }

    /// Unrotated bounds relative to (`x`, `y`).
    pub fn local_bounds(&self) -> Rect {
        if self.is_linear() && !self.points.is_empty() {
            let first = Point::new(self.points[0][0], self.points[0][1]);
            self.points[1..]
                .iter()
                .fold(Rect::from_points(first, first), |r, p| r.union_pt(Point::new(p[0], p[1])))
        } else {
            Rect::new(0.0, 0.0, self.width, self.height)
        }
    }

    /// Maps element-local coordinates to scene coordinates.
    pub fn transform(&self) -> Affine {
        let local = Affine::translate((self.x, self.y));
        if self.angle == 0.0 {
            local
        } else {
            local * Affine::rotate_about(self.angle, self.local_bounds().center())
        }
    }

    /// Axis-aligned bounds in scene coordinates, rotation included.
    pub fn scene_bounds(&self) -> Rect {
        self.transform().transform_rect_bbox(self.local_bounds())
    }
}

/// A `.excalidraw` file.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Document {
    #[serde(rename = "type")]
    pub kind: String,
    pub version: u32,
    pub source: String,
    pub elements: Vec<Element>,
    pub app_state: Value,
    pub files: Value,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

impl Default for Document {
    fn default() -> Self {
        Document {
            kind: "excalidraw".to_owned(),
            version: 2,
            source: "pizarra".to_owned(),
            elements: Vec::new(),
            app_state: Value::Object(Map::new()),
            files: Value::Object(Map::new()),
            extra: Map::new(),
        }
    }
}

impl Document {
    pub fn from_json(json: &str) -> serde_json::Result<Self> {
        serde_json::from_str(json)
    }

    /// Serializes for saving. Like Excalidraw, deleted elements are dropped.
    pub fn to_json(&self) -> String {
        let mut doc = self.clone();
        doc.elements.retain(|e| !e.is_deleted);
        serde_json::to_string_pretty(&doc).expect("document is always serializable")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r##"{
      "type": "excalidraw", "version": 2, "source": "https://excalidraw.com",
      "elements": [
        {"id": "a", "type": "rectangle", "x": 10, "y": 20, "width": 100, "height": 50,
         "angle": 0, "strokeColor": "#1e1e1e", "backgroundColor": "#ffc9c9",
         "fillStyle": "cross-hatch", "strokeWidth": 2, "strokeStyle": "dashed",
         "roughness": 1, "opacity": 100, "groupIds": [], "frameId": null,
         "roundness": {"type": 3}, "seed": 1234, "version": 5, "versionNonce": 99,
         "isDeleted": false, "boundElements": [{"id": "b", "type": "arrow"}],
         "updated": 1700000000000, "link": null, "locked": false, "index": "a0"},
        {"id": "b", "type": "arrow", "x": 0, "y": 0, "width": 30, "height": 40,
         "points": [[0, 0], [30, -40]], "endArrowhead": "arrow", "startArrowhead": null,
         "startBinding": {"elementId": "a", "focus": 0.1, "gap": 4}},
        {"id": "c", "type": "magicframe", "x": 0, "y": 0, "width": 1, "height": 1}
      ],
      "appState": {"viewBackgroundColor": "#ffffff"}, "files": {}
    }"##;

    #[test]
    fn round_trips_unknown_fields_and_types() {
        let doc = Document::from_json(SAMPLE).unwrap();
        assert_eq!(doc.elements[0].fill_style, FillStyle::CrossHatch);
        assert_eq!(doc.elements[0].extra["index"], "a0");
        assert_eq!(doc.elements[1].end_arrowhead.as_deref(), Some("arrow"));
        assert_eq!(doc.elements[2].kind, Kind::Other("magicframe".into()));

        let again = Document::from_json(&doc.to_json()).unwrap();
        assert_eq!(doc, again);
        let json: Value = serde_json::from_str(&doc.to_json()).unwrap();
        assert_eq!(json["elements"][1]["startBinding"]["elementId"], "a");
        assert_eq!(json["elements"][2]["type"], "magicframe");
        assert!(json["elements"][0].get("points").is_none());
    }

    #[test]
    fn linear_bounds_follow_points() {
        let doc = Document::from_json(SAMPLE).unwrap();
        assert_eq!(doc.elements[1].local_bounds(), Rect::new(0.0, -40.0, 30.0, 0.0));
    }
}
