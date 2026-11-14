//! Editor chrome: the toolbar and the style panel.
//!
//! Laid out from scratch whenever it is needed (for painting and for
//! hit-testing alike), so there is no widget state to keep in sync.

use kurbo::{Affine, BezPath, Circle, Point, Rect, RoundedRect, Shape, Size};

use crate::draw::{Color, DrawOp, Style as Paint};
use crate::element::{FillStyle, StrokeStyle};
use crate::rough::{Options, Rough};
use crate::text;

pub(crate) const ACCENT: Color = Color::rgba(105, 101, 219, 255);
const INK: Color = Color::rgba(30, 30, 30, 255);
const ACTIVE_INK: Color = Color::rgba(3, 0, 100, 255);
const ACTIVE_BG: Color = Color::rgba(224, 223, 255, 255);
const BUTTON_BG: Color = Color::rgba(236, 236, 244, 255);
const BORDER: Color = Color::rgba(214, 214, 214, 255);
const LABEL: Color = Color::rgba(73, 80, 87, 255);

const TOOLBAR_TOP: f64 = 12.0;
const TOOLBAR_PADDING: f64 = 6.0;
const TOOL_BUTTON: f64 = 36.0;
const TOOL_GAP: f64 = 4.0;

const PANEL_LEFT: f64 = 12.0;
const PANEL_TOP: f64 = 76.0;
const PANEL_PADDING: f64 = 12.0;
const BUTTON: f64 = 28.0;
const BUTTON_GAP: f64 = 6.0;
const LABEL_SIZE: f64 = 11.0;
const LABEL_HEIGHT: f64 = 16.0;
const SECTION_GAP: f64 = 8.0;
const SLIDER_HEIGHT: f64 = 18.0;
const KNOB_RADIUS: f64 = 7.0;
const UI_FONT: i64 = 6;
/// Icons are designed in a square of this size, centered in their button.
const ICON: f64 = 20.0;

pub(crate) const STROKE_COLORS: [&str; 5] = ["#1e1e1e", "#e03131", "#2f9e44", "#1971c2", "#f08c00"];
pub(crate) const BACKGROUND_COLORS: [&str; 5] = ["transparent", "#ffc9c9", "#b2f2bb", "#a5d8ff", "#ffec99"];

/// Excalidraw's palette: three plain colors, then five shades per hue.
const PALETTE_BASICS: [&str; 3] = ["transparent", "#ffffff", "#1e1e1e"];
const PALETTE_HUES: [[&str; 5]; 12] = [
    ["#f8f9fa", "#e9ecef", "#ced4da", "#868e96", "#343a40"],
    ["#f8f1ee", "#eaddd7", "#d2bab0", "#a18072", "#846358"],
    ["#fff5f5", "#ffc9c9", "#ff8787", "#fa5252", "#e03131"],
    ["#fff0f6", "#fcc2d7", "#f783ac", "#e64980", "#c2255c"],
    ["#f8f0fc", "#eebefa", "#da77f2", "#be4bdb", "#9c36b5"],
    ["#f3f0ff", "#d0bfff", "#9775fa", "#7950f2", "#6741d9"],
    ["#e7f5ff", "#a5d8ff", "#4dabf7", "#228be6", "#1971c2"],
    ["#e3fafc", "#99e9f2", "#3bc9db", "#15aabf", "#0c8599"],
    ["#e6fcf5", "#96f2d7", "#38d9a9", "#12b886", "#099268"],
    ["#ebfbee", "#b2f2bb", "#69db7c", "#40c057", "#2f9e44"],
    ["#fff9db", "#ffec99", "#ffd43b", "#fab005", "#f08c00"],
    ["#fff4e6", "#ffd8a8", "#ffa94d", "#fd7e14", "#e8590c"],
];
const PICKER_SWATCH: f64 = 24.0;
const PICKER_GAP: f64 = 5.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tool {
    Hand,
    Select,
    Rectangle,
    Diamond,
    Ellipse,
    Arrow,
    Line,
    Freedraw,
    Text,
    /// Not a mode: picking it asks the shell for an image to insert.
    Image,
}

