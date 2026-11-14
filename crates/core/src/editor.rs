//! The interactive editor: tools, selection, text editing, history and
//! frame building.
//!
//! The shell forwards pointer/keyboard input in logical pixels and paints
//! whatever [`Editor::frame`] returns. No platform types appear here.

use std::collections::{HashMap, HashSet};
use std::f64::consts::{FRAC_PI_2, TAU};
use std::rc::Rc;

use kurbo::{Affine, BezPath, Circle, Point, Rect, Shape, Size, Vec2};
use serde_json::{json, Value};

use crate::binding::{self, Binding, End};
use crate::bound::{bound_position, container_size_for, is_container, max_height, max_width};
use crate::draw::{Color, DrawOp, Frame, Item};
use crate::element::{Document, Element, Kind, Roundness};
use crate::export::background;
use crate::hit::hit;
use crate::rough::Random;
use crate::shape::{element_ops, outline};
use crate::text::{self, Line};
use crate::ui::{self, Action, Chrome, ColorTarget, Layer, Sections, Style, ACCENT};

pub use crate::ui::Tool;

/// Hit slack around strokes and handles, in screen pixels.
const HIT_TOLERANCE: f64 = 6.0;
const SELECTION_PADDING: f64 = 4.0;
const HANDLE_SIZE: f64 = 8.0;
const POINT_GRIP_RADIUS: f64 = 4.5;
const MID_GRIP_RADIUS: f64 = 3.0;
/// Segments shorter than this on screen get no midpoint grip.
const MID_GRIP_MIN_SEGMENT: f64 = 40.0;
/// Distance of the rotation grip above the selection box, in screen pixels.
const ROTATE_GRIP_OFFSET: f64 = 20.0;
const ROTATE_SNAP: f64 = 15.0;
/// MIME-like tag Excalidraw puts on elements it copies to the clipboard.
const CLIPBOARD_TYPE: &str = "excalidraw/clipboard";
const MIN_ZOOM: f64 = 0.1;
const MAX_ZOOM: f64 = 30.0;
const FIT_MARGIN: f64 = 60.0;
const DUPLICATE_OFFSET: f64 = 10.0;

/// Excalidraw's keyboard shortcuts.
fn tool_from_key(c: char) -> Option<Tool> {
    Some(match c {
        'h' => Tool::Hand,
        'v' | '1' => Tool::Select,
        'r' | '2' => Tool::Rectangle,
        'd' | '3' => Tool::Diamond,
        'o' | '4' => Tool::Ellipse,
        'a' | '5' => Tool::Arrow,
        'l' | '6' => Tool::Line,
        'p' | '7' => Tool::Freedraw,
        't' | '8' => Tool::Text,
        '9' => Tool::Image,
        _ => return None,
    })
}

fn element_kind(tool: Tool) -> Option<Kind> {
    Some(match tool {
        Tool::Rectangle => Kind::Rectangle,
        Tool::Diamond => Kind::Diamond,
        Tool::Ellipse => Kind::Ellipse,
        Tool::Arrow => Kind::Arrow,
        Tool::Line => Kind::Line,
        Tool::Freedraw => Kind::Freedraw,
        Tool::Text => Kind::Text,
        Tool::Hand | Tool::Select | Tool::Image => return None,
    })
}

fn is_stroked(kind: &Kind) -> bool {
    is_container(kind) || matches!(kind, Kind::Line | Kind::Arrow)
}

fn is_fillable(kind: &Kind) -> bool {
    is_container(kind) || matches!(kind, Kind::Line | Kind::Freedraw)
}

fn has_stroke_width(kind: &Kind) -> bool {
    is_stroked(kind) || *kind == Kind::Freedraw
}

fn has_edges(kind: &Kind) -> bool {
    matches!(kind, Kind::Rectangle | Kind::Diamond | Kind::Line | Kind::Arrow)
}

fn is_text(kind: &Kind) -> bool {
    *kind == Kind::Text
}

/// Lines and arrows are edited through their points, not a bounding box.
fn has_point_grips(kind: &Kind) -> bool {
    matches!(kind, Kind::Line | Kind::Arrow)
}

/// Records on `el` that the element `id` (an arrow or a label) is attached
/// to it, in Excalidraw's `boundElements` list.
fn link(el: &mut Element, kind: &str, id: &str) {
    let entry = json!({ "type": kind, "id": id });
    match el.extra.get_mut("boundElements").and_then(Value::as_array_mut) {
        Some(list) if list.iter().any(|l| l["id"] == id) => {}
        Some(list) => list.push(entry),
        None => {
            el.extra.insert("boundElements".to_owned(), json!([entry]));
        }
    }
}

fn unlink(el: &mut Element, id: &str) {
    if let Some(list) = el.extra.get_mut("boundElements").and_then(Value::as_array_mut) {
        list.retain(|l| l["id"] != id);
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Modifiers {
    pub shift: bool,
    pub ctrl: bool,
    pub alt: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Button {
    Primary,
    Middle,
    Secondary,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Key {
    /// A printable key, lowercased. Typed text goes through
    /// [`Editor::text_input`] instead.
    Char(char),
    Enter,
    Backspace,
    Delete,
    Escape,
    Space,
    Left,
    Right,
    Up,
    Down,
    Home,
    End,
}

/// Something only the shell can do, asked for through
/// [`Editor::take_request`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Request {
    /// Let the user choose an image file, then call
    /// [`Editor::insert_image`] with it.
    PickImage,
}

/// Pointer shape the shell should show.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cursor {
    Default,
    Crosshair,
    Text,
    Grab,
}

/// Which edges of the selection box a resize handle moves: -1 is the
/// left/top edge, 1 the right/bottom edge, 0 leaves that axis alone.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Handle {
    hx: i8,
    hy: i8,
}

/// Something the pointer can grab on the selected element.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Grip {
    /// A resize handle on the selection box.
    Box(Handle),
    /// The point with this index of a line or arrow.
    Point(usize),
    /// The middle of the segment starting at this point index; dragging it
    /// adds a point there.
    Mid(usize),
    /// The rotation knob above the selection box.
    Rotate,
}

const HANDLES: [Handle; 8] = [
    Handle { hx: -1, hy: -1 },
    Handle { hx: 0, hy: -1 },
    Handle { hx: 1, hy: -1 },
    Handle { hx: 1, hy: 0 },
    Handle { hx: 1, hy: 1 },
    Handle { hx: 0, hy: 1 },
    Handle { hx: -1, hy: 1 },
    Handle { hx: -1, hy: 0 },
];

enum Drag {
    Pan { last: Point },
    /// Dragging out a new shape; the element is the last one in the scene.
    Create { origin: Point },
    /// Freehand drawing into the last element.
    Draw,
    Move { last: Point, started: bool },
    /// `grab` is the offset from the pointer to the edge being dragged, so
    /// the edge does not jump to the pointer when the drag starts.
    Resize { handle: Handle, original: Box<Element>, grab: Vec2, started: bool },
    /// Dragging point `index` of the line or arrow `id`. `other` is what
    /// the arrow's opposite end was bound to when the drag began. With
    /// `insert`, the point does not exist yet and is added on the first move.
    Point { id: String, index: usize, other: Option<Binding>, insert: bool, started: bool },
    Marquee { origin: Point, current: Point },
    Rotate { id: String, started: bool },
    /// Dragging the opacity slider.
    Slider,
}

/// An in-progress text edit. The text being typed is the element's
/// `original_text`; `text` holds its wrapped form.
struct TextEdit {
    id: String,
    /// Byte offset of the caret in `original_text`.
    caret: usize,
    /// `modified` before the edit began, restored if nothing changes.
    was_modified: bool,
}

pub struct Editor {
    doc: Document,
    scroll: Vec2,
    zoom: f64,
    size: Size,
    tool: Tool,
    /// Defaults for new elements, as set in the style panel.
    style: Style,
    selected: HashSet<String>,
    editing: Option<TextEdit>,
    /// The color whose palette popup is open.
    palette: Option<ColorTarget>,
    /// The last element is a line or arrow being laid out click by click;
    /// its final point floats under the pointer until the next click.
    placing: bool,
    undo: Vec<Vec<Element>>,
    redo: Vec<Vec<Element>>,
    drag: Option<Drag>,
    /// Shape the arrow end being dragged would attach to; highlighted.
    bind_hover: Option<String>,
    /// Last known pointer position on screen; where pasted content lands.
    pointer: Option<Point>,
    request: Option<Request>,
    space_held: bool,
    modified: bool,
    rng: Random,
    /// Generated ops per element id, valid while (version, nonce) match.
    cache: HashMap<String, ((i64, i64), Rc<Vec<DrawOp>>)>,
}

impl Editor {
    /// `seed` feeds element ids and shape seeds; pass something that varies
    /// between runs (the shell owns the clock, this crate does not).
    pub fn new(seed: u64) -> Self {
        Editor {
            doc: Document::default(),
            scroll: Vec2::ZERO,
            zoom: 1.0,
            size: Size::new(800.0, 600.0),
            tool: Tool::Select,
            style: Style::default(),
            selected: HashSet::new(),
            editing: None,
            palette: None,
            placing: false,
            undo: Vec::new(),
            redo: Vec::new(),
            drag: None,
            bind_hover: None,
            pointer: None,
            request: None,
            space_held: false,
            modified: false,
            rng: Random::new((seed ^ (seed >> 32)) as i64),
            cache: HashMap::new(),
        }
    }

    pub fn load(&mut self, doc: Document) {
        self.doc = doc;
        binding::migrate_legacy(&mut self.doc.elements);
        self.selected.clear();
        self.editing = None;
        self.placing = false;
        self.undo.clear();
        self.redo.clear();
        self.drag = None;
        self.modified = false;
        self.zoom_to_fit();
    }

    /// The document as it should be saved; an in-progress text edit is
    /// included as typed so far.
    pub fn document(&self) -> &Document {
        &self.doc
    }

    pub fn tool(&self) -> Tool {
        self.tool
    }

    pub fn set_tool(&mut self, tool: Tool) {
        self.commit_text();
        self.finish_placing();
        if tool == Tool::Image {
            // The shell shows a file dialog; the current tool stays.
            self.request = Some(Request::PickImage);
            return;
        }
        self.tool = tool;
        if tool != Tool::Select {
            self.selected.clear();
        }
    }

    /// What the editor needs the shell to do, if anything. Call after
    /// feeding input.
    pub fn take_request(&mut self) -> Option<Request> {
        self.request.take()
    }

    /// Adds an image to the scene, centered on the pointer and selected.
    /// `data_url` holds the encoded file (`data:<mime>;base64,...`), `width`
    /// and `height` its size in pixels, `now_ms` the current Unix time.
    pub fn insert_image(&mut self, data_url: &str, mime: &str, width: f64, height: f64, now_ms: f64) {
        if width <= 0.0 || height <= 0.0 {
            return;
        }
        self.commit_text();
        self.finish_placing();
        self.checkpoint();
        let file_id: String = (0..40).map(|_| char::from_digit((self.rng.next() * 16.0) as u32, 16).unwrap_or('0')).collect();
        let file = json!({ "id": file_id, "mimeType": mime, "dataURL": data_url, "created": now_ms });
        match self.doc.files.as_object_mut() {
            Some(files) => {
                files.insert(file_id.clone(), file);
            }
            None => self.doc.files = json!({ file_id.clone(): file }),
        }

        // As in Excalidraw: natural size, but at most half the window high.
        let room = (self.size.height - 120.0).max(160.0).min((self.size.height * 0.5).floor() / self.zoom);
        let shown_height = height.min(room);
        let shown_width = shown_height * width / height;
        let center = Point::new(self.size.width / 2.0, self.size.height / 2.0);
        let at = self.to_scene(self.pointer.unwrap_or(center));
        let mut el = self.new_element(Kind::Image, at - Vec2::new(shown_width / 2.0, shown_height / 2.0));
        (el.width, el.height) = (shown_width, shown_height);
        (el.stroke_color, el.background_color) = ("transparent".to_owned(), "transparent".to_owned());
        el.extra.insert("fileId".to_owned(), Value::String(file_id));
        el.extra.insert("status".to_owned(), Value::String("saved".to_owned()));
        el.extra.insert("scale".to_owned(), json!([1, 1]));
        el.extra.insert("crop".to_owned(), Value::Null);
        self.selected = HashSet::from([el.id.clone()]);
        self.tool = Tool::Select;
        self.doc.elements.push(el);
    }

    /// Whether there are changes since the last load or [`Self::mark_saved`].
    pub fn is_modified(&self) -> bool {
        self.modified
    }

    pub fn mark_saved(&mut self) {
        self.modified = false;
    }

    pub fn resize(&mut self, width: f64, height: f64) {
        self.size = Size::new(width, height);
    }

    /// Whether typed characters currently go into a text element.
    pub fn is_editing_text(&self) -> bool {
        self.editing.is_some()
    }

    pub fn cursor(&self) -> Cursor {
        match self.tool {
            _ if self.space_held || matches!(self.drag, Some(Drag::Pan { .. })) => Cursor::Grab,
            Tool::Hand => Cursor::Grab,
            Tool::Text => Cursor::Text,
            Tool::Select => Cursor::Default,
            _ => Cursor::Crosshair,
        }
    }

    // ----- view ---------------------------------------------------------

    /// Scene → screen, Excalidraw's convention: `(scene + scroll) * zoom`.
    fn view(&self) -> Affine {
        Affine::scale(self.zoom) * Affine::translate(self.scroll)
    }

    fn to_scene(&self, screen: Point) -> Point {
        self.view().inverse() * screen
    }

    /// Multiplies the zoom by `factor`, keeping the scene point under
    /// `anchor` (screen coordinates) fixed.
    pub fn zoom_at(&mut self, anchor: Point, factor: f64) {
        let before = self.to_scene(anchor);
        self.zoom = (self.zoom * factor).clamp(MIN_ZOOM, MAX_ZOOM);
        self.scroll += self.to_scene(anchor) - before;
    }

