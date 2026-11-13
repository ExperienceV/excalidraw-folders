export interface Board {
  id: string
  name: string
  createdAt: number
  updatedAt: number
}

export interface Project {
  id: string
  name: string
  color: string
  createdAt: number
  updatedAt: number
  boards: Board[]
}

export interface Workspace {
  version: 1
  projects: Project[]
}

export interface BoardLocation {
  project: Project
  board: Board
}

/** API exposed by the preload script on `window.boards`. */
export interface BoardsApi {
  getWorkspace(): Promise<Workspace>
  createProject(name: string): Promise<Project>
  updateProject(projectId: string, changes: { name?: string; color?: string }): Promise<Project>
  deleteProject(projectId: string): Promise<void>

  createBoard(projectId: string, name: string): Promise<Board>
  renameBoard(boardId: string, name: string): Promise<Board>
  duplicateBoard(boardId: string): Promise<Board>
  moveBoard(boardId: string, targetProjectId: string): Promise<void>
  deleteBoard(boardId: string): Promise<void>

  /** Raw `.excalidraw` JSON, or null for a board that was never saved. */
  loadBoard(boardId: string): Promise<string | null>
  /** Saves the scene JSON and, optionally, a PNG thumbnail as a data URL. */
  saveBoard(boardId: string, scene: string, thumbnail?: string): Promise<Board>
  getThumbnail(boardId: string): Promise<string | null>

  /** Opens a save dialog; resolves false when the user cancels. */
  exportBoard(boardId: string): Promise<boolean>
  /** Opens a file picker and imports `.excalidraw` files into the project. */
  importBoards(projectId: string): Promise<Board[]>

  loadLibrary(): Promise<string | null>
  saveLibrary(library: string): Promise<void>

  openWorkspaceFolder(): Promise<void>

  /** Main asks the renderer to persist pending changes before the window closes. */
  onFlushRequest(callback: () => Promise<void>): () => void
}