impl Tool {
    /// Toolbar order, same as Excalidraw's.
    pub const ALL: [Tool; 10] = [
        Tool::Hand,
        Tool::Select,
        Tool::Rectangle,
        Tool::Diamond,
        Tool::Ellipse,
        Tool::Arrow,
        Tool::Line,
        Tool::Freedraw,
        Tool::Text,
        Tool::Image,
    ];
}

/// Which color a palette popup edits.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ColorTarget {
    Stroke,
    Background,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Layer {
    ToBack,
    Backward,
    Forward,
    ToFront,
}

/// What clicking a button does.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Action {
    Tool(Tool),
    Stroke(&'static str),
    Background(&'static str),
    /// Opens or closes the full palette for a color.
    Palette(ColorTarget),
    Fill(FillStyle),
    StrokeWidth(f64),
    StrokeStyle(StrokeStyle),
    Roughness(f64),
    Round(bool),
    FontSize(f64),
    FontFamily(i64),
    TextAlign(&'static str),
    Opacity(f64),
    Layer(Layer),
}

/// The properties the style panel edits: the defaults for new elements, or
/// the values of the current selection.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Style {
    pub stroke_color: String,
    pub background_color: String,
    pub fill_style: FillStyle,
    pub stroke_width: f64,
    pub stroke_style: StrokeStyle,
    pub roughness: f64,
    pub round: bool,
    pub opacity: f64,
    pub font_size: f64,
    pub font_family: i64,
    pub text_align: String,
}

impl Default for Style {
    fn default() -> Self {
        Style {
            stroke_color: STROKE_COLORS[0].to_owned(),
            background_color: BACKGROUND_COLORS[0].to_owned(),
            fill_style: FillStyle::Solid,
            stroke_width: 2.0,
            stroke_style: StrokeStyle::Solid,
            roughness: 1.0,
            round: true,
            opacity: 100.0,
            font_size: text::DEFAULT_FONT_SIZE,
            font_family: text::DEFAULT_FONT_FAMILY,
            text_align: "left".to_owned(),
        }
    }
}

/// Which groups of options make sense for the selection or active tool.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Sections {
    pub background: bool,
    pub fill: bool,
    pub stroke_width: bool,
    pub stroke_style: bool,
    pub roughness: bool,
    pub edges: bool,
    pub text: bool,
    pub layers: bool,
}

pub(crate) struct Button {
    pub rect: Rect,
    pub action: Action,
    pub active: bool,
    /// Color shown by the palette button (the current one).
    pub color: Option<String>,
}

pub(crate) enum Hit {
    Button(Action),
    /// The opacity slider, with the value under the pointer.
    Slider(f64),
    /// Dead space on a panel: swallows the click.
    Panel,
}

