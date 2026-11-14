# Native engine (`native-engine` branch)

This branch is **not** the Boards app on `main`. It is an experiment to build a
drawing engine very similar to Excalidraw that **does not depend on Chromium**.

## How it differs from `main`

| | `main` (Boards) | `native-engine` |
|---|---|---|
| Drawing engine | Excalidraw (React/TypeScript) | New engine written in Rust, in `native/` |
| Shell | Electron (bundles Chromium) | Native window (`winit`), no webview |
| Rendering | Canvas 2D in Chromium | `vello` on `wgpu` (Vulkan/Metal/DX12/WebGPU) |
| File format | `.excalidraw` | `.excalidraw` (kept compatible) |
| Platforms | Linux (Electron) | Linux first, then Windows, possibly Web via WASM |
| Code | `src/` | `native/` only; `src/` is untouched |

The Electron app stays as-is on `main`. Nothing in `native/` is imported by it.

## Layout

```
native/
├── crates/core   # everything that is not platform-specific
│   ├── element.rs   # scene model, .excalidraw (de)serialization
│   ├── rough.rs     # hand-drawn stroke and hachure generation (seeded)
│   ├── shape.rs     # element → fill/stroke paths
│   ├── text.rs      # fonts, measuring, wrapping, glyph outlines
│   ├── bound.rs     # labels inside shapes (bound text geometry)
│   ├── binding.rs   # arrows attached to shapes
│   ├── hit.rs       # hit-testing
│   ├── editor.rs    # tools, selection, text editing, undo/redo, display list
│   ├── ui.rs        # toolbar and style panel
│   ├── export.rs    # SVG export, display list for raster export
│   └── fonts/       # Excalidraw's WOFF2 fonts, embedded in the binary
├── crates/app    # winit window + vello/wgpu renderer, PNG export
└── samples/      # drawings to try the engine with
```

`core` never touches a window, a GPU or the clock. It receives input events
and returns a display list (`Frame`) of plain fill/stroke paths; `app` paints
it. That keeps `core` compiling unchanged for Windows and `wasm32`, where only
a different shell is needed.

## Status

Behaviour is ported from Excalidraw's source where it matters for
compatibility (text metrics and wrapping, label placement, arrow bindings,
corner radii).

Working:

- Rectangle, diamond, ellipse, line, arrow (with arrowheads) and freedraw.
  Lines and arrows can have several points: click to place them one by one,
  or drag the midpoint of a segment to add one.
- Images (PNG, JPEG, GIF, WebP, BMP) with crop and flips: shown, inserted
  from a file dialog or pasted from the clipboard, exported.
- Sketchy rendering: roughness 0–2, solid/dashed/dotted strokes, rounded
  corners, and solid, hachure, cross-hatch and zigzag fills.
- Text in Excalidraw's fonts (Excalifont, Virgil, Nunito, Comic Shanns,
  Cascadia, Liberation Sans, Lilita One): typing, caret movement, multi-line,
  alignment, and labels inside shapes that wrap, stay centered and follow
  the shape when it moves or is resized.
- Style panel: stroke and background color (quick picks plus Excalidraw's
  full palette), fill, stroke width and style, sloppiness, edges, font, font
  size, alignment, opacity, layer order.
- Opens and saves `.excalidraw`; unknown fields and element types survive a
  round trip.
- Arrows attach to the shapes under their ends, stop at the outline and
  follow the shape when it moves, resizes or rotates (Excalidraw's
  fixed-point bindings; older `focus`/`gap` bindings are upgraded on load).
- Select (click, shift-click, marquee), move, resize and rotate with
  handles, drag the points of lines and arrows, duplicate, delete, nudge,
  group/ungroup, undo/redo.
- Copy, cut and paste through the system clipboard in Excalidraw's format
  (elements can be exchanged with Excalidraw); plain text pastes as text.
- Pan and zoom, toolbar, Excalidraw's tool shortcuts.
- Open and save-as dialogs (through the desktop portal).
- Headless export to SVG and PNG.

Not yet:

- SVG images are not rendered (a placeholder box is shown); animated GIFs
  show their first frame.
- Text: no selection ranges, kerning or complex shaping; emoji and CJK have
  no glyphs. Arrow labels are drawn but not created or moved with their
  arrow.
- Elbow arrows; removing a point from a line.
- Rotating or resizing several elements at once.
- Colors by hex code, canvas background color, arrowhead choice, frames,
  pressure-sensitive freedraw outlines.
- Errors (a file that cannot be opened or saved) are only printed to the
  terminal; there is no prompt before closing with unsaved changes.
- Windows and WASM shells, real-time collaboration.

Strokes follow rough.js's algorithms but are not pixel-identical to
Excalidraw's for the same seed. Text is measured without kerning, so line
widths can differ slightly from Excalidraw's.

## Build and run

Requires a Rust toolchain and a Vulkan-capable GPU on Linux.

```sh
cd native
cargo run -p pizarra-app --release -- samples/demo.excalidraw
cargo test                       # core has unit tests; no GPU needed
# Scripted editor session rendered to a PNG, UI included (needs a GPU):
cargo run -p pizarra-app --example ui_snapshot -- /tmp/ui.png
```

```
pizarra-app [FILE.excalidraw] [--svg OUT.svg] [--png OUT.png [--scale N]]
```

Without a file it starts empty and saves to `untitled.excalidraw` in the
current directory (`untitled-1.excalidraw`, ... if that name is taken).
`--svg`/`--png` export the file and exit without opening a window.

| Keys | Action |
| --- | --- |
| `H` · `V`/`1` · `R`/`2` · `D`/`3` · `O`/`4` · `A`/`5` · `L`/`6` · `P`/`7` · `T`/`8` · `9` | Hand, select, rectangle, diamond, ellipse, arrow, line, freedraw, text, insert image |
| Click (instead of drag) with line or arrow | Place points one by one; `Enter`, `Esc`, double click or a click on the last point ends |
| Double click · `Enter` on a selection | New text, or edit the text / label under the pointer · edit it |
| While typing: `Esc` or `Ctrl+Enter` · arrows, `Home`, `End` | Finish · move the caret |
| `Shift` while drawing / resizing / rotating | Square or 15° steps / keep aspect ratio / 15° steps |
| Wheel · `Shift`+wheel · `Ctrl`+wheel or pinch | Scroll · scroll sideways · zoom |
| `Space`+drag or middle button | Pan |
| `Ctrl+Z` · `Ctrl+Shift+Z`/`Ctrl+Y` | Undo · redo |
| `Ctrl+A` · `Ctrl+D` · `Delete` · arrows | Select all · duplicate · delete · nudge |
| `Ctrl+C` · `Ctrl+X` · `Ctrl+V` | Copy · cut · paste (at the pointer) |
| `Ctrl+G` · `Ctrl+Shift+G` | Group · ungroup |
| `Ctrl+0` · `Ctrl++` · `Ctrl+-` | Reset zoom · zoom in · zoom out |
| `Ctrl+S` · `Ctrl+Shift+S` · `Ctrl+O` | Save · save as · open (unsaved changes are saved first) |
| `Esc` | Deselect, back to the select tool |

## Licensing

The sketchy stroke generation in `native/crates/core/src/rough.rs` is adapted
from rough.js (MIT), and text measuring, wrapping, label placement and arrow
bindings are ported from Excalidraw (MIT). Unlike `main`, which copies fonts from the npm
package at build time, this branch stores Excalidraw's font files in
`native/crates/core/fonts/` so the engine builds without Node. All of this is
credited in `THIRD_PARTY_NOTICES.md`; keep it up to date when porting more.
