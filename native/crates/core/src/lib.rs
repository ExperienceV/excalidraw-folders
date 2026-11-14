//! Platform-independent drawing engine: scene model, `.excalidraw` I/O,
//! hand-drawn shape generation, hit-testing and editor logic.
//!
//! Nothing here touches a window, a GPU or the clock, so the crate compiles
//! unchanged for Linux, Windows and `wasm32`. A platform shell feeds input
//! into [`Editor`] and paints the [`Frame`] it returns.

mod binding;
mod bound;
pub mod draw;
pub mod editor;
pub mod element;
pub mod export;
mod font_data;
mod freehand;
pub mod hit;
pub mod rough;
pub mod shape;
pub mod text;
mod ui;

pub use draw::{Color, Crop, DrawOp, Frame, Item, Style};
pub use editor::{Button, Cursor, Editor, Key, Modifiers, Request, Tool};
pub use element::{Document, Element, Kind};
pub use kurbo;