    /// Frames the whole drawing in the window, never magnifying past 100%.
    pub fn zoom_to_fit(&mut self) {
        let bounds = self.live().map(Element::scene_bounds).reduce(|a, b| a.union(b));
        let Some(bounds) = bounds else {
            self.zoom = 1.0;
            self.scroll = Vec2::ZERO;
            return;
        };
        let fit = |avail: f64, extent: f64| if extent > 0.0 { avail / extent } else { 1.0 };
        let zoom = fit(self.size.width - 2.0 * FIT_MARGIN, bounds.width())
            .min(fit(self.size.height - 2.0 * FIT_MARGIN, bounds.height()));
        self.zoom = zoom.clamp(MIN_ZOOM, 1.0);
        let center = Point::new(self.size.width / 2.0, self.size.height / 2.0);
        self.scroll = center.to_vec2() / self.zoom - bounds.center().to_vec2();
    }

    // ----- scene helpers --------------------------------------------------

    fn live(&self) -> impl Iterator<Item = &Element> {
        self.doc.elements.iter().filter(|e| !e.is_deleted)
    }

    fn index_of(&self, id: &str) -> Option<usize> {
        self.doc.elements.iter().position(|e| !e.is_deleted && e.id == id)
    }

    /// The label bound to the container with this id.
    fn bound_text(&self, container_id: &str) -> Option<usize> {
        self.doc
            .elements
            .iter()
            .position(|e| !e.is_deleted && is_text(&e.kind) && e.container_id.as_deref() == Some(container_id))
    }

    /// The container a label belongs to, if it still exists.
    fn container_of(&self, label: &Element) -> Option<usize> {
        self.index_of(label.container_id.as_deref()?)
    }

    /// Indices of the elements an operation on the selection affects: the
    /// selected elements and the labels bound to them, in z-order.
    fn targets(&self) -> Vec<usize> {
        let selected = |id: &str| self.selected.contains(id);
        self.doc
            .elements
            .iter()
            .enumerate()
            .filter(|(_, e)| !e.is_deleted && (selected(&e.id) || e.container_id.as_deref().is_some_and(selected)))
            .map(|(i, _)| i)
            .collect()
    }

    fn single_selection(&self) -> Option<&Element> {
        if self.selected.len() != 1 {
            return None;
        }
        self.live().find(|e| self.selected.contains(&e.id))
    }

    /// Saves the scene for undo. Call before mutating.
    fn checkpoint(&mut self) {
        self.undo.push(self.doc.elements.clone());
        self.redo.clear();
        self.modified = true;
    }

    fn random_int(&mut self) -> i64 {
        (self.rng.next() * 2_147_483_647.0) as i64 + 1
    }

    fn random_id(&mut self) -> String {
        const ALPHABET: &[u8] = b"0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz-_";
        (0..21).map(|_| ALPHABET[(self.rng.next() * 64.0) as usize] as char).collect()
    }

    /// Records a change so caches and (later) collaborators notice it.
    fn touch(el: &mut Element, nonce: i64) {
        el.version += 1;
        el.version_nonce = nonce;
    }

    fn new_element(&mut self, kind: Kind, at: Point) -> Element {
        let style = self.style.clone();
        let roundness = match kind {
            Kind::Rectangle => Some(Roundness::ADAPTIVE),
            Kind::Diamond | Kind::Line | Kind::Arrow => Some(Roundness::PROPORTIONAL),
            _ => None,
        }
        .filter(|_| style.round)
        .map(|kind| Roundness { kind, value: None });
        let linear = matches!(kind, Kind::Line | Kind::Arrow | Kind::Freedraw);
        Element {
            id: self.random_id(),
            end_arrowhead: (kind == Kind::Arrow).then(|| "arrow".to_owned()),
            points: if linear { vec![[0.0, 0.0]] } else { Vec::new() },
            kind,
            x: at.x,
            y: at.y,
            stroke_color: style.stroke_color,
            background_color: style.background_color,
            fill_style: style.fill_style,
            stroke_width: style.stroke_width,
            stroke_style: style.stroke_style,
            roughness: style.roughness,
            opacity: style.opacity,
            roundness,
            seed: self.random_int(),
            version_nonce: self.random_int(),
            ..Element::default()
        }
    }

    // ----- arrow bindings --------------------------------------------------

    /// The binding an arrow end dropped at `at` would get. The arrow at
    /// `idx` itself is never a target.
    fn candidate_binding(&self, idx: usize, at: Point) -> Option<Binding> {
        let shape = binding::candidate(&self.doc.elements, at, self.zoom).filter(|&s| s != idx)?;
        let shape = &self.doc.elements[shape];
        Some(Binding { element_id: shape.id.clone(), fixed_point: binding::fixed_point_for(shape, at), inside: false })
    }

    /// Attaches (or with `None`, detaches) one end of the arrow at `idx`,
    /// keeping the shapes' back-references in sync.
    fn set_binding(&mut self, idx: usize, end: End, new: Option<Binding>) {
        let arrow = &self.doc.elements[idx];
        let old = binding::read(arrow, end);
        if old == new {
            return;
        }
        let arrow_id = arrow.id.clone();
        let still_used = |id: &str| {
            new.as_ref().is_some_and(|b| b.element_id == id)
                || binding::read(arrow, end.other()).is_some_and(|b| b.element_id == id)
        };
        let dropped = old.map(|b| b.element_id).filter(|id| !still_used(id));
        binding::write(&mut self.doc.elements[idx], end, new.as_ref());
        if let Some(shape) = dropped.and_then(|id| self.index_of(&id)) {
            unlink(&mut self.doc.elements[shape], &arrow_id);
        }
        if let Some(shape) = new.and_then(|b| self.index_of(&b.element_id)) {
            link(&mut self.doc.elements[shape], "arrow", &arrow_id);
        }
    }

    /// Sets both bindings of the arrow at `idx` and snaps its ends. An arrow
    /// with both ends in one shape lives inside it instead of orbiting it.
    fn apply_bindings(&mut self, idx: usize, start: Option<Binding>, finish: Option<Binding>) {
        let same = matches!((&start, &finish), (Some(a), Some(b)) if a.element_id == b.element_id);
        for (end, binding) in [(End::Start, start), (End::End, finish)] {
            self.set_binding(idx, end, binding.map(|b| Binding { inside: b.inside || same, ..b }));
        }
        self.refresh_arrow(idx);
    }

    /// Moves the bound ends of the arrow at `idx` to where their shapes
    /// are now.
    fn refresh_arrow(&mut self, idx: usize) {
        let nonce = self.random_int();
        let arrow = &self.doc.elements[idx];
        if arrow.kind != Kind::Arrow || arrow.points.len() < 2 {
            return;
        }
        let resolve = |end: End| {
            let binding = binding::read(arrow, end)?;
            let shape = self.index_of(&binding.element_id)?;
            Some((binding, &self.doc.elements[shape]))
        };
        let (start, finish) = (resolve(End::Start), resolve(End::End));
        let before = binding::global_points(arrow);
        let mut points = before.clone();
        for (end, this, other) in [(End::Start, &start, &finish), (End::End, &finish, &start)] {
            if let Some((binding, shape)) = this {
                let other = other.as_ref().map(|(b, s)| (b, *s));
                let at = binding::bound_point(arrow, &points, end, binding, shape, other);
                points[end.indices(before.len()).0] = at;
            }
        }
        if points != before {
            let arrow = &mut self.doc.elements[idx];
            binding::set_global_points(arrow, &points);
            Self::touch(arrow, nonce);
        }
    }

    /// Makes arrows follow the shapes in `shapes` after those moved or
    /// changed size.
    fn refresh_bound_arrows(&mut self, shapes: &HashSet<String>) {
        let bound_to = |e: &Element| {
            End::BOTH.into_iter().any(|end| binding::read(e, end).is_some_and(|b| shapes.contains(&b.element_id)))
        };
        let arrows: Vec<usize> = self
            .doc
            .elements
            .iter()
            .enumerate()
            .filter(|(_, e)| !e.is_deleted && e.kind == Kind::Arrow && bound_to(e))
            .map(|(i, _)| i)
            .collect();
        for i in arrows {
            self.refresh_arrow(i);
        }
    }

    // ----- multi-point lines -----------------------------------------------

    /// Moves the floating last point of the line being placed to `p`.
    fn placing_move(&mut self, p: Point, nonce: i64) {
        let idx = self.doc.elements.len() - 1;
        let el = &mut self.doc.elements[idx];
        let mut points = binding::global_points(el);
        if let Some(last) = points.last_mut() {
            *last = p;
        }
        binding::set_global_points(el, &points);
        Self::touch(el, nonce);
        if el.kind == Kind::Arrow {
            let (start, finish) = (binding::read(el, End::Start), self.candidate_binding(idx, p));
            self.bind_hover = finish.as_ref().map(|b| b.element_id.clone());
            self.apply_bindings(idx, start, finish);
        }
    }

    /// A click while placing: fixes the floating point at `p` and starts a
    /// new one, or ends the line if the click is on its last fixed point.
    fn placing_click(&mut self, p: Point, screen: Point) {
        let nonce = self.random_int();
        let view = self.view();
        let Some(el) = self.doc.elements.last_mut() else {
            return;
        };
        let mut points = binding::global_points(el);
        let fixed = points[points.len().saturating_sub(2)];
        if (view * fixed).distance(screen) <= HIT_TOLERANCE {
            return self.finish_placing();
        }
        if let Some(last) = points.last_mut() {
            *last = p;
        }
        points.push(p);
        binding::set_global_points(el, &points);
        Self::touch(el, nonce);
    }

    /// Ends click-by-click placement, dropping the floating point. A line
    /// left with a single point is discarded.
    fn finish_placing(&mut self) {
        if !std::mem::take(&mut self.placing) {
            return;
        }
        self.bind_hover = None;
        let nonce = self.random_int();
        let idx = self.doc.elements.len() - 1;
        let el = &mut self.doc.elements[idx];
        let mut points = binding::global_points(el);
        points.pop();
        let Some(&tip) = points.last().filter(|_| points.len() >= 2) else {
            if let Some(before) = self.undo.pop() {
                self.doc.elements = before;
            }
            return;
        };
        binding::set_global_points(el, &points);
        Self::touch(el, nonce);
        self.selected = HashSet::from([el.id.clone()]);
        self.tool = Tool::Select;
        if el.kind == Kind::Arrow {
            let (start, finish) = (binding::read(el, End::Start), self.candidate_binding(idx, tip));
            self.apply_bindings(idx, start, finish);
        }
    }

    // ----- groups ---------------------------------------------------------

    /// Grows the selection to whole groups: picking one member of a group
    /// picks them all.
    fn select_groups(&mut self) {
        let groups: HashSet<&String> =
            self.live().filter(|e| self.selected.contains(&e.id)).filter_map(|e| e.group_ids.last()).collect();
        if groups.is_empty() {
            return;
        }
        let members: Vec<String> = self
            .live()
            .filter(|e| e.container_id.is_none() && e.group_ids.iter().any(|g| groups.contains(g)))
            .map(|e| e.id.clone())
            .collect();
        self.selected.extend(members);
    }

    fn group_selected(&mut self) {
        if self.selected.len() < 2 {
            return;
        }
        let targets = self.targets();
        self.checkpoint();
        let (group, nonce) = (self.random_id(), self.random_int());
        for i in targets {
            let el = &mut self.doc.elements[i];
            el.group_ids.push(group.clone());
            Self::touch(el, nonce);
        }
    }

    fn ungroup_selected(&mut self) {
        let targets = self.targets();
        if targets.iter().all(|&i| self.doc.elements[i].group_ids.is_empty()) {
            return;
        }
        self.checkpoint();
        let nonce = self.random_int();
        for i in targets {
            let el = &mut self.doc.elements[i];
            if el.group_ids.pop().is_some() {
                Self::touch(el, nonce);
            }
        }
    }

    // ----- text -----------------------------------------------------------

    /// (family, size, unitless line height) of a text element.
    fn font(el: &Element) -> (i64, f64, f64) {
        let family = el.font_family.unwrap_or(text::DEFAULT_FONT_FAMILY);
        let size = el.font_size.unwrap_or(text::DEFAULT_FONT_SIZE);
        (family, size, el.line_height.unwrap_or(text::metrics(family).line_height))
    }

    /// Width the text at `idx` wraps at: its container's, its own if it has
    /// a fixed width, or none.
    fn wrap_width(&self, idx: usize) -> Option<f64> {
        let el = &self.doc.elements[idx];
        match self.container_of(el) {
            Some(c) => Some(max_width(&self.doc.elements[c])),
            None => {
                let auto_resize = el.extra.get("autoResize").and_then(Value::as_bool).unwrap_or(true);
                (!auto_resize).then_some(el.width)
            }
        }
    }

    /// Re-wraps and re-measures the text at `idx` after its content, font or
    /// container changed, growing the container if the text no longer fits.
    /// Follows Excalidraw's `redrawTextBoundingBox`.
    fn relayout_text(&mut self, idx: usize) {
        let nonce = self.random_int();
        let wrap_width = self.wrap_width(idx);
        let container = self.container_of(&self.doc.elements[idx]);
        let el = &self.doc.elements[idx];
        let (family, size, line_height) = Self::font(el);
        let source = el.original_text.clone().or_else(|| el.text.clone()).unwrap_or_default();
        let wrapped = match wrap_width {
            Some(max) => text::wrap_text(&source, family, size, max),
            None => source,
        };
        let (width, height) = text::measure(&wrapped, family, size, line_height);

        if let Some(c) = container {
            let holder = &mut self.doc.elements[c];
            let size = (holder.width, holder.height);
            if height > max_height(holder) {
                holder.height = container_size_for(height, &holder.kind);
            }
            if width > max_width(holder) {
                holder.width = container_size_for(width, &holder.kind);
            }
            let grew = size != (holder.width, holder.height);
            if grew {
                Self::touch(holder, nonce);
            }
            let holder = holder.clone();
            let el = &mut self.doc.elements[idx];
            (el.width, el.height, el.angle) = (width, height, holder.angle);
            (el.x, el.y) = bound_position(&holder, el);
            if grew {
                self.refresh_bound_arrows(&HashSet::from([holder.id]));
            }
        } else {
            let el = &mut self.doc.elements[idx];
            if wrap_width.is_none() {
                // Keep the aligned edge (or the center) where it was.
                let slack = el.width - width;
                el.x += match el.text_align.as_deref() {
                    Some("center") => slack / 2.0,
                    Some("right") => slack,
                    _ => 0.0,
                };
                el.width = width;
            }
            el.height = height;
        }
        let el = &mut self.doc.elements[idx];
        el.text = Some(wrapped);
        Self::touch(el, nonce);
    }

