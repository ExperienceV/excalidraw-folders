import { app, BrowserWindow, dialog, ipcMain, Menu, nativeTheme, net, protocol, session, shell } from "electron"
import { readFile, writeFile } from "node:fs/promises"
import { basename, extname, join, normalize, sep } from "node:path"
import { pathToFileURL } from "node:url"
import { WorkspaceStore } from "./store"
import icon from "../../resources/icon.png?asset"

const store = new WorkspaceStore(join(app.getPath("userData"), "workspace"))

const FLUSH_TIMEOUT_MS = 3000
const RENDERER_DIR = join(__dirname, "../renderer")
const APP_ORIGIN = "app://boards"

// The UI is served from app:// instead of file:// so fetch() works (Excalidraw fetches
// its bundled fonts) and the page gets a proper origin for its CSP.
protocol.registerSchemesAsPrivileged([
  { scheme: "app", privileges: { standard: true, secure: true, supportFetchAPI: true } },
])

function serveRenderer() {
  protocol.handle("app", (request) => {
    const { pathname } = new URL(request.url)
    const filePath = normalize(join(RENDERER_DIR, decodeURIComponent(pathname)))
    if (!filePath.startsWith(RENDERER_DIR + sep)) {
      return new Response("Forbidden", { status: 403 })
    }
    return net.fetch(pathToFileURL(filePath).toString())
  })

  // Fonts ship with the app; never reach for Excalidraw's CDN fallback
  session.defaultSession.webRequest.onBeforeRequest({ urls: ["https://esm.sh/*"] }, (_details, callback) =>
    callback({ cancel: true }),
  )
}