#[derive(Default)]
pub(crate) struct Chrome {
    panels: Vec<Rect>,
    labels: Vec<(Point, &'static str)>,
    pub buttons: Vec<Button>,
    /// Opacity track and its current value.
    slider: Option<(Rect, f64)>,
}

/// Lays out the toolbar and, when `panel` is given, the style panel showing
/// `style`, with the palette popup for `palette` open beside it.
pub(crate) fn layout(
    size: Size,
    tool: Tool,
    panel: Option<(&Style, Sections)>,
    palette: Option<ColorTarget>,
) -> Chrome {
    let mut chrome = Chrome::default();

    let n = Tool::ALL.len() as f64;
    let width = n * TOOL_BUTTON + (n - 1.0) * TOOL_GAP + 2.0 * TOOLBAR_PADDING;
    let x = ((size.width - width) / 2.0).round();
    chrome.panels.push(Rect::new(x, TOOLBAR_TOP, x + width, TOOLBAR_TOP + TOOL_BUTTON + 2.0 * TOOLBAR_PADDING));
    for (i, t) in Tool::ALL.into_iter().enumerate() {
        let origin = Point::new(
            x + TOOLBAR_PADDING + i as f64 * (TOOL_BUTTON + TOOL_GAP),
            TOOLBAR_TOP + TOOLBAR_PADDING,
        );
        let rect = Rect::from_origin_size(origin, (TOOL_BUTTON, TOOL_BUTTON));
        chrome.buttons.push(Button { rect, action: Action::Tool(t), active: t == tool, color: None });
    }

    let Some((style, sections)) = panel else {
        return chrome;
    };
    let left = PANEL_LEFT + PANEL_PADDING;
    let inner = 6.0 * BUTTON + 5.0 * BUTTON_GAP;
    let mut y = PANEL_TOP + PANEL_PADDING;
    let label = |chrome: &mut Chrome, y: &mut f64, text: &'static str| {
        chrome.labels.push((Point::new(left, *y + LABEL_SIZE), text));
        *y += LABEL_HEIGHT;
    };
    let row = |chrome: &mut Chrome, y: &mut f64, text: &'static str, options: &[(Action, bool)]| {
        label(chrome, y, text);
        for (i, &(action, active)) in options.iter().enumerate() {
            let origin = Point::new(left + i as f64 * (BUTTON + BUTTON_GAP), *y);
            let rect = Rect::from_origin_size(origin, (BUTTON, BUTTON));
            chrome.buttons.push(Button { rect, action, active, color: None });
        }
        *y += BUTTON + SECTION_GAP;
    };
    // Five quick picks, then a button showing the current color that opens
    // the full palette (lit when the color is not one of the picks).
    let colors = |chrome: &mut Chrome, y: &mut f64, text: &'static str, target: ColorTarget| {
        let (picks, current) = match target {
            ColorTarget::Stroke => (STROKE_COLORS, &style.stroke_color),
            ColorTarget::Background => (BACKGROUND_COLORS, &style.background_color),
        };
        let action = |c| match target {
            ColorTarget::Stroke => Action::Stroke(c),
            ColorTarget::Background => Action::Background(c),
        };
        let is_pick = |c: &str| current.eq_ignore_ascii_case(c);
        row(chrome, y, text, &picks.map(|c| (action(c), is_pick(c))));
        let origin = Point::new(left + 5.0 * (BUTTON + BUTTON_GAP), *y - BUTTON - SECTION_GAP);
        let rect = Rect::from_origin_size(origin, (BUTTON, BUTTON));
        let active = palette == Some(target) || !picks.into_iter().any(is_pick);
        chrome.buttons.push(Button { rect, action: Action::Palette(target), active, color: Some(current.clone()) });

        if palette == Some(target) {
            let step = PICKER_SWATCH + PICKER_GAP;
            let corner = Point::new(left + inner + 2.0 * PANEL_PADDING, origin.y);
            let rows = std::iter::once(&PALETTE_BASICS[..]).chain(PALETTE_HUES.iter().map(|hue| &hue[..]));
            let mut bottom = corner.y;
            for (r, shades) in rows.enumerate() {
                for (c, &color) in shades.iter().enumerate() {
                    let at = corner + (PANEL_PADDING + c as f64 * step, PANEL_PADDING + r as f64 * step);
                    let rect = Rect::from_origin_size(at, (PICKER_SWATCH, PICKER_SWATCH));
                    chrome.buttons.push(Button { rect, action: action(color), active: is_pick(color), color: None });
                    bottom = rect.y1;
                }
            }
            let right = corner.x + 2.0 * PANEL_PADDING + 5.0 * step - PICKER_GAP;
            chrome.panels.push(Rect::new(corner.x, corner.y, right, bottom + PANEL_PADDING));
        }
    };

    colors(&mut chrome, &mut y, "Trazo", ColorTarget::Stroke);
    if sections.background {
        colors(&mut chrome, &mut y, "Fondo", ColorTarget::Background);
    }
    if sections.fill {
        let options = [FillStyle::Hachure, FillStyle::CrossHatch, FillStyle::Solid];
        row(&mut chrome, &mut y, "Relleno", &options.map(|f| (Action::Fill(f), style.fill_style == f)));
    }
    if sections.stroke_width {
        let options = [1.0, 2.0, 4.0].map(|w| (Action::StrokeWidth(w), style.stroke_width == w));
        row(&mut chrome, &mut y, "Grosor del trazo", &options);
    }
    if sections.stroke_style {
        let options = [StrokeStyle::Solid, StrokeStyle::Dashed, StrokeStyle::Dotted];
        row(&mut chrome, &mut y, "Estilo del trazo", &options.map(|s| (Action::StrokeStyle(s), style.stroke_style == s)));
    }
    if sections.roughness {
        let options = [0.0, 1.0, 2.0].map(|r| (Action::Roughness(r), style.roughness == r));
        row(&mut chrome, &mut y, "Trazado", &options);
    }
    if sections.edges {
        let options = [false, true].map(|r| (Action::Round(r), style.round == r));
        row(&mut chrome, &mut y, "Bordes", &options);
    }
    if sections.text {
        let options = [5, 6, 8].map(|f| (Action::FontFamily(f), style.font_family == f));
        row(&mut chrome, &mut y, "Fuente", &options);
        let options = [16.0, 20.0, 28.0, 36.0].map(|s| (Action::FontSize(s), style.font_size == s));
        row(&mut chrome, &mut y, "Tamaño de fuente", &options);
        let options = ["left", "center", "right"].map(|a| (Action::TextAlign(a), style.text_align == a));
        row(&mut chrome, &mut y, "Alineación", &options);
    }
    label(&mut chrome, &mut y, "Opacidad");
    // Inset so the knob stays inside the panel at both ends.
    let track = Rect::new(left + KNOB_RADIUS, y, left + inner - KNOB_RADIUS, y + SLIDER_HEIGHT);
    chrome.slider = Some((track, style.opacity));
    y += SLIDER_HEIGHT + SECTION_GAP;
    if sections.layers {
        let options = [Layer::ToBack, Layer::Backward, Layer::Forward, Layer::ToFront];
        row(&mut chrome, &mut y, "Capas", &options.map(|l| (Action::Layer(l), false)));
    }

    let bottom = y - SECTION_GAP + PANEL_PADDING;
    chrome.panels.push(Rect::new(PANEL_LEFT, PANEL_TOP, left + inner + PANEL_PADDING, bottom));
    chrome
}