    /// Starts typing a new text: free-standing at `at`, or as the label of
    /// the container at index `container`.
    fn create_text(&mut self, at: Point, container: Option<usize>) {
        let was_modified = self.modified;
        self.checkpoint();
        let mut el = self.new_element(Kind::Text, at);
        let family = self.style.font_family;
        let line_height = text::metrics(family).line_height;
        el.text = Some(String::new());
        el.original_text = Some(String::new());
        el.font_size = Some(self.style.font_size);
        el.font_family = Some(family);
        el.line_height = Some(line_height);
        el.extra.insert("autoResize".to_owned(), Value::Bool(true));
        let id = el.id.clone();
        let idx = match container {
            Some(c) => {
                el.text_align = Some("center".to_owned());
                el.vertical_align = Some("middle".to_owned());
                let holder = &mut self.doc.elements[c];
                el.container_id = Some(holder.id.clone());
                link(holder, "text", &id);
                // A label sits right above its container in z-order.
                self.doc.elements.insert(c + 1, el);
                c + 1
            }
            None => {
                el.text_align = Some(self.style.text_align.clone());
                el.vertical_align = Some("top".to_owned());
                // The click marks the middle of the first line.
                el.y -= self.style.font_size * line_height / 2.0;
                self.doc.elements.push(el);
                self.doc.elements.len() - 1
            }
        };
        self.relayout_text(idx);
        self.selected.clear();
        self.editing = Some(TextEdit { id, caret: 0, was_modified });
    }

    /// Starts editing the existing text at `idx`, caret at the end.
    fn start_edit(&mut self, idx: usize) {
        let was_modified = self.modified;
        self.checkpoint();
        let el = &mut self.doc.elements[idx];
        let source = el.original_text.get_or_insert_with(|| el.text.clone().unwrap_or_default());
        let edit = TextEdit { id: el.id.clone(), caret: source.len(), was_modified };
        self.selected.clear();
        self.drag = None;
        self.editing = Some(edit);
    }

    /// Ends the current text edit. Empty text is discarded, and an edit
    /// that changed nothing leaves no undo step.
    pub fn commit_text(&mut self) {
        let Some(edit) = self.editing.take() else {
            return;
        };
        self.tool = Tool::Select;
        let Some(idx) = self.index_of(&edit.id) else {
            return;
        };
        let el = &self.doc.elements[idx];
        let empty = el.original_text.as_deref().unwrap_or("").trim().is_empty();
        let container = el.container_id.clone();
        let existed = self.undo.last().is_some_and(|before| before.iter().any(|e| e.id == edit.id));
        if empty && !existed {
            // A new text nobody typed into: as if it never happened.
            if let Some(before) = self.undo.pop() {
                self.doc.elements = before;
            }
            self.modified = edit.was_modified;
            return;
        }
        if empty {
            let nonce = self.random_int();
            let el = &mut self.doc.elements[idx];
            el.is_deleted = true;
            Self::touch(el, nonce);
            if let Some(c) = container.as_deref().and_then(|id| self.index_of(id)) {
                unlink(&mut self.doc.elements[c], &edit.id);
            }
        } else {
            self.selected = HashSet::from([container.unwrap_or(edit.id)]);
        }
        if self.undo.last() == Some(&self.doc.elements) {
            self.undo.pop();
            self.modified = edit.was_modified;
        }
    }

    /// Index and caret of the text being edited.
    fn edit_target(&self) -> Option<(usize, usize)> {
        let edit = self.editing.as_ref()?;
        Some((self.index_of(&edit.id)?, edit.caret))
    }

    /// The visual lines of the text at `idx`, with their source.
    fn edit_lines(&self, idx: usize) -> (Vec<Line>, &str) {
        let el = &self.doc.elements[idx];
        let (family, size, _) = Self::font(el);
        let source = el.original_text.as_deref().unwrap_or("");
        (text::wrap(source, family, size, self.wrap_width(idx)), source)
    }

    /// Index of the visual line holding byte offset `caret`.
    fn caret_line(lines: &[Line], caret: usize) -> usize {
        lines.iter().rposition(|l| l.start <= caret).unwrap_or(0)
    }

    /// Horizontal position of byte offset `caret` on `line`, element-local.
    fn caret_x(&self, idx: usize, line: &Line, source: &str, caret: usize) -> f64 {
        let el = &self.doc.elements[idx];
        let (family, size, _) = Self::font(el);
        let width = |s: &str| text::line_width(s, family, size);
        let upto = caret.clamp(line.start, line.end);
        text::line_x(el, width(&source[line.start..line.end])) + width(&source[line.start..upto])
    }

    /// Byte offset on `line` closest to the element-local abscissa `x`.
    fn offset_at_x(&self, idx: usize, line: &Line, source: &str, x: f64) -> usize {
        let stops = source[line.start..line.end].char_indices().map(|(i, _)| line.start + i).chain([line.end]);
        stops
            .map(|at| (at, (self.caret_x(idx, line, source, at) - x).abs()))
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map_or(line.start, |(at, _)| at)
    }

    /// Inserts typed or composed text at the caret. Ignored unless a text
    /// is being edited.
    pub fn text_input(&mut self, input: &str) {
        // Tabs become spaces, as in Excalidraw, so they measure and render.
        let clean: String = input
            .replace("\r\n", "\n")
            .replace('\t', "        ")
            .chars()
            .filter(|c| !c.is_control() || *c == '\n')
            .collect();
        if !clean.is_empty() {
            self.edit_source(|source, caret| {
                source.insert_str(*caret, &clean);
                *caret += clean.len();
            });
        }
    }

    /// Applies `change` to the edited text and its caret, then re-wraps.
    fn edit_source(&mut self, change: impl FnOnce(&mut String, &mut usize)) {
        let Some((idx, _)) = self.edit_target() else {
            return;
        };
        let source = self.doc.elements[idx].original_text.get_or_insert_with(String::new);
        if let Some(edit) = &mut self.editing {
            edit.caret = edit.caret.min(source.len());
            change(source, &mut edit.caret);
        }
        self.relayout_text(idx);
    }

    fn edit_key(&mut self, key: Key, mods: Modifiers) {
        let Some((idx, caret)) = self.edit_target() else {
            self.editing = None;
            return;
        };
        let (lines, source) = self.edit_lines(idx);
        let row = Self::caret_line(&lines, caret);
        let prev = source[..caret].char_indices().next_back().map_or(caret, |(i, _)| i);
        let next = source[caret..].chars().next().map_or(caret, |c| caret + c.len_utf8());
        let vertical = |to: Option<&Line>| {
            to.map(|line| self.offset_at_x(idx, line, source, self.caret_x(idx, &lines[row], source, caret)))
        };
        let moved = match key {
            Key::Escape => return self.commit_text(),
            Key::Enter if mods.ctrl => return self.commit_text(),
            Key::Enter => return self.text_input("\n"),
            Key::Backspace => {
                return self.edit_source(|source, at| {
                    source.replace_range(prev..*at, "");
                    *at = prev;
                })
            }
            Key::Delete => return self.edit_source(|source, at| source.replace_range(*at..next, "")),
            Key::Left => Some(prev),
            Key::Right => Some(next),
            Key::Home => Some(lines[row].start),
            Key::End => Some(lines[row].end),
            Key::Up => vertical(row.checked_sub(1).map(|r| &lines[r])),
            Key::Down => vertical(lines.get(row + 1)),
            Key::Char(_) | Key::Space => None,
        };
        if let (Some(at), Some(edit)) = (moved, &mut self.editing) {
            edit.caret = at;
        }
    }

    /// Places the caret at the scene point `p` if it lies on the text being
    /// edited; returns whether it did.
    fn caret_to_point(&mut self, p: Point) -> bool {
        let Some((idx, _)) = self.edit_target() else {
            return false;
        };
        let el = &self.doc.elements[idx];
        let local = el.transform().inverse() * p;
        if !el.local_bounds().inflate(HIT_TOLERANCE, HIT_TOLERANCE).contains(local) {
            return false;
        }
        let (_, size, line_height) = Self::font(el);
        let (lines, source) = self.edit_lines(idx);
        let row = ((local.y / (size * line_height)).floor().max(0.0) as usize).min(lines.len() - 1);
        let at = self.offset_at_x(idx, &lines[row], source, local.x);
        if let Some(edit) = &mut self.editing {
            edit.caret = at;
        }
        true
    }

    // ----- style ----------------------------------------------------------

    /// Sets one property of `el` if it applies to that kind of element.
    fn apply_to(el: &mut Element, action: Action) -> bool {
        let kind = &el.kind;
        match action {
            Action::Stroke(color) => el.stroke_color = color.to_owned(),
            Action::Opacity(opacity) => el.opacity = opacity,
            Action::Background(color) if is_fillable(kind) => el.background_color = color.to_owned(),
            Action::Fill(fill) if is_fillable(kind) => el.fill_style = fill,
            Action::StrokeWidth(width) if has_stroke_width(kind) => el.stroke_width = width,
            Action::StrokeStyle(style) if is_stroked(kind) => el.stroke_style = style,
            Action::Roughness(roughness) if is_stroked(kind) => el.roughness = roughness,
            Action::Round(round) if has_edges(kind) => {
                let kind = if *kind == Kind::Rectangle { Roundness::ADAPTIVE } else { Roundness::PROPORTIONAL };
                el.roundness = round.then_some(Roundness { kind, value: None });
            }
            Action::FontSize(size) if is_text(kind) => el.font_size = Some(size),
            Action::FontFamily(family) if is_text(kind) => {
                el.font_family = Some(family);
                el.line_height = Some(text::metrics(family).line_height);
            }
            Action::TextAlign(align) if is_text(kind) => el.text_align = Some(align.to_owned()),
            _ => return false,
        }
        true
    }

    /// Elements the style panel acts on: the selection with its labels, or
    /// the text being edited.
    fn styled(&self) -> Vec<usize> {
        match self.edit_target() {
            Some((idx, _)) => vec![idx],
            None => self.targets(),
        }
    }

    /// Handles a click on a toolbar or style-panel button. `undoable` is
    /// false for the follow-up steps of one continuous gesture.
    fn apply(&mut self, action: Action, undoable: bool) {
        match action {
            Action::Tool(tool) => return self.set_tool(tool),
            Action::Layer(layer) => return self.reorder(layer),
            Action::Palette(target) => {
                self.palette = (self.palette != Some(target)).then_some(target);
                return;
            }
            Action::Stroke(color) => self.style.stroke_color = color.to_owned(),
            Action::Background(color) => self.style.background_color = color.to_owned(),
            Action::Fill(fill) => self.style.fill_style = fill,
            Action::StrokeWidth(width) => self.style.stroke_width = width,
            Action::StrokeStyle(style) => self.style.stroke_style = style,
            Action::Roughness(roughness) => self.style.roughness = roughness,
            Action::Round(round) => self.style.round = round,
            Action::FontSize(size) => self.style.font_size = size,
            Action::FontFamily(family) => self.style.font_family = family,
            Action::TextAlign(align) => self.style.text_align = align.to_owned(),
            Action::Opacity(opacity) => self.style.opacity = opacity,
        }
        let targets = self.styled();
        if targets.is_empty() {
            return;
        }
        // While typing, the edit as a whole is the undo step.
        if undoable && self.editing.is_none() {
            self.checkpoint();
        }
        let nonce = self.random_int();
        for i in targets {
            let el = &mut self.doc.elements[i];
            if Self::apply_to(el, action) {
                Self::touch(el, nonce);
                if is_text(&el.kind) {
                    self.relayout_text(i);
                }
            }
        }
    }

    /// The values the style panel shows (those of the affected elements
    /// where they have them, the defaults for new elements otherwise) and
    /// the option groups that apply.
    fn shown_style(&self) -> (Style, Sections) {
        let mut style = self.style.clone();
        let elements: Vec<&Element> = self.styled().into_iter().map(|i| &self.doc.elements[i]).collect();
        let tool_kind = element_kind(self.tool);
        let kinds: Vec<&Kind> = match elements.is_empty() {
            true => tool_kind.iter().collect(),
            false => elements.iter().map(|e| &e.kind).collect(),
        };
        let any = |test: fn(&Kind) -> bool| kinds.iter().any(|k| test(k));
        let first = |test: fn(&Kind) -> bool| elements.iter().find(|e| test(&e.kind));

        if let Some(e) = elements.first() {
            style.stroke_color = e.stroke_color.clone();
            style.opacity = e.opacity;
        }
        if let Some(e) = first(is_fillable) {
            style.background_color = e.background_color.clone();
            style.fill_style = e.fill_style;
        }
        if let Some(e) = first(has_stroke_width) {
            style.stroke_width = e.stroke_width;
        }
        if let Some(e) = first(is_stroked) {
            style.stroke_style = e.stroke_style;
            style.roughness = e.roughness;
        }
        if let Some(e) = first(has_edges) {
            style.round = e.roundness.is_some();
        }
        if let Some(e) = first(is_text) {
            (style.font_family, style.font_size, _) = Self::font(e);
            style.text_align = e.text_align.clone().unwrap_or_else(|| "left".to_owned());
        }

        let has_fill = !Color::parse(&style.background_color).is_some_and(Color::is_transparent);
        let sections = Sections {
            background: any(is_fillable),
            fill: any(is_fillable) && has_fill,
            stroke_width: any(has_stroke_width),
            stroke_style: any(is_stroked),
            roughness: any(is_stroked),
            edges: any(has_edges),
            text: any(is_text),
            layers: !elements.is_empty() && self.editing.is_none(),
        };
        (style, sections)
    }