function createWindow() {
  const window = new BrowserWindow({
    width: 1360,
    height: 860,
    minWidth: 760,
    minHeight: 520,
    title: "Boards",
    icon,
    show: false,
    autoHideMenuBar: true,
    backgroundColor: nativeTheme.shouldUseDarkColors ? "#121212" : "#f6f5f1",
    webPreferences: {
      preload: join(__dirname, "../preload/index.js"),
      contextIsolation: true,
      sandbox: true,
      nodeIntegration: false,
      spellcheck: false,
    },
  })

  window.once("ready-to-show", () => window.show())

  // Links inside boards open in the system browser, never inside the app
  window.webContents.setWindowOpenHandler(({ url }) => {
    if (/^https?:\/\//.test(url)) void shell.openExternal(url)
    return { action: "deny" }
  })
  window.webContents.on("will-navigate", (event, url) => {
    if (url !== window.webContents.getURL()) {
      event.preventDefault()
      if (/^https?:\/\//.test(url)) void shell.openExternal(url)
    }
  })

  // Give the renderer a chance to save the open board before closing
  let flushed = false
  window.on("close", (event) => {
    if (flushed) return
    event.preventDefault()
    const finish = () => {
      if (flushed) return
      flushed = true
      ipcMain.removeListener("app:flushed", onFlushed)
      window.close()
    }
    const onFlushed = (ipcEvent: Electron.IpcMainEvent) => {
      if (ipcEvent.sender === window.webContents) finish()
    }
    ipcMain.on("app:flushed", onFlushed)
    window.webContents.send("app:flush")
    setTimeout(finish, FLUSH_TIMEOUT_MS)
  })

  if (process.env.ELECTRON_RENDERER_URL) {
    void window.loadURL(process.env.ELECTRON_RENDERER_URL)
  } else {
    void window.loadURL(`${APP_ORIGIN}/index.html`)
  }
}

function registerIpc() {
  ipcMain.handle("workspace:get", () => store.getWorkspace())
  ipcMain.handle("workspace:open-folder", async () => {
    await store.getWorkspace()
    await shell.openPath(store.root)
  })

  ipcMain.handle("project:create", (_e, name: string) => store.createProject(String(name)))
  ipcMain.handle("project:update", (_e, id: string, changes: { name?: string; color?: string }) =>
    store.updateProject(String(id), { name: changes?.name, color: changes?.color }),
  )
  ipcMain.handle("project:delete", (_e, id: string) => store.deleteProject(String(id)))

  ipcMain.handle("board:create", (_e, projectId: string, name: string) => store.createBoard(String(projectId), String(name)))
  ipcMain.handle("board:rename", (_e, id: string, name: string) => store.renameBoard(String(id), String(name)))
  ipcMain.handle("board:duplicate", (_e, id: string) => store.duplicateBoard(String(id)))
  ipcMain.handle("board:move", (_e, id: string, projectId: string) => store.moveBoard(String(id), String(projectId)))
  ipcMain.handle("board:delete", (_e, id: string) => store.deleteBoard(String(id)))
  ipcMain.handle("board:load", (_e, id: string) => store.loadBoard(String(id)))
  ipcMain.handle("board:save", (_e, id: string, scene: string, thumbnail?: string) =>
    store.saveBoard(String(id), String(scene), typeof thumbnail === "string" ? thumbnail : undefined),
  )
  ipcMain.handle("board:thumbnail", (_e, id: string) => store.getThumbnail(String(id)))

  ipcMain.handle("board:export", async (event, id: string) => {
    const workspace = await store.getWorkspace()
    const board = workspace.projects.flatMap((p) => p.boards).find((b) => b.id === id)
    if (!board) throw new Error(`Board not found: ${id}`)
    const owner = BrowserWindow.fromWebContents(event.sender) ?? undefined
    const options = {
      title: "Exportar pizarra",
      defaultPath: `${board.name.replace(/[\\/:*?"<>|]/g, "_")}.excalidraw`,
      filters: [{ name: "Excalidraw", extensions: ["excalidraw"] }],
    }
    const result = owner ? await dialog.showSaveDialog(owner, options) : await dialog.showSaveDialog(options)
    if (result.canceled || !result.filePath) return false
    const scene = await store.loadBoard(board.id)
    await writeFile(result.filePath, scene ?? JSON.stringify({ type: "excalidraw", version: 2, elements: [], appState: {}, files: {} }))
    return true
  })

  ipcMain.handle("board:import", async (event, projectId: string) => {
    const owner = BrowserWindow.fromWebContents(event.sender) ?? undefined
    const options: Electron.OpenDialogOptions = {
      title: "Importar pizarras",
      properties: ["openFile", "multiSelections"],
      filters: [{ name: "Excalidraw", extensions: ["excalidraw", "json"] }],
    }
    const result = owner ? await dialog.showOpenDialog(owner, options) : await dialog.showOpenDialog(options)
    if (result.canceled) return []
    const imported = []
    for (const path of result.filePaths) {
      const scene = await readFile(path, "utf8")
      imported.push(await store.importBoard(String(projectId), basename(path, extname(path)), scene))
    }
    return imported
  })

  ipcMain.handle("library:load", () => store.loadLibrary())
  ipcMain.handle("library:save", (_e, library: string) => store.saveLibrary(String(library)))
}

app.setName("Boards")

// Escape hatch for Wayland compositors whose tablet protocol support doesn't reach
// Chromium: run under XWayland, where pen pressure goes through XInput2 instead.
if (process.env.BOARDS_FORCE_X11 === "1") {
  app.commandLine.appendSwitch("ozone-platform", "x11")
}

// Only one instance should write to the workspace at a time
if (!app.requestSingleInstanceLock()) {
  app.quit()
} else {
  app.on("second-instance", () => {
    const [window] = BrowserWindow.getAllWindows()
    if (window) {
      if (window.isMinimized()) window.restore()
      window.focus()
    }
  })

  void app.whenReady().then(() => {
    Menu.setApplicationMenu(null)
    serveRenderer()
    registerIpc()
    createWindow()

    app.on("activate", () => {
      if (BrowserWindow.getAllWindows().length === 0) createWindow()
    })
  })

  app.on("window-all-closed", () => {
    if (process.platform !== "darwin") app.quit()
  })
}