impl Chrome {
    pub fn hit(&self, pos: Point) -> Option<Hit> {
        if let Some(button) = self.buttons.iter().find(|b| b.rect.contains(pos)) {
            return Some(Hit::Button(button.action));
        }
        if let Some(value) = self.slider_value(pos).filter(|_| self.on_slider(pos)) {
            return Some(Hit::Slider(value));
        }
        self.panels.iter().any(|p| p.contains(pos)).then_some(Hit::Panel)
    }

    fn on_slider(&self, pos: Point) -> bool {
        self.slider.is_some_and(|(track, _)| track.inflate(KNOB_RADIUS, 0.0).contains(pos))
    }

    /// Opacity for a pointer at `pos`, which may be dragged past the track.
    pub fn slider_value(&self, pos: Point) -> Option<f64> {
        let (track, _) = self.slider?;
        let t = ((pos.x - track.x0) / track.width()).clamp(0.0, 1.0);
        Some((t * 10.0).round() * 10.0)
    }

    pub fn draw(&self, ops: &mut Vec<DrawOp>) {
        for panel in &self.panels {
            let shape = RoundedRect::from_rect(*panel, 10.0).to_path(0.1);
            ops.push(DrawOp::fill(shape.clone(), Color::WHITE));
            ops.push(DrawOp::stroke(shape, BORDER, 1.0));
        }
        let mut labels = BezPath::new();
        for (baseline, label) in &self.labels {
            text::append_line(label, UI_FONT, LABEL_SIZE, *baseline, &mut labels);
        }
        ops.push(DrawOp::fill(labels, LABEL));

        for button in &self.buttons {
            let is_tool = matches!(button.action, Action::Tool(_));
            if button.active || !is_tool {
                let bg = RoundedRect::from_rect(button.rect, 8.0).to_path(0.1);
                ops.push(DrawOp::fill(bg, if button.active { ACTIVE_BG } else { BUTTON_BG }));
            }
            let origin = button.rect.center() - (ICON / 2.0, ICON / 2.0);
            let place = Affine::translate(origin.to_vec2());
            let ink = if button.active { ACTIVE_INK } else { INK };
            let glyph = match &button.color {
                Some(color) => swatch(color),
                None => icon(button.action, ink),
            };
            for mut op in glyph {
                op.path = place * op.path;
                ops.push(op);
            }
        }

        if let Some((track, value)) = self.slider {
            let mid = track.center().y;
            let knob_x = track.x0 + track.width() * value / 100.0;
            let bar = |x1: f64| RoundedRect::new(track.x0, mid - 3.0, x1, mid + 3.0, 3.0).to_path(0.1);
            ops.push(DrawOp::fill(bar(track.x1), BUTTON_BG));
            ops.push(DrawOp::fill(bar(knob_x), ACCENT));
            ops.push(DrawOp::fill(Circle::new((knob_x, mid), KNOB_RADIUS).to_path(0.1), ACCENT));
        }
    }
}

