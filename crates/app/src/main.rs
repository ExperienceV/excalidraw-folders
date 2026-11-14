//! Desktop shell: a winit window painted with vello (wgpu). All drawing
//! logic lives in `board-core`; this file only translates input events
//! and turns the core's display list into GPU draw calls.

use std::error::Error;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use board_core::kurbo::{Point, Rect, Vec2};
use base64::Engine;
use board_core::{export, Button, Cursor, Document, Editor, Key, Modifiers, Request};
use vello::util::{RenderContext, RenderSurface};
use vello::wgpu;
use vello::{AaConfig, RenderParams, Renderer, Scene};
use winit::application::ApplicationHandler;
use winit::cursor::CursorIcon;
use winit::dpi::LogicalSize;
use winit::event::{ButtonSource, ElementState, Ime, MouseButton, MouseScrollDelta, PointerSource, WindowEvent};
use winit::event_loop::{ActiveEventLoop, EventLoop};
use winit::keyboard::{Key as WinitKey, NamedKey};
use winit::window::{
    ImeCapabilities, ImeEnableRequest, ImeRequest, ImeRequestData, Window, WindowAttributes, WindowId,
};

mod render;

use render::{encode, renderer_options, to_peniko, Images};

const IMAGE_EXTENSIONS: [&str; 6] = ["png", "jpg", "jpeg", "gif", "webp", "bmp"];
const DRAWING_EXTENSION: &str = "excalidraw";

fn load_document(path: &Path) -> Result<Document, String> {
    let json = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
    Document::from_json(&json).map_err(|e| e.to_string())
}

fn now_ms() -> f64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0.0, |d| d.as_millis() as f64)
}

/// Logical pixels scrolled per mouse-wheel notch.
const WHEEL_LINE: f64 = 40.0;
/// Two presses this close in time and space make a double click.
const DOUBLE_CLICK_TIME: Duration = Duration::from_millis(400);
const DOUBLE_CLICK_DISTANCE: f64 = 4.0;

struct App {
    context: RenderContext,
    renderers: Vec<Option<Renderer>>,
    surface: Option<(RenderSurface<'static>, Arc<dyn Window>)>,
    scene: Scene,
    editor: Editor,
    path: PathBuf,
    mods: Modifiers,
    /// Last cursor position, in logical pixels.
    cursor: Point,
    /// When and where the primary button was last pressed.
    last_press: Option<(Instant, Point)>,
    /// System clipboard, opened on first use.
    clipboard: Option<arboard::Clipboard>,
    images: Images,
}

impl App {
    fn is_double_click(&self) -> bool {
        self.last_press.is_some_and(|(at, pos)| {
            at.elapsed() <= DOUBLE_CLICK_TIME && pos.distance(self.cursor) <= DOUBLE_CLICK_DISTANCE
        })
    }

    /// The system clipboard, or `None` (reported once per attempt) where
    /// the compositor gives no access to it.
    fn clipboard(&mut self) -> Option<&mut arboard::Clipboard> {
        if self.clipboard.is_none() {
            self.clipboard = arboard::Clipboard::new().map_err(|err| eprintln!("clipboard unavailable: {err}")).ok();
        }
        self.clipboard.as_mut()
    }

    fn copy(&mut self, cut: bool) {
        let copied = if cut { self.editor.cut() } else { self.editor.copy() };
        if let (Some(text), Some(clipboard)) = (copied, self.clipboard()) {
            if let Err(err) = clipboard.set_text(text) {
                eprintln!("could not copy: {err}");
            }
        }
    }

    /// Pastes text (elements or plain text) if the clipboard has any, an
    /// image otherwise.
    fn paste(&mut self) {
        let Some(clipboard) = self.clipboard() else {
            return;
        };
        if let Ok(text) = clipboard.get_text() {
            return self.editor.paste(&text);
        }
        let Ok(bitmap) = clipboard.get_image() else {
            return;
        };
        let (width, height) = (bitmap.width as u32, bitmap.height as u32);
        let mut png = Vec::new();
        let encoded = image::RgbaImage::from_raw(width, height, bitmap.bytes.into_owned())
            .ok_or_else(|| "unexpected bitmap size".to_owned())
            .and_then(|pixels| {
                let format = image::ImageFormat::Png;
                pixels.write_to(&mut std::io::Cursor::new(&mut png), format).map_err(|e| e.to_string())
            });
        match encoded {
            Ok(()) => self.insert_image(&png),
            Err(err) => eprintln!("could not paste the image: {err}"),
        }
    }

