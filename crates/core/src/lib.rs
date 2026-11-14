//! Platform-independent drawing engine: scene model, `.excalidraw` I/O,
//! hand-drawn shape generation, hit-testing, editor logic, and project manager UI.
//!
//! Nothing here touches a window, a GPU or the clock, so the crate compiles
//! unchanged for Linux, Windows and `wasm32`. A platform shell feeds input
//! into [`Editor`] or [`manager_ui::ManagerState`] and paints the [`Frame`]
//! they return.

mod binding;
mod bound;
pub mod draw;
pub mod editor;
pub mod element;
pub mod export;
mod font_data;
mod freehand;
pub mod hit;
pub mod manager_ui;
pub mod rough;
pub mod shape;
pub mod text;
mod ui;
pub mod workspace;

pub use draw::{Color, Crop, DrawOp, Frame, Item, Style};
pub use editor::{Button, Cursor, Editor, Key, Modifiers, Request, Tool};
pub use element::{Document, Element, Kind};
pub use kurbo;