fn poly(pts: &[(f64, f64)], close: bool) -> BezPath {
    let mut path = BezPath::new();
    path.move_to(pts[0]);
    for &p in &pts[1..] {
        path.line_to(p);
    }
    if close {
        path.close_path();
    }
    path
}

fn join(parts: &[BezPath]) -> BezPath {
    BezPath::from_vec(parts.iter().flat_map(|p| p.elements().to_vec()).collect())
}

fn line(width: f64, dash: &[f64], path: BezPath, color: Color) -> DrawOp {
    DrawOp { path, color, style: Paint::Stroke { width, dash: dash.to_vec() } }
}

/// A glyph centered in the icon box.
fn letter(label: &str, family: i64, size: f64, color: Color) -> DrawOp {
    let x = (ICON - text::line_width(label, family, size)) / 2.0;
    let mut path = BezPath::new();
    text::append_line(label, family, size, Point::new(x, ICON / 2.0 + size * 0.36), &mut path);
    DrawOp::fill(path, color)
}

fn swatch(color: &str) -> Vec<DrawOp> {
    let shape = RoundedRect::new(1.0, 1.0, 19.0, 19.0, 4.0).to_path(0.1);
    let color = Color::parse(color).unwrap_or(Color::TRANSPARENT);
    if color.is_transparent() {
        // Checkerboard, the usual stand-in for "no color".
        let squares = [(1.0, 1.0), (10.0, 10.0)].map(|(x, y)| Rect::new(x, y, x + 9.0, y + 9.0).to_path(0.1));
        vec![
            DrawOp::fill(shape.clone(), Color::WHITE),
            DrawOp::fill(join(&squares), BORDER),
            DrawOp::stroke(shape, BORDER, 1.0),
        ]
    } else {
        vec![DrawOp::fill(shape.clone(), color), DrawOp::stroke(shape, Color { a: 40, ..INK }, 1.0)]
    }
}

