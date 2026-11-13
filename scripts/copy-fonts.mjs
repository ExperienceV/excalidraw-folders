// Copies Excalidraw's font files into the renderer's public dir so the app works offline.
import { cpSync, existsSync } from "node:fs"
import { dirname, join } from "node:path"
import { fileURLToPath } from "node:url"

const root = join(dirname(fileURLToPath(import.meta.url)), "..")
const source = join(root, "node_modules/@excalidraw/excalidraw/dist/prod/fonts")
const target = join(root, "src/renderer/public/fonts")

if (!existsSync(source)) {
  console.error(`Excalidraw fonts not found at ${source}`)
  process.exit(1)
}
cpSync(source, target, { recursive: true })
console.log("Copied Excalidraw fonts to src/renderer/public/fonts")
