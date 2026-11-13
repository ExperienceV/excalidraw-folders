import { contextBridge, ipcRenderer } from "electron"
import type { BoardsApi } from "../shared/types"

const api: BoardsApi = {
  getWorkspace: () => ipcRenderer.invoke("workspace:get"),
  createProject: (name) => ipcRenderer.invoke("project:create", name),
  updateProject: (projectId, changes) => ipcRenderer.invoke("project:update", projectId, changes),
  deleteProject: (projectId) => ipcRenderer.invoke("project:delete", projectId),

  createBoard: (projectId, name) => ipcRenderer.invoke("board:create", projectId, name),
  renameBoard: (boardId, name) => ipcRenderer.invoke("board:rename", boardId, name),
  duplicateBoard: (boardId) => ipcRenderer.invoke("board:duplicate", boardId),
  moveBoard: (boardId, targetProjectId) => ipcRenderer.invoke("board:move", boardId, targetProjectId),
  deleteBoard: (boardId) => ipcRenderer.invoke("board:delete", boardId),

  loadBoard: (boardId) => ipcRenderer.invoke("board:load", boardId),
  saveBoard: (boardId, scene, thumbnail) => ipcRenderer.invoke("board:save", boardId, scene, thumbnail),
  getThumbnail: (boardId) => ipcRenderer.invoke("board:thumbnail", boardId),

  exportBoard: (boardId) => ipcRenderer.invoke("board:export", boardId),
  importBoards: (projectId) => ipcRenderer.invoke("board:import", projectId),

  loadLibrary: () => ipcRenderer.invoke("library:load"),
  saveLibrary: (library) => ipcRenderer.invoke("library:save", library),

  openWorkspaceFolder: () => ipcRenderer.invoke("workspace:open-folder"),

  onFlushRequest: (callback) => {
    const listener = () => {
      void callback()
        .catch((error) => console.error("Flush before close failed", error))
        .finally(() => ipcRenderer.send("app:flushed"))
    }
    ipcRenderer.on("app:flush", listener)
    return () => ipcRenderer.removeListener("app:flush", listener)
  },
}

contextBridge.exposeInMainWorld("boards", api)