    /// Adds an encoded image file (PNG, JPEG, ...) to the drawing.
    fn insert_image(&mut self, bytes: &[u8]) {
        let decoded = image::guess_format(bytes).and_then(|format| {
            let pixels = image::load_from_memory_with_format(bytes, format)?;
            Ok((format.to_mime_type(), pixels.width(), pixels.height()))
        });
        match decoded {
            Ok((mime, width, height)) => {
                let encoded = base64::engine::general_purpose::STANDARD.encode(bytes);
                let data_url = format!("data:{mime};base64,{encoded}");
                self.editor.insert_image(&data_url, mime, f64::from(width), f64::from(height), now_ms());
            }
            Err(err) => eprintln!("could not read the image: {err}"),
        }
    }

    fn pick_image(&mut self) {
        let picked = rfd::FileDialog::new().add_filter("Imágenes", &IMAGE_EXTENSIONS).pick_file();
        if let Some(path) = picked {
            match std::fs::read(&path) {
                Ok(bytes) => self.insert_image(&bytes),
                Err(err) => eprintln!("could not read {}: {err}", path.display()),
            }
        }
    }

    /// Asks for a drawing and opens it. Unsaved changes to the current one
    /// are saved first; if that fails, nothing is opened.
    fn open(&mut self) {
        let picked = rfd::FileDialog::new().add_filter("Excalidraw", &[DRAWING_EXTENSION, "json"]).pick_file();
        let Some(path) = picked else {
            return;
        };
        match load_document(&path) {
            Ok(doc) if !self.editor.is_modified() || self.save() => {
                self.editor.load(doc);
                self.path = path;
                self.images = Images::default();
            }
            Ok(_) => {}
            Err(err) => eprintln!("could not open {}: {err}", path.display()),
        }
    }

    fn save_as(&mut self) {
        let name = self.path.file_name().unwrap_or_default().to_string_lossy().into_owned();
        let picked = rfd::FileDialog::new().add_filter("Excalidraw", &[DRAWING_EXTENSION]).set_file_name(name).save_file();
        if let Some(mut path) = picked {
            if path.extension().is_none() {
                path.set_extension(DRAWING_EXTENSION);
            }
            self.path = path;
            self.save();
        }
    }

    /// Handles the application-level shortcuts (with ctrl held); returns
    /// whether `key` was one.
    fn shortcut(&mut self, key: Key, shift: bool) -> bool {
        match (key, shift) {
            (Key::Char('s'), false) => {
                self.save();
            }
            (Key::Char('s'), true) => self.save_as(),
            (Key::Char('o'), false) => self.open(),
            (Key::Char('c'), false) => self.copy(false),
            (Key::Char('x'), false) => self.copy(true),
            (Key::Char('v'), false) => self.paste(),
            _ => return false,
        }
        true
    }

    /// Writes the drawing to its file; returns whether it worked.
    fn save(&mut self) -> bool {
        match std::fs::write(&self.path, self.editor.document().to_json()) {
            Ok(()) => self.editor.mark_saved(),
            Err(err) => eprintln!("could not save {}: {err}", self.path.display()),
        }
        !self.editor.is_modified()
    }

    fn update_title(&self) {
        if let Some((_, window)) = &self.surface {
            let name = self.path.file_name().unwrap_or_default().to_string_lossy();
            let mark = if self.editor.is_modified() { " •" } else { "" };
            window.set_title(&format!("{name}{mark} — Board"));
        }
    }

