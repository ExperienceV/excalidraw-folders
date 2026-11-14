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
├── crates/core   # scene model, .excalidraw (de)serialization, geometry. No platform code.
└── crates/app    # window + rendering (winit + vello/wgpu)
```

`core` must stay platform-independent so it compiles unchanged to Windows and
`wasm32`; only `app` is a platform shell.

## Roadmap

1. Scene model and `.excalidraw` read/write in `core`.
2. Window that draws rectangle, ellipse and diamond, with pan and zoom.
3. Hand-drawn ("rough") style: seeded deterministic strokes and hachure fill.
4. Selection and transform handles, undo/redo.
5. Text (editing, IME), bound arrows, SVG/PNG export.
6. Later: Windows, WASM build, real-time collaboration.

## Build

```sh
cd native
cargo run -p pizarra-app
```

Requires a Rust toolchain and a Vulkan-capable GPU on Linux.

## Licensing

Excalidraw is MIT-licensed. If code, fonts or algorithms are ported from it
(e.g. the Excalifont/Virgil fonts, rough.js-style rendering), keep the notices
in `THIRD_PARTY_NOTICES.md` up to date.