/// The glyph for a button, drawn in a 20×20 box.
fn icon(action: Action, ink: Color) -> Vec<DrawOp> {
    let stroke = |path: BezPath| vec![DrawOp::stroke(path, ink, 1.5)];
    let frame = || RoundedRect::new(2.5, 2.5, 17.5, 17.5, 3.0).to_path(0.1);
    let hatch = |flip: bool| {
        let lines = [(2.5, 9.0, 9.0, 2.5), (2.5, 15.5, 15.5, 2.5), (6.0, 17.5, 17.5, 6.0), (12.5, 17.5, 17.5, 12.5)];
        join(&lines.map(|(x0, y0, x1, y1)| {
            if flip { poly(&[(ICON - x0, y0), (ICON - x1, y1)], false) } else { poly(&[(x0, y0), (x1, y1)], false) }
        }))
    };
    let arrow = |up: bool, bar: bool| {
        let flip = |y: f64| if up { y } else { ICON - y };
        let top = if bar { 6.5 } else { 4.0 };
        let mut parts = vec![
            poly(&[(10.0, flip(top)), (10.0, flip(17.0))], false),
            poly(&[(5.5, flip(top + 4.5)), (10.0, flip(top)), (14.5, flip(top + 4.5))], false),
        ];
        if bar {
            parts.push(poly(&[(4.0, flip(3.0)), (16.0, flip(3.0))], false));
        }
        join(&parts)
    };
    match action {
        Action::Tool(tool) => stroke(tool_icon(tool)),
        Action::Stroke(color) | Action::Background(color) => swatch(color),
        // Drawn from the button's own color.
        Action::Palette(_) => Vec::new(),
        Action::Fill(FillStyle::Solid) => {
            vec![DrawOp::fill(frame(), ink)]
        }
        Action::Fill(style) => {
            let mut ops = vec![DrawOp::stroke(hatch(false), ink, 1.0)];
            if style == FillStyle::CrossHatch {
                ops.push(DrawOp::stroke(hatch(true), ink, 1.0));
            }
            ops.push(DrawOp::stroke(frame(), ink, 1.5));
            ops
        }
        Action::StrokeWidth(width) => {
            vec![DrawOp::stroke(poly(&[(3.5, 10.0), (16.5, 10.0)], false), ink, width.mul_add(0.9, 0.4))]
        }
        Action::StrokeStyle(style) => {
            let dash: &[f64] = match style {
                StrokeStyle::Solid => &[],
                StrokeStyle::Dashed => &[3.5, 3.5],
                StrokeStyle::Dotted => &[0.1, 3.2],
            };
            vec![line(1.8, dash, poly(&[(3.0, 10.0), (17.0, 10.0)], false), ink)]
        }
        Action::Roughness(roughness) => {
            let options = Options { roughness: roughness * 0.9, disable_multi_stroke: roughness == 0.0, ..Options::default() };
            let mut path = BezPath::new();
            let wave = [(2.5, 13.0), (7.5, 6.0), (12.0, 13.5), (17.5, 6.5)].map(Point::from);
            Rough::new(options, 11).curve(&wave, &mut path);
            vec![DrawOp::stroke(path, ink, 1.3)]
        }
        Action::Round(false) => stroke(poly(&[(4.0, 16.0), (4.0, 4.0), (16.0, 4.0)], false)),
        Action::Round(true) => {
            let mut path = BezPath::new();
            path.move_to((4.0, 16.0));
            path.line_to((4.0, 11.0));
            path.curve_to((4.0, 7.0), (7.0, 4.0), (11.0, 4.0));
            path.line_to((16.0, 4.0));
            stroke(path)
        }
        Action::FontSize(size) => {
            let label = match size as u32 {
                0..=16 => "S",
                17..=20 => "M",
                21..=28 => "L",
                _ => "XL",
            };
            vec![letter(label, UI_FONT, 13.0, ink)]
        }
        Action::FontFamily(family) => vec![letter("Aa", family, 13.0, ink)],
        Action::TextAlign(align) => {
            let rows = [(4.5, 15.0), (8.5, 9.0), (12.5, 13.0), (16.5, 7.0)].map(|(y, len): (f64, f64)| {
                let x = match align {
                    "center" => (ICON - len) / 2.0,
                    "right" => 17.5 - len,
                    _ => 2.5,
                };
                poly(&[(x, y), (x + len, y)], false)
            });
            stroke(join(&rows))
        }
        Action::Opacity(_) => Vec::new(),
        Action::Layer(Layer::ToBack) => stroke(arrow(false, true)),
        Action::Layer(Layer::Backward) => stroke(arrow(false, false)),
        Action::Layer(Layer::Forward) => stroke(arrow(true, false)),
        Action::Layer(Layer::ToFront) => stroke(arrow(true, true)),
    }
}