    fn chrome(&self) -> Chrome {
        let (style, sections) = self.shown_style();
        let show_panel = !self.selected.is_empty() || self.editing.is_some() || element_kind(self.tool).is_some();
        ui::layout(self.size, self.tool, show_panel.then_some((&style, sections)), self.palette)
    }

    /// Moves the selection (and its labels) in the stacking order.
    fn reorder(&mut self, layer: Layer) {
        let targets: HashSet<usize> = self.targets().into_iter().collect();
        if targets.is_empty() {
            return;
        }
        self.checkpoint();
        let n = self.doc.elements.len();
        let mut order: Vec<usize> = (0..n).collect();
        match layer {
            Layer::ToFront => order.sort_by_key(|i| targets.contains(i)),
            Layer::ToBack => order.sort_by_key(|i| !targets.contains(i)),
            // One step: each selected element swaps with its unselected neighbor.
            Layer::Forward => {
                for i in (0..n.saturating_sub(1)).rev() {
                    if targets.contains(&order[i]) && !targets.contains(&order[i + 1]) {
                        order.swap(i, i + 1);
                    }
                }
            }
            Layer::Backward => {
                for i in 1..n {
                    if targets.contains(&order[i]) && !targets.contains(&order[i - 1]) {
                        order.swap(i, i - 1);
                    }
                }
            }
        }
        let mut slots: Vec<Option<Element>> = std::mem::take(&mut self.doc.elements).into_iter().map(Some).collect();
        self.doc.elements = order.into_iter().filter_map(|i| slots[i].take()).collect();
    }

    // ----- pointer --------------------------------------------------------

    pub fn pointer_down(&mut self, pos: Point, button: Button, mods: Modifiers) {
        self.pointer_down_with_pressure(pos, button, mods, None);
    }

    pub fn pointer_down_with_pressure(&mut self, pos: Point, button: Button, mods: Modifiers, pressure: Option<f64>) {
        self.pointer = Some(pos);
        if button == Button::Primary && self.drag.is_none() {
            let hit = self.chrome().hit(pos);
            // The palette stays open only while it is being used.
            if !matches!(hit, Some(ui::Hit::Button(Action::Palette(_)) | ui::Hit::Panel)) {
                self.palette = None;
            }
            match hit {
                Some(ui::Hit::Button(action)) => return self.apply(action, true),
                Some(ui::Hit::Slider(value)) => {
                    self.apply(Action::Opacity(value), true);
                    self.drag = Some(Drag::Slider);
                    return;
                }
                Some(ui::Hit::Panel) => return,
                None => {}
            }
        }
        let p = self.to_scene(pos);
        if self.placing && button == Button::Primary {
            return self.placing_click(p, pos);
        }
        if self.editing.is_some() {
            // A click on the text moves the caret; anywhere else ends the edit.
            if button != Button::Primary || !self.caret_to_point(p) {
                self.commit_text();
            }
            return;
        }
        let pan = button == Button::Middle
            || (button == Button::Primary && (self.space_held || self.tool == Tool::Hand));
        if pan {
            self.drag = Some(Drag::Pan { last: pos });
        } else if button == Button::Primary {
            match element_kind(self.tool) {
                None => self.select_down(p, pos, mods),
                Some(Kind::Text) => match self.top_hit(p).filter(|&i| is_text(&self.doc.elements[i].kind)) {
                    Some(existing) => self.start_edit(existing),
                    None => self.create_text(p, None),
                },
                Some(kind) => {
                    self.checkpoint();
                    let drag = if kind == Kind::Freedraw { Drag::Draw } else { Drag::Create { origin: p } };
                    let mut el = self.new_element(kind.clone(), p);
                    if kind == Kind::Freedraw {
                        let simulate = pressure.is_none();
                        el.simulate_pressure = Some(simulate);
                        if let Some(pr) = pressure {
                            el.pressures.push(pr.clamp(0.0, 1.0));
                        }
                    }
                    self.doc.elements.push(el);
                    self.drag = Some(drag);
                }
            }
        }
    }

    /// The shell reports the second press of a double click here instead of
    /// through [`Self::pointer_down`]. Edits the text under the pointer, or
    /// starts a new one (inside the shape under the pointer, if any).
    pub fn double_click(&mut self, pos: Point) {
        if self.placing {
            // The first click of the pair already fixed the last point.
            return self.finish_placing();
        }
        if self.tool != Tool::Select || self.editing.is_some() || self.chrome().hit(pos).is_some() {
            return;
        }
        let p = self.to_scene(pos);
        let Some(idx) = self.top_hit(p) else {
            return self.create_text(p, None);
        };
        let el = &self.doc.elements[idx];
        if is_text(&el.kind) {
            self.start_edit(idx);
        } else if is_container(&el.kind) {
            match self.bound_text(&el.id) {
                Some(label) => self.start_edit(label),
                None => self.create_text(p, Some(idx)),
            }
        }
    }

    /// Index of the topmost element under the scene point `p`.
    fn top_hit(&self, p: Point) -> Option<usize> {
        let tolerance = HIT_TOLERANCE / self.zoom;
        self.doc.elements.iter().rposition(|e| hit(e, p, tolerance))
    }

    fn select_down(&mut self, p: Point, screen: Point, mods: Modifiers) {
        if let (Some(grip), Some(el)) = (self.grip_hit(screen), self.single_selection()) {
            self.drag = Some(match grip {
                Grip::Box(handle) => {
                    let original = Box::new(el.clone());
                    let bounds = original.local_bounds();
                    let along = |t: i8, a: f64, b: f64| a + (b - a) * f64::from(t + 1) / 2.0;
                    let edge =
                        Point::new(along(handle.hx, bounds.x0, bounds.x1), along(handle.hy, bounds.y0, bounds.y1));
                    let grab = original.transform() * edge - p;
                    Drag::Resize { handle, original, grab, started: false }
                }
                Grip::Point(index) => {
                    let far_end = if index == 0 { End::End } else { End::Start };
                    let other = binding::read(el, far_end);
                    Drag::Point { id: el.id.clone(), index, other, insert: false, started: false }
                }
                Grip::Mid(segment) => {
                    Drag::Point { id: el.id.clone(), index: segment + 1, other: None, insert: true, started: false }
                }
                Grip::Rotate => Drag::Rotate { id: el.id.clone(), started: false },
            });
            return;
        }
        // A label stands for its container.
        let top = self.top_hit(p).map(|i| {
            let el = &self.doc.elements[i];
            self.container_of(el).map_or(el, |c| &self.doc.elements[c]).id.clone()
        });
        match top {
            Some(id) => {
                if mods.shift {
                    if !self.selected.remove(&id) {
                        self.selected.insert(id);
                    }
                } else if !self.selected.contains(&id) {
                    self.selected = HashSet::from([id]);
                }
                self.select_groups();
                self.drag = Some(Drag::Move { last: p, started: false });
            }
            // Dragging from inside the selection box moves the selection even
            // where the shapes themselves are hollow.
            None if !self.single_selection().is_some_and(|e| has_point_grips(&e.kind))
                && self.selection_boxes().any(|(to_screen, rect)| rect.contains(to_screen.inverse() * screen)) =>
            {
                self.drag = Some(Drag::Move { last: p, started: false });
            }
            None => {
                if !mods.shift {
                    self.selected.clear();
                }
                self.drag = Some(Drag::Marquee { origin: p, current: p });
            }
        }
    }

    pub fn pointer_move(&mut self, pos: Point, mods: Modifiers) {
        self.pointer_move_with_pressure(pos, mods, None);
    }

    pub fn pointer_move_with_pressure(&mut self, pos: Point, mods: Modifiers, pressure: Option<f64>) {
        self.pointer = Some(pos);
        let p = self.to_scene(pos);
        let nonce = self.random_int();
        let Some(mut drag) = self.drag.take() else {
            if self.placing {
                self.placing_move(p, nonce);
            }
            return;
        };
        match &mut drag {
            Drag::Pan { last } => {
                self.scroll += (pos - *last) / self.zoom;
                *last = pos;
            }
            Drag::Create { origin } => {
                if let Some(el) = self.doc.elements.last_mut() {
                    drag_out(el, *origin, p, mods.shift);
                    Self::touch(el, nonce);
                    if el.kind == Kind::Arrow {
                        // Attach to the shapes under both ends, live.
                        let tip = *origin + Vec2::new(el.points[1][0], el.points[1][1]);
                        let idx = self.doc.elements.len() - 1;
                        let (start, finish) = (self.candidate_binding(idx, *origin), self.candidate_binding(idx, tip));
                        self.bind_hover = finish.as_ref().map(|b| b.element_id.clone());
                        self.apply_bindings(idx, start, finish);
                    }
                }
            }
            Drag::Draw => {
                if let Some(el) = self.doc.elements.last_mut() {
                    let point = [p.x - el.x, p.y - el.y];
                    if el.points.last() != Some(&point) {
                        el.points.push(point);
                        if el.simulate_pressure == Some(false) {
                            el.pressures.push(pressure.unwrap_or(0.5).clamp(0.0, 1.0));
                        } else if let Some(pr) = pressure {
                            let clamped = pr.clamp(0.0, 1.0);
                            el.simulate_pressure = Some(false);
                            el.pressures.resize(el.points.len() - 1, clamped);
                            el.pressures.push(clamped);
                        }
                        let bounds = el.local_bounds();
                        (el.width, el.height) = (bounds.width(), bounds.height());
                        Self::touch(el, nonce);
                    }
                }
            }
            Drag::Move { last, started } => {
                let delta = p - *last;
                if delta != Vec2::ZERO {
                    if !*started {
                        self.checkpoint();
                        *started = true;
                    }
                    self.translate(delta, nonce);
                    *last = p;
                }
            }
            Drag::Resize { handle, original, grab, started } => {
                if !*started {
                    self.checkpoint();
                    *started = true;
                }
                if let Some(i) = self.index_of(&original.id) {
                    let el = &mut self.doc.elements[i];
                    resize(el, original, *handle, p + *grab, mods.shift);
                    Self::touch(el, nonce);
                    if let Some(label) = self.bound_text(&original.id) {
                        self.relayout_text(label);
                    }
                    self.refresh_bound_arrows(&HashSet::from([original.id.clone()]));
                }
            }
            Drag::Rotate { id, started } => {
                if !*started {
                    self.checkpoint();
                    *started = true;
                }
                if let Some(idx) = self.index_of(id) {
                    let el = &mut self.doc.elements[idx];
                    // The knob sits above the element, so "up" is angle zero.
                    let toward = p - el.transform() * el.local_bounds().center();
                    if toward.hypot() > 1e-6 {
                        let mut angle = toward.atan2() + FRAC_PI_2;
                        if mods.shift {
                            let step = ROTATE_SNAP.to_radians();
                            angle = (angle / step).round() * step;
                        }
                        el.angle = angle.rem_euclid(TAU);
                        Self::touch(el, nonce);
                        if let Some(label) = self.bound_text(id) {
                            self.relayout_text(label);
                        }
                        self.refresh_bound_arrows(&HashSet::from([id.clone()]));
                    }
                }
            }
            Drag::Point { id, index, other, insert, started } => {
                let first_move = !*started;
                if first_move {
                    self.checkpoint();
                    *started = true;
                }
                if let Some(idx) = self.index_of(id) {
                    let el = &mut self.doc.elements[idx];
                    let mut points = binding::global_points(el);
                    if first_move && *insert {
                        points.insert(*index, p);
                    }
                    points[*index] = p;
                    binding::set_global_points(el, &points);
                    Self::touch(el, nonce);
                    let end = match *index {
                        0 => Some(End::Start),
                        i if i == points.len() - 1 => Some(End::End),
                        _ => None,
                    };
                    if let Some(end) = end.filter(|_| el.kind == Kind::Arrow) {
                        // The dragged end attaches to what it is over; the far
                        // end keeps its shape and turns to face the new point.
                        let this = self.candidate_binding(idx, p);
                        self.bind_hover = this.as_ref().map(|b| b.element_id.clone());
                        let (start, finish) = match end {
                            End::Start => (this, other.clone()),
                            End::End => (other.clone(), this),
                        };
                        self.apply_bindings(idx, start, finish);
                    } else {
                        // Bound ends aim at their neighboring point, which may
                        // be the one that just moved.
                        self.refresh_arrow(idx);
                    }
                }
            }
            Drag::Marquee { origin, current } => {
                *current = p;
                let area = Rect::from_points(*origin, p);
                self.selected = self
                    .live()
                    .filter(|e| e.container_id.is_none() && area.contains_rect(e.scene_bounds()))
                    .map(|e| e.id.clone())
                    .collect();
                self.select_groups();
            }
            Drag::Slider => {
                if let Some(value) = self.chrome().slider_value(pos) {
                    self.apply(Action::Opacity(value), false);
                }
            }
        }
        self.drag = Some(drag);
    }

    pub fn pointer_up(&mut self, _pos: Point, _button: Button, _mods: Modifiers) {
        self.bind_hover = None;
        match self.drag.take() {
            Some(Drag::Create { .. }) => {
                let Some(el) = self.doc.elements.last() else {
                    return;
                };
                let b = el.local_bounds();
                if b.width() < 2.0 && b.height() < 2.0 && has_point_grips(&el.kind) {
                    // A click without a drag starts a line placed point by
                    // point: the click is its first point, the next floats.
                    let idx = self.doc.elements.len() - 1;
                    let at = Point::new(el.x, el.y);
                    let start = self.candidate_binding(idx, at);
                    let el = &mut self.doc.elements[idx];
                    binding::set_global_points(el, &[at, at]);
                    self.set_binding(idx, End::Start, start);
                    self.set_binding(idx, End::End, None);
                    self.placing = true;
                } else if b.width() < 2.0 && b.height() < 2.0 {
                    // A click without a drag: back to the scene as it was, so
                    // neither the empty shape nor an undo entry is left.
                    if let Some(before) = self.undo.pop() {
                        self.doc.elements = before;
                    }
                } else {
                    self.selected = HashSet::from([el.id.clone()]);
                    self.tool = Tool::Select;
                }
            }
            // Freedraw stays active so several strokes can follow each other.
            Some(_) | None => {}
        }
    }