    fn render(&mut self) {
        let Some((surface, window)) = &mut self.surface else {
            return;
        };
        let (width, height) = (surface.config.width, surface.config.height);
        let frame = self.editor.frame();
        let viewport = Rect::new(0.0, 0.0, f64::from(width), f64::from(height));
        let files = &self.editor.document().files;
        encode(&mut self.scene, &frame, window.scale_factor(), viewport, &mut self.images, files);

        let device = &self.context.devices[surface.dev_id];
        let renderer = self.renderers[surface.dev_id].as_mut().expect("renderer exists for the surface's device");
        let params = RenderParams {
            base_color: to_peniko(frame.background),
            width,
            height,
            antialiasing_method: AaConfig::Area,
        };
        if let Err(err) =
            renderer.render_to_texture(&device.device, &device.queue, &self.scene, &surface.target_view, &params)
        {
            eprintln!("render failed: {err}");
            return;
        }

        use wgpu::CurrentSurfaceTexture as Current;
        let target = match surface.surface.get_current_texture() {
            Current::Success(t) | Current::Suboptimal(t) => t,
            Current::Outdated | Current::Lost => {
                self.context.configure_surface(surface);
                window.request_redraw();
                return;
            }
            // Occluded or timed out: nothing to show this frame.
            _ => return,
        };
        let mut encoder = device.device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
        surface.blitter.copy(
            &device.device,
            &mut encoder,
            &surface.target_view,
            &target.texture.create_view(&wgpu::TextureViewDescriptor::default()),
        );
        device.queue.submit([encoder.finish()]);
        window.pre_present_notify();
        device.queue.present(target);
    }
}

fn tablet_pressure(source: &PointerSource) -> Option<f64> {
    match source {
        PointerSource::TabletTool { data, .. } => data.force.map(|f| f.normalized(None)),
        PointerSource::Touch { force, .. } => force.map(|f| f.normalized(None)),
        _ => None,
    }
}

fn button_pressure(source: &ButtonSource) -> Option<f64> {
    match source {
        ButtonSource::TabletTool { data, .. } => data.force.map(|f| f.normalized(None)),
        ButtonSource::Touch { force, .. } => force.map(|f| f.normalized(None)),
        _ => None,
    }
}

fn map_button(button: MouseButton) -> Option<Button> {
    match button {
        MouseButton::Left => Some(Button::Primary),
        MouseButton::Middle => Some(Button::Middle),
        MouseButton::Right => Some(Button::Secondary),
        _ => None,
    }
}

fn map_key(key: &WinitKey) -> Option<Key> {
    Some(match key {
        WinitKey::Named(NamedKey::Delete) => Key::Delete,
        WinitKey::Named(NamedKey::Backspace) => Key::Backspace,
        WinitKey::Named(NamedKey::Enter) => Key::Enter,
        WinitKey::Named(NamedKey::Home) => Key::Home,
        WinitKey::Named(NamedKey::End) => Key::End,
        WinitKey::Named(NamedKey::Escape) => Key::Escape,
        WinitKey::Character(s) if s.as_str() == " " => Key::Space,
        WinitKey::Named(NamedKey::ArrowLeft) => Key::Left,
        WinitKey::Named(NamedKey::ArrowRight) => Key::Right,
        WinitKey::Named(NamedKey::ArrowUp) => Key::Up,
        WinitKey::Named(NamedKey::ArrowDown) => Key::Down,
        WinitKey::Character(s) => Key::Char(s.chars().next()?.to_ascii_lowercase()),
        _ => return None,
    })
}

impl ApplicationHandler for App {
    fn can_create_surfaces(&mut self, event_loop: &dyn ActiveEventLoop) {
        if self.surface.is_some() {
            return;
        }
        let attributes = WindowAttributes::default().with_surface_size(LogicalSize::new(1280, 800));
        let window: Arc<dyn Window> =
            Arc::from(event_loop.create_window(attributes).expect("failed to create window"));
        let size = window.surface_size();
        let surface = pollster::block_on(self.context.create_surface(
            window.clone(),
            size.width.max(1),
            size.height.max(1),
            wgpu::PresentMode::AutoVsync,
        ))
        .expect("failed to create a GPU surface");

        self.renderers.resize_with(self.context.devices.len(), || None);
        self.renderers[surface.dev_id].get_or_insert_with(|| {
            Renderer::new(&self.context.devices[surface.dev_id].device, renderer_options())
                .expect("failed to create the vello renderer")
        });

        let logical = size.to_logical::<f64>(window.scale_factor());
        self.editor.resize(logical.width, logical.height);
        self.editor.zoom_to_fit();
        // Lets input methods (dead keys, CJK, emoji pickers) compose text.
        if let Some(req) = ImeEnableRequest::new(ImeCapabilities::default(), ImeRequestData::default()) {
            let _ = window.request_ime_update(ImeRequest::Enable(req));
        }
        window.request_redraw();
        self.surface = Some((surface, window));
        self.update_title();
    }

