//! Drives the editor with scripted input and saves what the window would
//! show, chrome included, as a PNG. Handy for checking UI changes without
//! opening a window:
//!
//! ```sh
//! cargo run -p pizarra-app --example ui_snapshot -- out.png
//! ```

use pizarra_core::kurbo::Point;
use pizarra_core::{Button, Editor, Key, Modifiers};

#[path = "../src/render.rs"]
mod render;

const SIZE: (u32, u32) = (1100, 760);

fn main() {
    let out = std::env::args().nth(1).unwrap_or_else(|| "ui_snapshot.png".to_owned());
    let none = Modifiers::default();
    let mut ed = Editor::new(7);
    ed.resize(f64::from(SIZE.0), f64::from(SIZE.1));
    let drag = |ed: &mut Editor, tool: char, from: (f64, f64), to: (f64, f64)| {
        ed.key_down(Key::Char(tool), none);
        ed.pointer_down(Point::from(from), Button::Primary, none);
        ed.pointer_move(Point::from(to), none);
        ed.pointer_up(Point::from(to), Button::Primary, none);
    };

    drag(&mut ed, 'o', (330.0, 150.0), (560.0, 290.0));
    ed.double_click(Point::new(445.0, 150.0));
    ed.text_input("Una elipse con su etiqueta");
    ed.key_down(Key::Escape, none);

    drag(&mut ed, 'd', (700.0, 140.0), (900.0, 300.0));
    drag(&mut ed, 'a', (570.0, 220.0), (690.0, 220.0));

    ed.key_down(Key::Char('t'), none);
    ed.pointer_down(Point::new(330.0, 400.0), Button::Primary, none);
    ed.text_input("Texto libre\nen dos líneas");
    ed.key_down(Key::Escape, none);

    drag(&mut ed, 'r', (640.0, 400.0), (900.0, 560.0));
    ed.double_click(Point::new(770.0, 400.0));
    ed.text_input("Editando: el cursor va al final");

    let frame = ed.frame();
    if let Err(err) = render::write_png(&frame, SIZE.0, SIZE.1, &ed.document().files, out.as_ref()) {
        eprintln!("could not write {out}: {err}");
        std::process::exit(1);
    }
}
