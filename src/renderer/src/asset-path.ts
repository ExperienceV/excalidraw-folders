// Must run before Excalidraw is imported: serve its fonts from the app bundle
// (copied by scripts/copy-fonts.mjs) instead of a CDN, so the app works offline.
window.EXCALIDRAW_ASSET_PATH = new URL("./", window.location.href).href
