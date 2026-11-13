#!/bin/sh
# Installs the unpacked build for the current user (no root needed):
#   ~/.local/opt/boards            app files
#   ~/.local/bin/boards            launcher
#   ~/.local/share/applications    desktop entry (rofi, app launchers, taskbars)
#   ~/.local/share/icons/hicolor   icons
# Run `pnpm dist:dir` first, or use `pnpm install:local`, which does both.
set -eu

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
SOURCE="$ROOT/dist/linux-unpacked"
DATA_HOME="${XDG_DATA_HOME:-$HOME/.local/share}"
APP_DIR="$HOME/.local/opt/boards"
BIN="$HOME/.local/bin/boards"

if [ ! -x "$SOURCE/boards" ]; then
  echo "No build found at $SOURCE — run 'pnpm dist:dir' first." >&2
  exit 1
fi

rm -rf "$APP_DIR"
mkdir -p "$(dirname "$APP_DIR")" "$(dirname "$BIN")" "$DATA_HOME/applications"
cp -r "$SOURCE" "$APP_DIR"

cat > "$BIN" <<EOF
#!/bin/sh
exec "$APP_DIR/boards" "\$@"
EOF
chmod +x "$BIN"

for icon in "$ROOT"/resources/icons/*.png; do
  size="$(basename "$icon" .png)"
  mkdir -p "$DATA_HOME/icons/hicolor/$size/apps"
  cp "$icon" "$DATA_HOME/icons/hicolor/$size/apps/boards.png"
done
mkdir -p "$DATA_HOME/icons/hicolor/scalable/apps"
cp "$ROOT/resources/icon.svg" "$DATA_HOME/icons/hicolor/scalable/apps/boards.svg"

cat > "$DATA_HOME/applications/boards.desktop" <<EOF
[Desktop Entry]
Type=Application
Name=Boards
GenericName=Pizarras
Comment=Proyectos y pizarras con el motor de Excalidraw
Exec=$BIN %U
Icon=boards
Terminal=false
Categories=Graphics;2DGraphics;
Keywords=whiteboard;pizarra;dibujo;excalidraw;
StartupWMClass=boards
EOF

command -v update-desktop-database >/dev/null && update-desktop-database "$DATA_HOME/applications" || true
command -v gtk-update-icon-cache >/dev/null && gtk-update-icon-cache -q -t "$DATA_HOME/icons/hicolor" || true

echo "Boards installed: run 'boards' or launch it from your app menu."