    fn window_event(&mut self, event_loop: &dyn ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        let Some((surface, window)) = &mut self.surface else {
            return;
        };
        let scale = window.scale_factor();
        match event {
            WindowEvent::CloseRequested => {
                event_loop.exit();
                return;
            }
            WindowEvent::RedrawRequested => {
                self.render();
                return;
            }
            WindowEvent::SurfaceResized(size) => {
                if size.width > 0 && size.height > 0 {
                    self.context.resize_surface(surface, size.width, size.height);
                    let logical = size.to_logical::<f64>(scale);
                    self.editor.resize(logical.width, logical.height);
                }
            }
            WindowEvent::ModifiersChanged(mods) => {
                let state = mods.state();
                self.mods =
                    Modifiers { shift: state.shift_key(), ctrl: state.control_key(), alt: state.alt_key() };
            }
            WindowEvent::PointerMoved { position, source, .. } => {
                let p = position.to_logical::<f64>(scale);
                self.cursor = Point::new(p.x, p.y);
                self.editor.pointer_move_with_pressure(self.cursor, self.mods, tablet_pressure(&source));
            }
            WindowEvent::PointerButton { state, position, button, .. } => {
                let p = position.to_logical::<f64>(scale);
                self.cursor = Point::new(p.x, p.y);
                let pressure = button_pressure(&button);
                let Some(button) = button.mouse_button().and_then(map_button) else {
                    return;
                };
                if state == ElementState::Released {
                    self.editor.pointer_up(self.cursor, button, self.mods);
                } else if button == Button::Primary && self.is_double_click() {
                    self.last_press = None;
                    self.editor.double_click(self.cursor);
                } else {
                    if button == Button::Primary {
                        self.last_press = Some((Instant::now(), self.cursor));
                    }
                    self.editor.pointer_down_with_pressure(self.cursor, button, self.mods, pressure);
                }
            }
            WindowEvent::MouseWheel { delta, .. } => {
                let delta = match delta {
                    MouseScrollDelta::LineDelta(x, y) => Vec2::new(f64::from(x), f64::from(y)) * WHEEL_LINE,
                    MouseScrollDelta::PixelDelta(d) => Vec2::new(d.x, d.y) / scale,
                    _ => Vec2::ZERO,
                };
                self.editor.wheel(delta, self.cursor, self.mods);
            }
            WindowEvent::PinchGesture { delta, .. } => {
                self.editor.zoom_at(self.cursor, 1.0 + delta);
            }
            WindowEvent::KeyboardInput { event, .. } => {
                let key = map_key(&event.logical_key);
                let pressed = event.state == ElementState::Pressed;
                let shortcut = pressed && self.mods.ctrl;
                if shortcut && key.is_some_and(|key| self.shortcut(key, self.mods.shift)) {
                    // Handled by the application.
                } else if let Some(key) = key {
                    if pressed {
                        self.editor.key_down(key, self.mods);
                    } else {
                        self.editor.key_up(key);
                    }
                }
                // While a text is being edited, what the key types goes into it.
                if let Some(text) = event.text.filter(|_| pressed && !self.mods.ctrl && !self.mods.alt) {
                    self.editor.text_input(&text);
                }
            }
            WindowEvent::Ime(Ime::Commit(text)) => self.editor.text_input(&text),
            _ => return,
        }
        if self.editor.take_request() == Some(Request::PickImage) {
            self.pick_image();
        }
        self.update_title();
        if let Some((_, window)) = &self.surface {
            let icon = match self.editor.cursor() {
                Cursor::Default => CursorIcon::Default,
                Cursor::Crosshair => CursorIcon::Crosshair,
                Cursor::Text => CursorIcon::Text,
                Cursor::Grab => CursorIcon::Grab,
            };
            window.set_cursor(icon.into());
            window.request_redraw();
        }
    }
}

/// Where a new drawing is saved: a name in the current directory that does
/// not clobber an existing file.
fn untitled_path() -> PathBuf {
    (0u32..)
        .map(|n| match n {
            0 => PathBuf::from("untitled.excalidraw"),
            n => PathBuf::from(format!("untitled-{n}.excalidraw")),
        })
        .find(|path| !path.exists())
        .expect("some untitled name is free")
}

struct Args {
    file: Option<PathBuf>,
    svg: Option<PathBuf>,
    png: Option<PathBuf>,
    scale: f64,
}

fn parse_args() -> Result<Args, String> {
    let mut args = Args { file: None, svg: None, png: None, scale: 1.0 };
    let mut it = std::env::args().skip(1);
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--svg" => args.svg = Some(it.next().ok_or("--svg needs an output path")?.into()),
            "--png" => args.png = Some(it.next().ok_or("--png needs an output path")?.into()),
            "--scale" => {
                let value = it.next().ok_or("--scale needs a number")?;
                args.scale = value.parse().ok().filter(|s| *s > 0.0).ok_or("--scale must be a positive number")?;
            }
            "-h" | "--help" => return Err(String::new()),
            flag if flag.starts_with('-') => return Err(format!("unknown option {flag}")),
            _ if args.file.is_none() => args.file = Some(arg.into()),
            _ => return Err("only one file can be opened".to_owned()),
        }
    }
    Ok(args)
}