    /// Scrolls by `delta` screen pixels, or zooms around `pos` with ctrl.
    pub fn wheel(&mut self, delta: Vec2, pos: Point, mods: Modifiers) {
        if mods.ctrl {
            self.zoom_at(pos, (delta.y * 0.002).exp());
        } else if mods.shift && delta.x == 0.0 {
            self.scroll.x += delta.y / self.zoom;
        } else {
            self.scroll += delta / self.zoom;
        }
    }

    // ----- keyboard -------------------------------------------------------

    pub fn key_down(&mut self, key: Key, mods: Modifiers) {
        if self.editing.is_some() {
            return self.edit_key(key, mods);
        }
        if self.placing && matches!(key, Key::Enter | Key::Escape) {
            return self.finish_placing();
        }
        let center = Point::new(self.size.width / 2.0, self.size.height / 2.0);
        match key {
            Key::Space => self.space_held = true,
            Key::Escape => {
                self.selected.clear();
                self.tool = Tool::Select;
            }
            Key::Delete | Key::Backspace => self.delete_selected(),
            Key::Enter => {
                // Enter edits the selected text, or the label of the selected shape.
                let label = self.single_selection().and_then(|e| match e.kind {
                    Kind::Text => self.index_of(&e.id),
                    _ => self.bound_text(&e.id),
                });
                if let Some(idx) = label {
                    self.start_edit(idx);
                }
            }
            Key::Left => self.nudge(Vec2::new(-1.0, 0.0), mods),
            Key::Right => self.nudge(Vec2::new(1.0, 0.0), mods),
            Key::Up => self.nudge(Vec2::new(0.0, -1.0), mods),
            Key::Down => self.nudge(Vec2::new(0.0, 1.0), mods),
            Key::Home | Key::End => {}
            Key::Char(c) if mods.ctrl => match c {
                'z' if mods.shift => self.redo(),
                'z' => self.undo(),
                'y' => self.redo(),
                'a' => {
                    self.tool = Tool::Select;
                    self.selected =
                        self.live().filter(|e| e.container_id.is_none()).map(|e| e.id.clone()).collect();
                }
                'd' => self.duplicate_selected(),
                'g' if mods.shift => self.ungroup_selected(),
                'g' => self.group_selected(),
                '0' => self.zoom_at(center, 1.0 / self.zoom),
                '+' | '=' => self.zoom_at(center, 1.1),
                '-' => self.zoom_at(center, 1.0 / 1.1),
                _ => {}
            },
            Key::Char(c) => {
                if let Some(tool) = tool_from_key(c).filter(|_| self.drag.is_none()) {
                    self.set_tool(tool);
                }
            }
        }
    }

    pub fn key_up(&mut self, key: Key) {
        if key == Key::Space {
            self.space_held = false;
        }
    }

    pub fn undo(&mut self) {
        self.commit_text();
        self.finish_placing();
        if let Some(previous) = self.undo.pop() {
            self.redo.push(std::mem::replace(&mut self.doc.elements, previous));
            self.after_history_jump();
        }
    }

    pub fn redo(&mut self) {
        self.commit_text();
        self.finish_placing();
        if let Some(next) = self.redo.pop() {
            self.undo.push(std::mem::replace(&mut self.doc.elements, next));
            self.after_history_jump();
        }
    }

    fn after_history_jump(&mut self) {
        self.drag = None;
        self.modified = true;
        let elements = &self.doc.elements;
        self.selected.retain(|id| elements.iter().any(|e| !e.is_deleted && e.id == *id));
    }

    fn translate(&mut self, delta: Vec2, nonce: i64) {
        let targets = self.targets();
        let moved: HashSet<String> = targets.iter().map(|&i| self.doc.elements[i].id.clone()).collect();
        for &i in &targets {
            let el = &mut self.doc.elements[i];
            el.x += delta.x;
            el.y += delta.y;
            Self::touch(el, nonce);
        }
        // An arrow dragged away from a shape that stays behind lets go of it.
        for &i in &targets {
            for end in End::BOTH {
                let stays = |b: Binding| !moved.contains(&b.element_id);
                if binding::read(&self.doc.elements[i], end).is_some_and(stays) {
                    self.set_binding(i, end, None);
                }
            }
        }
        self.refresh_bound_arrows(&moved);
    }

    fn delete_selected(&mut self) {
        let targets = self.targets();
        if targets.is_empty() {
            return;
        }
        self.checkpoint();
        let nonce = self.random_int();
        let gone: HashSet<String> = targets.iter().map(|&i| self.doc.elements[i].id.clone()).collect();
        for i in 0..self.doc.elements.len() {
            // Deleted arrows let go of their shapes, and surviving arrows of
            // deleted shapes.
            let is_gone = gone.contains(&self.doc.elements[i].id);
            for end in End::BOTH {
                let dangling = |b: Binding| is_gone || gone.contains(&b.element_id);
                if binding::read(&self.doc.elements[i], end).is_some_and(dangling) {
                    self.set_binding(i, end, None);
                }
            }
        }
        for i in targets {
            // Soft delete, as Excalidraw does; dropped when saving.
            let el = &mut self.doc.elements[i];
            el.is_deleted = true;
            Self::touch(el, nonce);
        }
        self.selected.clear();
    }

    fn duplicate_selected(&mut self) {
        let copies: Vec<Element> = self.targets().into_iter().map(|i| self.doc.elements[i].clone()).collect();
        if !copies.is_empty() {
            self.checkpoint();
            self.insert_copies(copies, Vec2::new(DUPLICATE_OFFSET, DUPLICATE_OFFSET));
        }
    }

    /// Adds `copies` to the scene under fresh ids, shifted by `offset`, and
    /// selects them. Links among them (labels, groups, arrow bindings) are
    /// re-pointed at the copies; links to anything else are dropped.
    fn insert_copies(&mut self, mut copies: Vec<Element>, offset: Vec2) {
        let mut fresh = |keys: Vec<String>| -> HashMap<String, String> {
            keys.into_iter().map(|key| (key, self.random_id())).collect()
        };
        let ids = fresh(copies.iter().map(|e| e.id.clone()).collect());
        let groups = fresh(copies.iter().flat_map(|e| e.group_ids.clone()).collect());
        self.selected.clear();
        for el in &mut copies {
            el.id = ids[&el.id].clone();
            el.seed = self.random_int();
            el.version_nonce = self.random_int();
            el.x += offset.x;
            el.y += offset.y;
            for group in &mut el.group_ids {
                *group = groups[group.as_str()].clone();
            }
            el.container_id = el.container_id.take().and_then(|id| ids.get(&id).cloned());
            if let Some(list) = el.extra.get_mut("boundElements").and_then(Value::as_array_mut) {
                list.retain_mut(|link| match link["id"].as_str().and_then(|id| ids.get(id)) {
                    Some(copy) => {
                        link["id"] = Value::String(copy.clone());
                        true
                    }
                    None => false,
                });
            }
            for end in End::BOTH {
                // An arrow stays attached only to shapes copied along with it.
                if let Some(bound) = binding::read(el, end) {
                    let copy = ids.get(&bound.element_id).map(|id| Binding { element_id: id.clone(), ..bound });
                    binding::write(el, end, copy.as_ref());
                }
            }
            if el.container_id.is_none() {
                self.selected.insert(el.id.clone());
            }
        }
        self.doc.elements.extend(copies);
    }

    /// The selection in Excalidraw's clipboard format, for the shell to put
    /// on the system clipboard. `None` if nothing is selected.
    pub fn copy(&self) -> Option<String> {
        let elements: Vec<&Element> = self.targets().into_iter().map(|i| &self.doc.elements[i]).collect();
        (!elements.is_empty())
            .then(|| json!({ "type": CLIPBOARD_TYPE, "elements": elements, "files": {} }).to_string())
    }

    /// Like [`Self::copy`], and deletes the selection.
    pub fn cut(&mut self) -> Option<String> {
        let copied = self.copy()?;
        self.delete_selected();
        Some(copied)
    }

    /// Pastes clipboard text at the pointer: elements copied from this
    /// engine or from Excalidraw as elements, anything else as a new text
    /// (or into the text being edited).
    pub fn paste(&mut self, clipboard: &str) {
        if self.editing.is_some() {
            return self.text_input(clipboard);
        }
        let center = Point::new(self.size.width / 2.0, self.size.height / 2.0);
        let at = self.to_scene(self.pointer.unwrap_or(center));
        let elements = serde_json::from_str::<Value>(clipboard)
            .ok()
            .filter(|v| v["type"] == CLIPBOARD_TYPE || v["type"] == "excalidraw")
            .and_then(|mut v| serde_json::from_value::<Vec<Element>>(v["elements"].take()).ok());
        match elements {
            Some(mut elements) => {
                elements.retain(|e| !e.is_deleted);
                binding::migrate_legacy(&mut elements);
                let Some(bounds) = elements.iter().map(Element::scene_bounds).reduce(|a, b| a.union(b)) else {
                    return;
                };
                self.checkpoint();
                self.tool = Tool::Select;
                self.insert_copies(elements, at - bounds.center());
            }
            None if !clipboard.trim().is_empty() => {
                self.create_text(at, None);
                self.text_input(clipboard.trim_end());
                self.commit_text();
            }
            None => {}
        }
    }

    fn nudge(&mut self, dir: Vec2, mods: Modifiers) {
        if self.selected.is_empty() {
            return;
        }
        self.checkpoint();
        let nonce = self.random_int();
        self.translate(dir * if mods.shift { 5.0 } else { 1.0 }, nonce);
    }

    // ----- selection geometry ---------------------------------------------

    /// For each selected element: its local→screen transform and its padded
    /// local bounds.
    fn selection_boxes(&self) -> impl Iterator<Item = (Affine, Rect)> + '_ {
        let pad = SELECTION_PADDING / self.zoom;
        let view = self.view();
        self.live()
            .filter(|e| self.selected.contains(&e.id))
            .map(move |e| (view * e.transform(), e.local_bounds().inflate(pad, pad)))
    }

    /// What can be grabbed on a single selected element, with screen
    /// positions: the points of a line or arrow, resize handles otherwise.
    fn grips(&self) -> Vec<(Grip, Point)> {
        let Some(el) = self.single_selection() else {
            return Vec::new();
        };
        if has_point_grips(&el.kind) {
            let view = self.view();
            let points: Vec<Point> = binding::global_points(el).iter().map(|&p| view * p).collect();
            let mids = points
                .windows(2)
                .enumerate()
                .filter(|(_, w)| w[0].distance(w[1]) >= MID_GRIP_MIN_SEGMENT)
                .map(|(i, w)| (Grip::Mid(i), w[0].midpoint(w[1])));
            // Points come first so they win over a nearby midpoint.
            return points.iter().enumerate().map(|(i, &p)| (Grip::Point(i), p)).chain(mids).collect();
        }
        let Some((to_screen, rect)) = self.selection_boxes().next() else {
            return Vec::new();
        };
        // Text scales as a whole, so it only offers its corners.
        let corners_only = is_text(&el.kind);
        let along = |t: i8, a: f64, b: f64| a + (b - a) * f64::from(t + 1) / 2.0;
        let knob = Point::new(rect.center().x, rect.y0 - ROTATE_GRIP_OFFSET / self.zoom);
        HANDLES
            .into_iter()
            .filter(|h| !corners_only || (h.hx != 0 && h.hy != 0))
            .map(|h| {
                let local = Point::new(along(h.hx, rect.x0, rect.x1), along(h.hy, rect.y0, rect.y1));
                (Grip::Box(h), to_screen * local)
            })
            .chain([(Grip::Rotate, to_screen * knob)])
            .collect()
    }

    fn grip_hit(&self, screen: Point) -> Option<Grip> {
        self.grips()
            .into_iter()
            .find(|(_, at)| at.distance(screen) <= HANDLE_SIZE / 2.0 + HIT_TOLERANCE / 2.0)
            .map(|(grip, _)| grip)
    }

    // ----- rendering ------------------------------------------------------

    fn cached_ops(&mut self, index: usize) -> Rc<Vec<DrawOp>> {
        let el = &self.doc.elements[index];
        let key = (el.version, el.version_nonce);
        match self.cache.get(&el.id) {
            Some((k, ops)) if *k == key => ops.clone(),
            _ => {
                let ops = Rc::new(element_ops(el));
                self.cache.insert(el.id.clone(), (key, ops.clone()));
                ops
            }
        }
    }

    /// Builds the display list for the current state.
    pub fn frame(&mut self) -> Frame {
        let view = self.view();
        let window = self.size.to_rect();
        let mut items = Vec::new();
        for i in 0..self.doc.elements.len() {
            let el = &self.doc.elements[i];
            let margin = el.stroke_width * 4.0 + 30.0;
            let on_screen = view.transform_rect_bbox(el.scene_bounds().inflate(margin, margin));
            if el.is_deleted || on_screen.intersect(window).is_zero_area() {
                continue;
            }
            let (transform, opacity) = (view * el.transform(), (el.opacity / 100.0) as f32);
            let ops = self.cached_ops(i);
            if !ops.is_empty() {
                items.push(Item { transform, opacity, ops });
            }
        }
        // Forget shapes of elements that no longer exist (e.g. after undo).
        if self.cache.len() > self.doc.elements.len() * 2 + 64 {
            let ids: HashSet<&str> = self.doc.elements.iter().map(|e| e.id.as_str()).collect();
            self.cache.retain(|id, _| ids.contains(id.as_str()));
        }

        let mut overlay = Vec::new();
        self.selection_ops(&mut overlay);
        self.caret_ops(&mut overlay);
        self.chrome().draw(&mut overlay);
        items.push(Item { transform: Affine::IDENTITY, opacity: 1.0, ops: Rc::new(overlay) });

        Frame { background: background(&self.doc), items }
    }

    fn selection_ops(&self, ops: &mut Vec<DrawOp>) {
        let single = self.single_selection();
        if !single.is_some_and(|e| has_point_grips(&e.kind)) {
            for (to_screen, rect) in self.selection_boxes() {
                ops.push(DrawOp::stroke(to_screen * rect.to_path(0.1), ACCENT, 1.0));
            }
        }
        let half = HANDLE_SIZE / 2.0;
        let square = Rect::new(-half, -half, half, half).to_path(0.1);
        for (grip, at) in self.grips() {
            let shape = match grip {
                Grip::Box(_) => Affine::rotate(single.map_or(0.0, |e| e.angle)) * square.clone(),
                Grip::Point(_) | Grip::Rotate => Circle::new((0.0, 0.0), POINT_GRIP_RADIUS).to_path(0.1),
                Grip::Mid(_) => Circle::new((0.0, 0.0), MID_GRIP_RADIUS).to_path(0.1),
            };
            let path = Affine::translate(at.to_vec2()) * shape;
            // Midpoints are only suggestions, so they are drawn fainter.
            let alpha = if matches!(grip, Grip::Mid(_)) { 110 } else { 255 };
            ops.push(DrawOp::fill(path.clone(), Color::WHITE));
            ops.push(DrawOp::stroke(path, Color { a: alpha, ..ACCENT }, 1.0));
        }
        if let Some(shape) = self.bind_hover.as_deref().and_then(|id| self.index_of(id)) {
            // The shape an arrow end is about to attach to.
            let shape = &self.doc.elements[shape];
            let path = self.view() * shape.transform() * outline(shape);
            ops.push(DrawOp::stroke(path, Color { a: 150, ..ACCENT }, 3.0));
        }
        if let Some(Drag::Marquee { origin, current }) = &self.drag {
            let path = self.view() * Rect::from_points(*origin, *current).to_path(0.1);
            ops.push(DrawOp::fill(path.clone(), Color { a: 26, ..ACCENT }));
            ops.push(DrawOp::stroke(path, ACCENT, 1.0));
        }
    }

    fn caret_ops(&self, ops: &mut Vec<DrawOp>) {
        let Some((idx, caret)) = self.edit_target() else {
            return;
        };
        let el = &self.doc.elements[idx];
        let (_, size, line_height) = Self::font(el);
        let (lines, source) = self.edit_lines(idx);
        let row = Self::caret_line(&lines, caret);
        let x = self.caret_x(idx, &lines[row], source, caret);
        let top = row as f64 * size * line_height;
        let mut path = BezPath::new();
        path.move_to((x, top));
        path.line_to((x, top + size * line_height));
        let color = Color::parse(&el.stroke_color).unwrap_or(ACCENT);
        ops.push(DrawOp::stroke(self.view() * el.transform() * path, color, 1.5));
    }
}

