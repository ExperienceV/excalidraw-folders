import { resolve } from "node:path"
import { defineConfig, externalizeDepsPlugin } from "electron-vite"
import react from "@vitejs/plugin-react"
import type { Plugin } from "vite"

// Production-only CSP: the dev server needs inline scripts and websockets for HMR
function contentSecurityPolicy(): Plugin {
  return {
    name: "boards-csp",
    apply: "build",
    transformIndexHtml(html) {
      const csp = [
        "default-src 'self'",
        "script-src 'self'",
        "style-src 'self' 'unsafe-inline'",
        "img-src 'self' data: blob:",
        // Excalidraw lists a CDN copy as fallback for every font; the main process cancels
        // those requests, so allowing it here only silences CSP noise
        "font-src 'self' data: https://esm.sh",
        "connect-src 'self' data: blob:",
        "worker-src 'self' blob:",
      ].join("; ")
      return html.replace("<head>", `<head>\n    <meta http-equiv="Content-Security-Policy" content="${csp}" />`)
    },
  }
}

export default defineConfig({
  main: {
    plugins: [externalizeDepsPlugin()],
  },
  preload: {
    plugins: [externalizeDepsPlugin()],
  },
  renderer: {
    root: resolve(__dirname, "src/renderer"),
    base: "./",
    plugins: [react(), contentSecurityPolicy()],
    define: {
      "process.env.IS_PREACT": JSON.stringify("false"),
    },
    build: {
      rollupOptions: {
        input: resolve(__dirname, "src/renderer/index.html"),
      },
    },
  },
})