fn tool_icon(tool: Tool) -> BezPath {
    match tool {
        Tool::Hand => join(&[
            poly(&[(10.0, 2.5), (10.0, 17.5)], false),
            poly(&[(2.5, 10.0), (17.5, 10.0)], false),
            poly(&[(7.5, 5.0), (10.0, 2.5), (12.5, 5.0)], false),
            poly(&[(7.5, 15.0), (10.0, 17.5), (12.5, 15.0)], false),
            poly(&[(5.0, 7.5), (2.5, 10.0), (5.0, 12.5)], false),
            poly(&[(15.0, 7.5), (17.5, 10.0), (15.0, 12.5)], false),
        ]),
        Tool::Select => poly(
            &[(5.0, 2.5), (5.0, 16.0), (8.5, 12.8), (11.0, 18.0), (13.2, 17.0), (10.8, 11.8), (15.5, 11.5)],
            true,
        ),
        Tool::Rectangle => RoundedRect::new(3.0, 4.0, 17.0, 16.0, 2.0).to_path(0.1),
        Tool::Diamond => poly(&[(10.0, 2.0), (18.0, 10.0), (10.0, 18.0), (2.0, 10.0)], true),
        Tool::Ellipse => Circle::new((10.0, 10.0), 7.5).to_path(0.1),
        Tool::Arrow => join(&[
            poly(&[(3.0, 10.0), (17.0, 10.0)], false),
            poly(&[(12.0, 5.0), (17.0, 10.0), (12.0, 15.0)], false),
        ]),
        Tool::Line => poly(&[(3.0, 10.0), (17.0, 10.0)], false),
        Tool::Freedraw => join(&[
            poly(&[(3.5, 16.5), (4.5, 12.5), (13.5, 3.5), (16.5, 6.5), (7.5, 15.5)], true),
            poly(&[(11.5, 5.5), (14.5, 8.5)], false),
        ]),
        Tool::Image => join(&[
            RoundedRect::new(2.5, 3.5, 17.5, 16.5, 2.0).to_path(0.1),
            poly(&[(2.5, 14.0), (7.0, 9.5), (10.5, 13.0), (13.0, 10.5), (17.5, 15.0)], false),
            Circle::new((13.0, 7.2), 1.3).to_path(0.1),
        ]),
        Tool::Text => join(&[
            poly(&[(4.0, 6.5), (4.0, 4.0), (16.0, 4.0), (16.0, 6.5)], false),
            poly(&[(10.0, 4.0), (10.0, 16.5)], false),
            poly(&[(7.5, 16.5), (12.5, 16.5)], false),
        ]),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn all() -> Sections {
        Sections {
            background: true,
            fill: true,
            stroke_width: true,
            stroke_style: true,
            roughness: true,
            edges: true,
            text: true,
            layers: true,
        }
    }

    #[test]
    fn toolbar_alone_has_one_button_per_tool() {
        let chrome = layout(Size::new(1000.0, 700.0), Tool::Arrow, None, None);
        assert_eq!(chrome.buttons.len(), Tool::ALL.len());
        assert_eq!(chrome.buttons.iter().filter(|b| b.active).count(), 1);
        let bar = chrome.panels[0];
        assert!((bar.center().x - 500.0).abs() <= 0.5 && chrome.buttons.iter().all(|b| bar.contains_rect(b.rect)));
    }

    #[test]
    fn panel_contains_its_widgets_and_marks_current_values() {
        let style = Style { background_color: "#FFC9C9".into(), ..Style::default() };
        let chrome = layout(Size::new(1000.0, 900.0), Tool::Select, Some((&style, all())), None);
        let panel = chrome.panels[1];
        let widgets = chrome.buttons.iter().filter(|b| !matches!(b.action, Action::Tool(_)));
        assert!(widgets.clone().all(|b| panel.contains_rect(b.rect)));
        let active: Vec<Action> = widgets.filter(|b| b.active).map(|b| b.action).collect();
        assert!(active.contains(&Action::Background("#ffc9c9")) && active.contains(&Action::StrokeWidth(2.0)));
        assert!(active.contains(&Action::Round(true)) && active.contains(&Action::FontFamily(5)));

        let fewer = layout(Size::new(1000.0, 900.0), Tool::Select, Some((&style, Sections::default())), None);
        assert!(fewer.panels[1].height() < panel.height());
    }

    #[test]
    fn hits_buttons_slider_and_dead_space() {
        let style = Style::default();
        let chrome = layout(Size::new(1000.0, 900.0), Tool::Select, Some((&style, all())), None);
        let red = chrome.buttons.iter().find(|b| b.action == Action::Stroke("#e03131")).unwrap();
        assert!(matches!(chrome.hit(red.rect.center()), Some(Hit::Button(Action::Stroke("#e03131")))));
        let (track, _) = chrome.slider.unwrap();
        assert!(matches!(chrome.hit(Point::new(track.x0 + track.width() * 0.52, track.center().y)), Some(Hit::Slider(v)) if v == 50.0));
        assert_eq!(chrome.slider_value(Point::new(-500.0, 0.0)), Some(0.0));
        assert!(matches!(chrome.hit(chrome.panels[1].origin() + (3.0, 3.0)), Some(Hit::Panel)));
        assert!(chrome.hit(Point::new(600.0, 500.0)).is_none());
    }

    #[test]
    fn every_button_has_an_icon() {
        let style = Style::default();
        let chrome = layout(Size::new(1000.0, 900.0), Tool::Select, Some((&style, all())), None);
        for button in chrome.buttons.iter().filter(|b| b.color.is_none()) {
            let ops = icon(button.action, INK);
            assert!(!ops.is_empty() && ops.iter().all(|op| !op.path.is_empty()), "{:?}", button.action);
        }
    }

    #[test]
    fn palette_popup_offers_every_color_beside_the_panel() {
        let style = Style { stroke_color: "#9c36b5".into(), ..Style::default() };
        let size = Size::new(1000.0, 900.0);
        let closed = layout(size, Tool::Select, Some((&style, all())), None);
        let opener = closed.buttons.iter().find(|b| b.action == Action::Palette(ColorTarget::Stroke)).unwrap();
        assert!(opener.active && opener.color.as_deref() == Some("#9c36b5"), "a custom color lights the opener");
        assert!(!closed.buttons.iter().any(|b| b.action == Action::Stroke("#9c36b5")));

        let open = layout(size, Tool::Select, Some((&style, all())), Some(ColorTarget::Stroke));
        // Panels: toolbar, popup, style panel.
        let (popup, panel) = (open.panels[1], open.panels[2]);
        let swatches: Vec<&Button> = open.buttons.iter().filter(|b| popup.contains_rect(b.rect)).collect();
        assert_eq!(swatches.len(), 3 + 12 * 5);
        assert!(swatches.iter().all(|b| matches!(b.action, Action::Stroke(_))));
        assert_eq!(swatches.iter().filter(|b| b.active).map(|b| b.action).collect::<Vec<_>>(), [Action::Stroke("#9c36b5")]);
        assert!(popup.x0 > panel.x1 && popup.y1 < size.height);
    }
}