/// Shapes a freshly created element from the drag `origin` → `p`.
/// Shift constrains boxes to squares and lines to 15° steps.
fn drag_out(el: &mut Element, origin: Point, p: Point, constrain: bool) {
    let mut d = p - origin;
    if matches!(el.kind, Kind::Line | Kind::Arrow) {
        if constrain {
            let step = 15f64.to_radians();
            let angle = (d.atan2() / step).round() * step;
            d = Vec2::from_angle(angle) * d.hypot();
        }
        el.points = vec![[0.0, 0.0], [d.x, d.y]];
        (el.width, el.height) = (d.x.abs(), d.y.abs());
    } else {
        if constrain {
            let side = d.x.abs().max(d.y.abs());
            d = Vec2::new(side.copysign(d.x), side.copysign(d.y));
        }
        (el.x, el.y) = (origin.x + d.x.min(0.0), origin.y + d.y.min(0.0));
        (el.width, el.height) = (d.x.abs(), d.y.abs());
    }
}

/// Resizes `el` (currently equal to `original` or an earlier step of the
/// same drag) so that the edges selected by `handle` follow the scene point
/// `p`. Works in the element's own rotated frame, so rotated shapes keep
/// their opposite edges anchored. Text always keeps its proportions and
/// scales its font instead.
fn resize(el: &mut Element, original: &Element, handle: Handle, p: Point, keep_aspect: bool) {
    // Images keep their proportions unless shift is held; shapes the reverse.
    let keep_aspect = (keep_aspect != (original.kind == Kind::Image)) || is_text(&original.kind);
    let bounds = original.local_bounds();
    let origin = Vec2::new(original.x, original.y);
    let center = bounds.center() + origin;
    let unrotate = Affine::rotate_about(-original.angle, center);
    let q = unrotate * p;

    let (mut x0, mut y0) = (bounds.x0 + origin.x, bounds.y0 + origin.y);
    let (mut x1, mut y1) = (bounds.x1 + origin.x, bounds.y1 + origin.y);
    match handle.hx {
        -1 => x0 = q.x,
        1 => x1 = q.x,
        _ => {}
    }
    match handle.hy {
        -1 => y0 = q.y,
        1 => y1 = q.y,
        _ => {}
    }
    if keep_aspect && handle.hx != 0 && handle.hy != 0 && bounds.width() > 0.0 && bounds.height() > 0.0 {
        let scale = ((x1 - x0) / bounds.width()).abs().max(((y1 - y0) / bounds.height()).abs());
        let (w, h) = (bounds.width() * scale, bounds.height() * scale);
        if handle.hx < 0 { x0 = x1 - w.copysign(x1 - x0) } else { x1 = x0 + w.copysign(x1 - x0) }
        if handle.hy < 0 { y0 = y1 - h.copysign(y1 - y0) } else { y1 = y0 + h.copysign(y1 - y0) }
    }

    // The box grew around a different center; find where that center really
    // is in the scene so the untouched edges stay put after rotation.
    let new_center = unrotate.inverse() * Point::new((x0 + x1) / 2.0, (y0 + y1) / 2.0);
    let (w, h) = (x1 - x0, y1 - y0);

    if original.is_linear() {
        let scale = |extent: f64, size: f64| if size > 0.0 { extent / size } else { 1.0 };
        let (sx, sy) = (scale(w, bounds.width()), scale(h, bounds.height()));
        el.points = original
            .points
            .iter()
            .map(|pt| [(pt[0] - bounds.x0) * sx, (pt[1] - bounds.y0) * sy])
            .collect();
        // Excalidraw expects the first point at the element's origin.
        if let Some(&[fx, fy]) = el.points.first() {
            for pt in &mut el.points {
                pt[0] -= fx;
                pt[1] -= fy;
            }
        }
        let scaled = el.local_bounds();
        (el.width, el.height) = (scaled.width(), scaled.height());
        (el.x, el.y) = (new_center.x - scaled.center().x, new_center.y - scaled.center().y);
    } else {
        (el.width, el.height) = (w.abs().max(1.0), h.abs().max(1.0));
        (el.x, el.y) = (new_center.x - el.width / 2.0, new_center.y - el.height / 2.0);
        if is_text(&original.kind) && original.height > 0.0 {
            // Line widths are proportional to the font size, so the wrapped
            // text still fits the scaled box exactly.
            let size = original.font_size.unwrap_or(text::DEFAULT_FONT_SIZE);
            el.font_size = Some(size * el.height / original.height);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::element::FillStyle;

    const NONE: Modifiers = Modifiers { shift: false, ctrl: false, alt: false };
    const CTRL: Modifiers = Modifiers { shift: false, ctrl: true, alt: false };

    fn drag(ed: &mut Editor, from: (f64, f64), to: (f64, f64)) {
        ed.pointer_down(from.into(), Button::Primary, NONE);
        ed.pointer_move(to.into(), NONE);
        ed.pointer_up(to.into(), Button::Primary, NONE);
    }

    fn click(ed: &mut Editor, at: (f64, f64)) {
        drag(ed, at, at);
    }

    fn press(ed: &mut Editor, action: Action) {
        let at = ed.chrome().buttons.iter().find(|b| b.action == action).expect("button is shown").rect.center();
        click(ed, at.into());
    }

    fn editor() -> Editor {
        let mut ed = Editor::new(1);
        ed.resize(1200.0, 900.0);
        ed
    }

    /// A 200×100 rectangle at (300, 300), clear of the toolbar and panel.
    fn editor_with_rect() -> Editor {
        let mut ed = editor();
        ed.key_down(Key::Char('r'), NONE);
        drag(&mut ed, (300.0, 300.0), (500.0, 400.0));
        ed
    }

    fn texts(ed: &Editor) -> Vec<&Element> {
        ed.live().filter(|e| e.kind == Kind::Text).collect()
    }

    #[test]
    fn draws_selects_and_undoes_a_rectangle() {
        let mut ed = editor_with_rect();
        let el = &ed.document().elements[0];
        assert_eq!((el.x, el.y, el.width, el.height), (300.0, 300.0, 200.0, 100.0));
        assert_eq!(ed.tool(), Tool::Select);
        assert!(ed.selected.contains(&el.id) && ed.is_modified());

        ed.key_down(Key::Char('z'), CTRL);
        assert!(ed.document().elements.is_empty());
        ed.key_down(Key::Char('y'), CTRL);
        assert_eq!(ed.document().elements.len(), 1);
    }

    #[test]
    fn a_click_with_a_shape_tool_leaves_nothing_behind() {
        let mut ed = editor();
        ed.key_down(Key::Char('o'), NONE);
        click(&mut ed, (450.0, 350.0));
        assert!(ed.document().elements.is_empty() && ed.undo.is_empty());
    }

    #[test]
    fn dragging_upwards_normalizes_the_box() {
        let mut ed = editor();
        ed.key_down(Key::Char('d'), NONE);
        drag(&mut ed, (500.0, 400.0), (400.0, 350.0));
        let el = &ed.document().elements[0];
        assert_eq!((el.x, el.y, el.width, el.height), (400.0, 350.0, 100.0, 50.0));
    }

    #[test]
    fn moves_then_deletes_the_selection() {
        let mut ed = editor_with_rect();
        // Grab the top edge (the shape is hollow), away from the resize handles.
        drag(&mut ed, (350.0, 300.0), (380.0, 340.0));
        let el = &ed.document().elements[0];
        assert_eq!((el.x, el.y), (330.0, 340.0));

        ed.key_down(Key::Delete, NONE);
        assert!(ed.document().elements[0].is_deleted);
        assert!(!ed.document().to_json().contains("rectangle"));
        ed.undo();
        assert!(!ed.document().elements[0].is_deleted);
    }

    #[test]
    fn marquee_selects_enclosed_elements_only() {
        let mut ed = editor_with_rect();
        ed.key_down(Key::Char('o'), NONE);
        drag(&mut ed, (600.0, 500.0), (700.0, 600.0));
        drag(&mut ed, (550.0, 450.0), (750.0, 650.0));
        assert_eq!(ed.selected.len(), 1);
        drag(&mut ed, (260.0, 260.0), (760.0, 660.0));
        assert_eq!(ed.selected.len(), 2);
    }

    #[test]
    fn resizing_a_rotated_shape_keeps_the_opposite_corner_fixed() {
        let original = Element {
            x: 100.0,
            y: 100.0,
            width: 200.0,
            height: 100.0,
            angle: 0.5,
            ..Element::default()
        };
        let fixed = original.transform() * Point::new(0.0, 0.0);
        let mut el = original.clone();
        resize(&mut el, &original, Handle { hx: 1, hy: 1 }, Point::new(420.0, 380.0), false);
        assert!((el.transform() * Point::new(0.0, 0.0)).distance(fixed) < 1e-9);
        let corner = el.transform() * Point::new(el.width, el.height);
        assert!(corner.distance(Point::new(420.0, 380.0)) < 1e-9);
    }

    #[test]
    fn resizing_a_line_scales_its_points() {
        let original = Element {
            kind: Kind::Line,
            x: 10.0,
            y: 10.0,
            points: vec![[0.0, 0.0], [100.0, 50.0]],
            ..Element::default()
        };
        let mut el = original.clone();
        resize(&mut el, &original, Handle { hx: 1, hy: 1 }, Point::new(210.0, 110.0), false);
        assert_eq!(el.points, vec![[0.0, 0.0], [200.0, 100.0]]);
        assert_eq!((el.x, el.y, el.width, el.height), (10.0, 10.0, 200.0, 100.0));
    }

    #[test]
    fn zoom_keeps_the_anchor_point_still() {
        let mut ed = editor();
        let anchor = Point::new(321.0, 423.0);
        let before = ed.to_scene(anchor);
        ed.wheel(Vec2::new(0.0, 120.0), anchor, CTRL);
        assert!(ed.zoom > 1.0);
        assert!(ed.to_scene(anchor).distance(before) < 1e-9);
    }

    #[test]
    fn toolbar_click_switches_tool_without_drawing() {
        let mut ed = editor();
        press(&mut ed, Action::Tool(Tool::Ellipse));
        assert_eq!(ed.tool(), Tool::Ellipse);
        assert!(ed.document().elements.is_empty());
    }

    #[test]
    fn frame_caches_shapes_until_they_change() {
        let mut ed = editor_with_rect();
        let first = ed.frame().items[0].ops.clone();
        assert!(Rc::ptr_eq(&first, &ed.frame().items[0].ops));
        ed.key_down(Key::Right, NONE);
        assert!(!Rc::ptr_eq(&first, &ed.frame().items[0].ops));
    }

    #[test]
    fn style_panel_restyles_the_selection_and_sets_defaults() {
        let mut ed = editor_with_rect();
        press(&mut ed, Action::Stroke("#e03131"));
        press(&mut ed, Action::Background("#a5d8ff"));
        press(&mut ed, Action::Fill(FillStyle::Hachure));
        press(&mut ed, Action::Round(false));
        let el = &ed.document().elements[0];
        assert_eq!((el.stroke_color.as_str(), el.background_color.as_str()), ("#e03131", "#a5d8ff"));
        assert!(el.fill_style == FillStyle::Hachure && el.roundness.is_none());
        assert!(ed.selected.contains(&el.id), "clicking the panel keeps the selection");

        // The next shape starts with the same style, and each click is one undo step.
        ed.key_down(Key::Char('o'), NONE);
        drag(&mut ed, (600.0, 500.0), (700.0, 600.0));
        assert_eq!(ed.document().elements[1].background_color, "#a5d8ff");
        ed.undo();
        ed.undo();
        assert!(ed.document().elements[0].roundness.is_some());
    }

    #[test]
    fn panel_hides_the_fill_row_until_there_is_a_background() {
        let mut ed = editor_with_rect();
        let has_fill = |ed: &Editor| ed.chrome().buttons.iter().any(|b| matches!(b.action, Action::Fill(_)));
        assert!(!has_fill(&ed));
        press(&mut ed, Action::Background("#ffc9c9"));
        assert!(has_fill(&ed));
        ed.key_down(Key::Escape, NONE);
        assert!(ed.chrome().buttons.iter().all(|b| matches!(b.action, Action::Tool(_))), "no selection, no panel");
    }

    #[test]
    fn opacity_slider_drag_is_a_single_undo_step() {
        let mut ed = editor_with_rect();
        let steps = ed.undo.len();
        // The slider is the only widget of the panel that is not a button.
        let panel_x = ed.chrome().buttons.iter().find(|b| b.action == Action::Stroke("#1e1e1e")).unwrap().rect.x0;
        let slider_y = (80..900)
            .map(f64::from)
            .find(|&y| matches!(ed.chrome().hit(Point::new(panel_x + 60.0, y)), Some(ui::Hit::Slider(_))))
            .expect("slider is shown");
        ed.pointer_down(Point::new(panel_x + 150.0, slider_y), Button::Primary, NONE);
        ed.pointer_move(Point::new(panel_x - 80.0, slider_y), NONE);
        ed.pointer_up(Point::new(panel_x - 80.0, slider_y), Button::Primary, NONE);
        assert_eq!(ed.document().elements[0].opacity, 0.0);
        assert_eq!(ed.undo.len(), steps + 1);
    }

    #[test]
    fn types_a_text_with_the_text_tool() {
        let mut ed = editor();
        ed.key_down(Key::Char('t'), NONE);
        click(&mut ed, (400.0, 400.0));
        assert!(ed.is_editing_text());
        ed.text_input("Hola");
        ed.key_down(Key::Enter, NONE);
        ed.text_input("mundo\u{8}");
        ed.key_down(Key::Char('r'), NONE);
        ed.key_down(Key::Backspace, NONE);
        ed.key_down(Key::Escape, NONE);

        assert!(!ed.is_editing_text() && ed.tool() == Tool::Select);
        let el = texts(&ed)[0];
        assert_eq!(el.text.as_deref(), Some("Hola\nmund"));
        assert_eq!(el.original_text.as_deref(), Some("Hola\nmund"));
        assert_eq!((el.x, el.y, el.height), (400.0, 387.5, 50.0));
        assert_eq!(el.width, text::line_width("mund", 5, 20.0));
        assert!(ed.selected.contains(&el.id));

        ed.undo();
        assert!(texts(&ed).is_empty(), "the whole edit is one undo step");
    }

    #[test]
    fn an_empty_text_vanishes_without_an_undo_step() {
        let mut ed = editor_with_rect();
        ed.mark_saved();
        let steps = ed.undo.len();
        ed.double_click(Point::new(700.0, 600.0));
        assert!(ed.is_editing_text());
        ed.text_input("  ");
        click(&mut ed, (900.0, 700.0));
        assert!(!ed.is_editing_text() && texts(&ed).is_empty());
        assert_eq!(ed.document().elements.len(), 1);
        assert!(ed.undo.len() == steps && !ed.is_modified());
    }

    #[test]
    fn caret_moves_and_edits_in_the_middle() {
        let mut ed = editor();
        ed.double_click(Point::new(400.0, 400.0));
        ed.text_input("abc\ndéf");
        for key in [Key::Up, Key::Home, Key::Right] {
            ed.key_down(key, NONE);
        }
        ed.text_input("X");
        ed.key_down(Key::Down, NONE);
        ed.key_down(Key::End, NONE);
        ed.key_down(Key::Left, NONE);
        ed.key_down(Key::Backspace, NONE);
        ed.key_down(Key::Delete, NONE);
        ed.key_down(Key::Enter, CTRL);
        assert_eq!(texts(&ed)[0].text.as_deref(), Some("aXbc\nd"));
    }

    #[test]
    fn double_click_labels_a_shape_and_the_label_follows_it() {
        let mut ed = editor_with_rect();
        ed.double_click(Point::new(400.0, 300.0));
        ed.text_input("una etiqueta bastante larga que no cabe en una sola línea");
        ed.key_down(Key::Escape, NONE);

        let rect = ed.document().elements[0].clone();
        let label = texts(&ed)[0].clone();
        assert_eq!(label.container_id.as_deref(), Some(rect.id.as_str()));
        assert_eq!(rect.extra["boundElements"][0]["id"], label.id.as_str());
        assert!(label.text.as_deref().unwrap().contains('\n'), "wrapped to the container");
        assert!(label.width <= max_width(&rect) && label.height <= max_height(&rect));
        assert!(rect.height > 100.0, "the container grew to fit");
        let center = |e: &Element| e.scene_bounds().center();
        assert!(center(&label).distance(center(&rect)) < 1e-6);
        assert!(ed.selected.contains(&rect.id) && ed.selected.len() == 1);

        // Clicking the label grabs the container; both move, copy and die together.
        let grab = center(&label);
        drag(&mut ed, (grab.x, grab.y), (grab.x + 50.0, grab.y + 30.0));
        let (moved, label) = (ed.document().elements[0].clone(), texts(&ed)[0].clone());
        assert_eq!((moved.x, moved.y), (rect.x + 50.0, rect.y + 30.0));
        assert!(center(&label).distance(center(&moved)) < 1e-6);

        ed.key_down(Key::Char('d'), CTRL);
        assert_eq!(texts(&ed).len(), 2);
        let copy = texts(&ed)[1];
        let holder = ed.live().find(|e| Some(&e.id) == copy.container_id.as_ref()).unwrap();
        assert!(holder.id != rect.id && holder.extra["boundElements"][0]["id"] == copy.id.as_str());

        ed.key_down(Key::Delete, NONE);
        assert_eq!(texts(&ed).len(), 1);
    }

    #[test]
    fn resizing_a_container_rewraps_its_label() {
        let mut ed = editor_with_rect();
        ed.double_click(Point::new(400.0, 300.0));
        ed.text_input("uno dos tres cuatro cinco seis");
        ed.key_down(Key::Escape, NONE);
        let lines = |ed: &Editor| texts(ed)[0].text.as_deref().unwrap().lines().count();
        let before = lines(&ed);
        assert!(before > 1);
        // Drag the right-middle handle far to the right.
        let rect = ed.document().elements[0].clone();
        let handle = (rect.x + rect.width + SELECTION_PADDING, rect.y + rect.height / 2.0);
        drag(&mut ed, handle, (handle.0 + 400.0, handle.1));
        assert_eq!(ed.document().elements[0].width, rect.width + 400.0);
        assert_eq!(lines(&ed), 1);
        let center = |e: &Element| e.scene_bounds().center();
        assert!(center(texts(&ed)[0]).distance(center(&ed.document().elements[0])) < 1e-6);
    }

    #[test]
    fn font_changes_relayout_the_text() {
        let mut ed = editor();
        ed.double_click(Point::new(500.0, 500.0));
        ed.text_input("Hola");
        ed.key_down(Key::Escape, NONE);
        let before = texts(&ed)[0].clone();
        press(&mut ed, Action::FontSize(36.0));
        press(&mut ed, Action::FontFamily(8));
        let after = texts(&ed)[0];
        assert_eq!((after.font_size, after.font_family), (Some(36.0), Some(8)));
        assert_eq!(after.height, 36.0 * 1.25);
        assert!(after.width > before.width && after.width == text::line_width("Hola", 8, 36.0));
    }

    /// Rectangles at x 300..400 and 700..800 (y 300..400) joined by an arrow
    /// drawn from the middle of one to the middle of the other.
    fn editor_with_linked_shapes() -> Editor {
        let mut ed = editor();
        for x in [300.0, 700.0] {
            ed.key_down(Key::Char('r'), NONE);
            drag(&mut ed, (x, 300.0), (x + 100.0, 400.0));
        }
        ed.key_down(Key::Char('a'), NONE);
        drag(&mut ed, (350.0, 350.0), (750.0, 350.0));
        ed
    }

    fn ends(arrow: &Element) -> (Point, Point) {
        let points = binding::global_points(arrow);
        (points[0], points[points.len() - 1])
    }

    fn links(el: &Element) -> usize {
        el.extra.get("boundElements").and_then(Value::as_array).map_or(0, Vec::len)
    }

    #[test]
    fn a_new_arrow_attaches_to_the_shapes_under_its_ends() {
        let ed = editor_with_linked_shapes();
        let [left, right, arrow] = &ed.document().elements[..] else { panic!("three elements") };
        let (start, end) = ends(arrow);
        // Both ends stop a gap (5 + half the stroke) short of the outlines.
        assert!(start.distance(Point::new(406.0, 350.0)) < 0.1, "{start:?}");
        assert!(end.distance(Point::new(694.0, 350.0)) < 0.1, "{end:?}");
        assert_eq!(arrow.extra["startBinding"]["elementId"], left.id.as_str());
        assert_eq!(arrow.extra["endBinding"]["mode"], "orbit");
        assert_eq!(left.extra["boundElements"][0], json!({ "type": "arrow", "id": arrow.id }));
        assert_eq!((links(left), links(right)), (1, 1));
        assert_eq!(arrow.points[0], [0.0, 0.0]);
    }

    #[test]
    fn bound_arrows_follow_their_shapes() {
        let mut ed = editor_with_linked_shapes();
        // Move the right rectangle down by grabbing its top edge.
        click(&mut ed, (750.0, 300.0));
        drag(&mut ed, (720.0, 300.0), (720.0, 500.0));
        let (start, end) = ends(&ed.document().elements[2]);
        assert!((start.x - 406.0).abs() < 0.1 && start.y > 350.0, "{start:?}");
        assert!((end.x - 694.0).abs() < 0.1 && (end.y - 550.0).abs() < 45.0, "{end:?}");

        // Resizing moves the attachment point with the shape.
        let handle = (800.0 + SELECTION_PADDING, 550.0);
        drag(&mut ed, handle, (handle.0 + 100.0, handle.1));
        let wider = ends(&ed.document().elements[2]).1;
        assert!(wider != end && (690.0..705.0).contains(&wider.x), "{wider:?}");

        ed.undo();
        ed.undo();
        assert!(ends(&ed.document().elements[2]).1.distance(Point::new(694.0, 350.0)) < 0.1);
    }

    #[test]
    fn deleting_a_shape_frees_its_arrows() {
        let mut ed = editor_with_linked_shapes();
        click(&mut ed, (750.0, 300.0));
        ed.key_down(Key::Delete, NONE);
        let arrow = &ed.document().elements[2];
        assert!(arrow.extra["endBinding"].is_null() && arrow.extra["startBinding"].is_object());

        // And deleting the arrow clears the shape's back-reference.
        click(&mut ed, (550.0, 350.0));
        ed.key_down(Key::Delete, NONE);
        assert_eq!(links(&ed.document().elements[0]), 0);
    }

    #[test]
    fn dragging_an_arrow_end_rebinds_it() {
        let mut ed = editor_with_linked_shapes();
        // The new arrow is selected and shows a grip on each point.
        let tip = ends(&ed.document().elements[2]).1;
        drag(&mut ed, (tip.x, tip.y), (600.0, 600.0));
        let [left, right, arrow] = &ed.document().elements[..] else { panic!("three elements") };
        assert!(arrow.extra["endBinding"].is_null() && links(right) == 0 && links(left) == 1);
        let (start, end) = ends(arrow);
        assert_eq!(end, Point::new(600.0, 600.0));
        assert!(start.x > 390.0 && start.y > 390.0, "the bound end turns toward the free one: {start:?}");

        // Back onto the left shape: both ends in one shape go inside it.
        drag(&mut ed, (600.0, 600.0), (320.0, 380.0));
        let [left, _, arrow] = &ed.document().elements[..] else { panic!("three elements") };
        assert_eq!(arrow.extra["endBinding"]["mode"], "inside");
        assert_eq!(arrow.extra["startBinding"]["mode"], "inside");
        assert_eq!(links(left), 1);
        assert!(ends(arrow).1.distance(Point::new(320.0, 380.0)) < 1e-6);
    }

    #[test]
    fn moving_an_arrow_alone_detaches_it_but_moving_it_with_its_shapes_does_not() {
        let mut ed = editor_with_linked_shapes();
        ed.key_down(Key::Char('a'), CTRL);
        ed.key_down(Key::Down, NONE);
        let arrow = &ed.document().elements[2];
        assert!(arrow.extra["startBinding"].is_object() && arrow.extra["endBinding"].is_object());
        assert!(ends(arrow).0.distance(Point::new(406.0, 351.0)) < 0.1);

        ed.key_down(Key::Escape, NONE);
        click(&mut ed, (550.0, 351.0));
        ed.key_down(Key::Down, NONE);
        let [left, right, arrow] = &ed.document().elements[..] else { panic!("three elements") };
        assert!(arrow.extra["startBinding"].is_null() && arrow.extra["endBinding"].is_null());
        assert_eq!((links(left), links(right)), (0, 0));
    }

    #[test]
    fn duplicating_keeps_bindings_only_between_copies() {
        let mut ed = editor_with_linked_shapes();
        ed.key_down(Key::Char('a'), CTRL);
        ed.key_down(Key::Char('d'), CTRL);
        let els = &ed.document().elements;
        assert_eq!(els.len(), 6);
        assert_eq!(els[5].extra["startBinding"]["elementId"], els[3].id.as_str());
        assert_eq!(els[3].extra["boundElements"][0]["id"], els[5].id.as_str());

        // An arrow copied without its shapes is free.
        ed.key_down(Key::Escape, NONE);
        click(&mut ed, (550.0, 350.0));
        ed.key_down(Key::Char('d'), CTRL);
        let copy = ed.document().elements.last().unwrap();
        assert!(copy.extra["startBinding"].is_null() && copy.extra["endBinding"].is_null());
    }

    #[test]
    fn lines_are_edited_by_their_points() {
        let mut ed = editor();
        ed.key_down(Key::Char('l'), NONE);
        drag(&mut ed, (300.0, 300.0), (500.0, 400.0));
        let grips: Vec<Grip> = ed.grips().into_iter().map(|(g, _)| g).collect();
        assert_eq!(grips, [Grip::Point(0), Grip::Point(1), Grip::Mid(0)]);
        drag(&mut ed, (300.0, 300.0), (250.0, 450.0));
        let line = &ed.document().elements[0];
        assert_eq!(ends(line), (Point::new(250.0, 450.0), Point::new(500.0, 400.0)));
        assert_eq!((line.points[0], line.width, line.height), ([0.0, 0.0], 250.0, 50.0));
    }

    #[test]
    fn rotation_knob_turns_a_shape_with_its_label_and_arrows() {
        let mut ed = editor_with_linked_shapes();
        click(&mut ed, (350.0, 300.0));
        ed.double_click(Point::new(350.0, 300.0));
        ed.text_input("A");
        ed.key_down(Key::Escape, NONE);
        // The knob sits above the box; dragging it to the right is a quarter turn.
        let knob = |ed: &Editor| ed.grips().into_iter().find(|(g, _)| *g == Grip::Rotate).unwrap().1;
        let at = knob(&ed);
        assert!((at.x - 350.0).abs() < 1e-6 && at.y < 300.0);
        drag(&mut ed, (at.x, at.y), (500.0, 350.0));
        let els = &ed.document().elements;
        assert!((els[0].angle - FRAC_PI_2).abs() < 1e-9 && els[1].angle == els[0].angle);
        assert!(els[1].scene_bounds().center().distance(Point::new(350.0, 350.0)) < 1e-6);
        // The arrow still starts on the turned shape's outline.
        assert!((ends(&els[3]).0.distance(Point::new(350.0, 350.0)) - 56.0).abs() < 0.5);

        // The knob turned with the shape; with shift it snaps to 15° steps.
        let at = knob(&ed);
        assert!(at.x > 400.0 && (at.y - 350.0).abs() < 1e-6);
        ed.pointer_down(at, Button::Primary, NONE);
        ed.pointer_move(Point::new(340.0, 500.0), Modifiers { shift: true, ..NONE });
        ed.pointer_up(Point::new(340.0, 500.0), Button::Primary, NONE);
        assert!((ed.document().elements[0].angle.to_degrees() - 180.0).abs() < 1e-9);
        ed.undo();
        assert!((ed.document().elements[0].angle - FRAC_PI_2).abs() < 1e-9);
    }

    #[test]
    fn copy_and_paste_round_trip_through_the_clipboard_format() {
        let mut ed = editor_with_linked_shapes();
        assert_eq!(ed.copy().map(|c| c.contains("\"arrow\"")), Some(true), "the new arrow is selected");
        ed.key_down(Key::Char('a'), CTRL);
        let copied = ed.cut().unwrap();
        assert!(ed.live().next().is_none());
        assert_eq!(serde_json::from_str::<Value>(&copied).unwrap()["type"], "excalidraw/clipboard");

        ed.pointer_move(Point::new(600.0, 600.0), NONE);
        ed.paste(&copied);
        let pasted: Vec<&Element> = ed.live().collect();
        assert_eq!(pasted.len(), 3);
        assert_eq!(ed.selected.len(), 3);
        let bounds = pasted.iter().map(|e| e.scene_bounds()).reduce(|a, b| a.union(b)).unwrap();
        assert!(bounds.center().distance(Point::new(600.0, 600.0)) < 1e-6, "centered on the pointer");
        assert_eq!(pasted[2].extra["endBinding"]["elementId"], pasted[1].id.as_str());
        assert_eq!(pasted[1].extra["boundElements"][0]["id"], pasted[2].id.as_str());
        ed.undo();
        assert!(ed.live().next().is_none());
    }

    #[test]
    fn pasting_plain_text_makes_a_text_element_or_types_into_the_edit() {
        let mut ed = editor();
        ed.pointer_move(Point::new(500.0, 500.0), NONE);
        ed.paste("dos\tlíneas\r\naquí\n");
        let el = texts(&ed)[0];
        assert_eq!(el.text.as_deref(), Some("dos        líneas\naquí"));
        assert!(ed.selected.contains(&el.id) && !ed.is_editing_text());
        assert_eq!(ed.undo.len(), 1);

        ed.key_down(Key::Enter, NONE);
        ed.paste("!");
        ed.key_down(Key::Escape, NONE);
        assert_eq!(texts(&ed)[0].text.as_deref(), Some("dos        líneas\naquí!"));
        ed.paste("   ");
        ed.paste("{\"type\": \"excalidraw/clipboard\", \"elements\": []}");
        assert_eq!(ed.live().count(), 1);
    }

    #[test]
    fn palette_popup_sets_any_color_and_closes() {
        let mut ed = editor_with_rect();
        let custom = Action::Background("#da77f2");
        assert!(!ed.chrome().buttons.iter().any(|b| b.action == custom));
        press(&mut ed, Action::Palette(ColorTarget::Background));
        press(&mut ed, custom);
        assert_eq!(ed.document().elements[0].background_color, "#da77f2");
        assert!(ed.palette.is_none() && !ed.chrome().buttons.iter().any(|b| b.action == custom));

        // Clicking the canvas dismisses it without picking anything.
        press(&mut ed, Action::Palette(ColorTarget::Stroke));
        assert_eq!(ed.palette, Some(ColorTarget::Stroke));
        click(&mut ed, (900.0, 700.0));
        assert!(ed.palette.is_none() && ed.document().elements[0].stroke_color == "#1e1e1e");
    }

    #[test]
    fn dragging_a_midpoint_adds_a_point() {
        let mut ed = editor_with_linked_shapes();
        // The arrow runs along y = 350 between the shapes; pull its middle down.
        drag(&mut ed, (550.0, 350.0), (550.0, 450.0));
        let arrow = &ed.document().elements[2];
        assert_eq!(arrow.points.len(), 3);
        assert_eq!(binding::global_points(arrow)[1], Point::new(550.0, 450.0));
        // Both ends stay bound and now leave their shapes toward the new point.
        let (start, end) = ends(arrow);
        assert!(start.y > 351.0 && end.y > 351.0, "{start:?} {end:?}");
        assert!(arrow.extra["startBinding"].is_object() && arrow.extra["endBinding"].is_object());
        ed.undo();
        assert_eq!(ed.document().elements[2].points.len(), 2);
    }

    #[test]
    fn clicking_places_a_line_point_by_point() {
        let mut ed = editor();
        ed.key_down(Key::Char('l'), NONE);
        click(&mut ed, (300.0, 300.0));
        assert!(ed.placing && ed.tool() == Tool::Line);
        ed.pointer_move(Point::new(400.0, 320.0), NONE);
        assert_eq!(ends(&ed.document().elements[0]).1, Point::new(400.0, 320.0), "the next point follows the pointer");
        click(&mut ed, (400.0, 300.0));
        click(&mut ed, (400.0, 400.0));
        ed.pointer_move(Point::new(600.0, 600.0), NONE);
        ed.key_down(Key::Enter, NONE);

        let line = &ed.document().elements[0];
        let expected = [(300.0, 300.0), (400.0, 300.0), (400.0, 400.0)].map(Point::from);
        assert_eq!(binding::global_points(line), expected);
        assert!(!ed.placing && ed.tool() == Tool::Select && ed.selected.contains(&line.id));
        ed.undo();
        assert!(ed.document().elements.is_empty(), "the whole line is one undo step");
    }

    #[test]
    fn placing_ends_on_double_click_or_a_click_on_the_last_point() {
        let mut ed = editor_with_linked_shapes();
        ed.key_down(Key::Escape, NONE);
        ed.key_down(Key::Char('a'), NONE);
        click(&mut ed, (350.0, 380.0));
        click(&mut ed, (550.0, 600.0));
        // Second press of a double click on the right shape.
        ed.pointer_move(Point::new(750.0, 380.0), NONE);
        ed.pointer_down(Point::new(750.0, 380.0), Button::Primary, NONE);
        ed.pointer_up(Point::new(750.0, 380.0), Button::Primary, NONE);
        ed.double_click(Point::new(750.0, 380.0));
        let els = &ed.document().elements;
        assert_eq!((els.len(), els[3].points.len()), (4, 3));
        assert_eq!(els[3].extra["startBinding"]["elementId"], els[0].id.as_str());
        assert_eq!(els[3].extra["endBinding"]["elementId"], els[1].id.as_str());
        assert_eq!((links(&els[0]), links(&els[1])), (2, 2));

        // A lone first click followed by Escape leaves nothing behind.
        let steps = ed.undo.len();
        ed.key_down(Key::Char('l'), NONE);
        click(&mut ed, (500.0, 700.0));
        ed.key_down(Key::Escape, NONE);
        assert_eq!((ed.document().elements.len(), ed.undo.len()), (4, steps));

        ed.key_down(Key::Char('l'), NONE);
        click(&mut ed, (500.0, 700.0));
        click(&mut ed, (600.0, 700.0));
        click(&mut ed, (600.0, 700.0));
        assert!(!ed.placing);
        assert_eq!(ed.document().elements[4].points, vec![[0.0, 0.0], [100.0, 0.0]]);
    }

    #[test]
    fn image_tool_asks_the_shell_and_insert_places_the_image() {
        let mut ed = editor();
        ed.key_down(Key::Char('r'), NONE);
        press(&mut ed, Action::Tool(Tool::Image));
        assert_eq!(ed.take_request(), Some(Request::PickImage));
        assert!(ed.take_request().is_none() && ed.tool() == Tool::Rectangle);

        ed.pointer_move(Point::new(600.0, 500.0), NONE);
        ed.insert_image("data:image/png;base64,AAAA", "image/png", 1600.0, 1200.0, 1.0e12);
        let el = &ed.document().elements[0];
        // 1200px tall does not fit: capped to half the 900px window.
        assert_eq!((el.width, el.height), (600.0, 450.0));
        assert_eq!(el.scene_bounds().center(), Point::new(600.0, 500.0));
        let id = el.extra["fileId"].as_str().unwrap();
        assert_eq!(id.len(), 40);
        assert_eq!(ed.document().files[id]["dataURL"], "data:image/png;base64,AAAA");
        assert_eq!(ed.document().files[id]["mimeType"], "image/png");
        assert!(ed.selected.contains(&el.id) && ed.tool() == Tool::Select);
        assert!(matches!(element_ops(el)[0].style, crate::draw::Style::Image { .. }));

        // Corner handles keep the picture's proportions.
        let corner = (900.0 + SELECTION_PADDING, 725.0 + SELECTION_PADDING);
        drag(&mut ed, corner, (corner.0 - 300.0, corner.1 - 10.0));
        let el = &ed.document().elements[0];
        assert!((el.width / el.height - 4.0 / 3.0).abs() < 1e-9 && el.width < 600.0);
        ed.undo();
        ed.undo();
        assert!(ed.document().elements.is_empty());
    }

    #[test]
    fn groups_select_together() {
        let mut ed = editor_with_rect();
        ed.key_down(Key::Char('o'), NONE);
        drag(&mut ed, (600.0, 500.0), (700.0, 600.0));
        ed.key_down(Key::Char('a'), CTRL);
        ed.key_down(Key::Char('g'), CTRL);
        ed.key_down(Key::Escape, NONE);
        click(&mut ed, (350.0, 300.0));
        assert_eq!(ed.selected.len(), 2);
        ed.key_down(Key::Char('g'), Modifiers { shift: true, ..CTRL });
        ed.key_down(Key::Escape, NONE);
        click(&mut ed, (350.0, 300.0));
        assert_eq!(ed.selected.len(), 1);
    }

    #[test]
    fn layer_buttons_reorder_the_selection() {
        let mut ed = editor_with_rect();
        ed.key_down(Key::Char('o'), NONE);
        drag(&mut ed, (600.0, 500.0), (700.0, 600.0));
        let kinds = |ed: &Editor| ed.document().elements.iter().map(|e| e.kind.clone()).collect::<Vec<_>>();
        press(&mut ed, Action::Layer(Layer::ToBack));
        assert_eq!(kinds(&ed), [Kind::Ellipse, Kind::Rectangle]);
        press(&mut ed, Action::Layer(Layer::Forward));
        assert_eq!(kinds(&ed), [Kind::Rectangle, Kind::Ellipse]);
        ed.undo();
        assert_eq!(kinds(&ed), [Kind::Ellipse, Kind::Rectangle]);
    }

    #[test]
    fn freedraw_records_real_or_simulated_pressure() {
        let mut ed = editor();
        ed.key_down(Key::Char('p'), NONE);

        // Mouse stroke: no pressure -> simulate_pressure = Some(true), pressures empty.
        drag(&mut ed, (300.0, 300.0), (360.0, 320.0));
        let mouse_el = &ed.document().elements[0];
        assert_eq!(mouse_el.simulate_pressure, Some(true));
        assert!(mouse_el.pressures.is_empty());

        // Stylus stroke: real pressure -> simulate_pressure = Some(false), pressures per point.
        ed.pointer_down_with_pressure(Point::new(400.0, 300.0), Button::Primary, NONE, Some(0.25));
        ed.pointer_move_with_pressure(Point::new(440.0, 310.0), NONE, Some(0.8));
        ed.pointer_up(Point::new(440.0, 310.0), Button::Primary, NONE);
        let pen_el = &ed.document().elements[1];
        assert_eq!(pen_el.simulate_pressure, Some(false));
        assert_eq!(pen_el.pressures, vec![0.25, 0.8]);
    }
}
