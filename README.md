# Boards

**English** · [Español](README.es.md)

A desktop app to organize whiteboards into projects, built on the
[Excalidraw](https://github.com/excalidraw/excalidraw) drawing engine and Electron.
Everything stays on your machine as standard `.excalidraw` files.

![Project manager](docs/screenshots/manager-light.png)

![Board editor](docs/screenshots/editor-light.png)

<details>
<summary>Dark theme</summary>

![Project manager, dark](docs/screenshots/manager-dark.png)
![Board editor, dark](docs/screenshots/editor-dark.png)

</details>

## Features

- **Projects and boards**: colored projects, board thumbnails, search and a "recent" view.
- Rename, duplicate, move between projects, delete, and import/export `.excalidraw` files.
- **Autosave** to local files, plus a save when you go back to the list or close the window.
- **Works offline**: Excalidraw's fonts ship with the app instead of loading from a CDN.
- **Pen pressure**: Chromium passes the tablet's real `PointerEvent.pressure` to Excalidraw,
  so freehand strokes vary in width. A live pressure meter appears in the editor's top bar
  as soon as you use a pen.
- A trimmed-down Excalidraw menu: no collaboration, no "open file", no "save as".
- Light, dark or system theme.

The interface is in Spanish for now. Translations are a welcome contribution.

## Project status

This is a working base meant to be forked and extended. It is built and tested on **Linux**
(Arch, Wayland/niri). Electron runs on Windows and macOS too, but the build config and
testing currently cover Linux only.

## Getting started

Requires Node ≥ 20.19 and [pnpm](https://pnpm.io) (`corepack enable`).

```bash
pnpm install
pnpm dev            # run with hot reload
pnpm typecheck
```

### Building

```bash
pnpm dist           # AppImage + pacman package in dist/
pnpm dist:dir       # unpacked build only (faster)
pnpm install:local  # build and install for the current user (Linux, no root)
```

`install:local` copies the app to `~/.local/opt/boards` and creates the `boards` command
(`~/.local/bin`), a `boards.desktop` entry and icons under `~/.local/share`. The window
identifies itself as `boards` (Wayland app_id / X11 WM_CLASS), so taskbars and launchers
match it to its icon. Run it again after making changes.

## Where your data lives

Everything is stored in `~/.config/Boards/workspace/` (the "Carpeta de datos" button opens it):

```
workspace.json             project and board index
boards/<id>.excalidraw     scenes, in the standard Excalidraw format
thumbnails/<id>.png        board previews
library.excalidrawlib      shape library
```

Board files open as-is on [excalidraw.com](https://excalidraw.com).

## Pen pressure on Wayland

Electron runs natively on Wayland, where pen pressure depends on the compositor's tablet
protocol support. If the pressure meter doesn't appear or stays still, run the app under
XWayland, where pressure goes through XInput2:

```bash
BOARDS_FORCE_X11=1 boards   # installed app
pnpm dev:x11                # development
```

## Customizing

The app's own code is small (~1,700 lines of TypeScript). Good places to start:

| You want to… | Look at |
| --- | --- |
| Show or hide Excalidraw features and menu items | `UIOptions` and `<MainMenu>` in `src/renderer/src/components/BoardEditor.tsx` |
| Change autosave or thumbnail behavior | `save` / `handleChange` in `BoardEditor.tsx` |
| Change how projects and boards are stored | `src/main/store.ts` |
| Add an operation the UI can call | `BoardsApi` in `src/shared/types.ts`, then `src/preload/index.ts` and `registerIpc()` in `src/main/index.ts` |
| Restyle the project manager | `src/renderer/src/styles.css` (CSS variables at the top) |

```
src/main/        main process: window, app:// protocol, IPC, storage
src/preload/     secure bridge (contextBridge) exposed as window.boards
src/renderer/    React: project manager and editor
src/shared/      shared types
resources/       app icon (icon.svg is the source)
scripts/         font copy, license generation, local install
```

The UI is served from a custom `app://` protocol, with a strict Content Security Policy
in production builds. The renderer runs sandboxed with context isolation, and links open
in the system browser.

To regenerate the icon PNGs after editing `resources/icon.svg`:

```bash
for n in 16 24 32 48 64 128 256 512; do rsvg-convert -w $n -h $n resources/icon.svg -o resources/icons/${n}x${n}.png; done
cp resources/icons/512x512.png resources/icon.png
```

## License

[MIT](LICENSE). Boards is an independent project and is not affiliated with Excalidraw.
Third-party code and fonts are credited in [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md);
full license texts for every bundled library are in
[THIRD_PARTY_LICENSES.txt](THIRD_PARTY_LICENSES.txt), regenerated on each build.