fn main() {
    let args = parse_args().unwrap_or_else(|err| {
        if !err.is_empty() {
            eprintln!("error: {err}");
        }
        eprintln!("usage: board-app [FILE.excalidraw] [--svg OUT.svg] [--png OUT.png [--scale N]]");
        std::process::exit(2);
    });

    let mut document = None;
    if let Some(path) = args.file.as_ref().filter(|p| p.exists()) {
        match load_document(path) {
            Ok(doc) => document = Some(doc),
            Err(err) => {
                eprintln!("could not open {}: {err}", path.display());
                std::process::exit(1);
            }
        }
    }

    // Headless export: no window is opened.
    if args.svg.is_some() || args.png.is_some() {
        let Some(doc) = document else {
            eprintln!("error: exporting needs an existing input file");
            std::process::exit(2);
        };
        let mut failed = false;
        let mut report = |out: &Path, result: Result<(), Box<dyn Error>>| {
            if let Err(err) = result {
                eprintln!("could not export {}: {err}", out.display());
                failed = true;
            }
        };
        if let Some(out) = &args.svg {
            report(out, std::fs::write(out, export::to_svg(&doc)).map_err(Into::into));
        }
        if let Some(out) = &args.png {
            let (frame, width, height) = export::to_frame(&doc, args.scale);
            report(out, render::write_png(&frame, width, height, &doc.files, out));
        }
        std::process::exit(i32::from(failed));
    }

    let seed = SystemTime::now().duration_since(UNIX_EPOCH).map_or(1, |d| d.as_nanos() as u64);
    let mut editor = Editor::new(seed);
    if let Some(doc) = document {
        editor.load(doc);
    }
    let app = App {
        context: RenderContext::new(),
        renderers: Vec::new(),
        surface: None,
        scene: Scene::new(),
        editor,
        path: args.file.unwrap_or_else(untitled_path),
        mods: Modifiers::default(),
        cursor: Point::ZERO,
        last_press: None,
        clipboard: None,
        images: Images::default(),
    };
    let event_loop = EventLoop::new().expect("failed to start the event loop");
    event_loop.run_app(app).expect("event loop failed");
}
