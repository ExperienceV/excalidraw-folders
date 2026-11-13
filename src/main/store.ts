import { randomUUID } from "node:crypto"
import { mkdir, readFile, rename, rm, writeFile, copyFile } from "node:fs/promises"
import { join } from "node:path"
import type { Board, BoardLocation, Project, Workspace } from "../shared/types"

const PROJECT_COLORS = ["#c4ed62", "#7dd3fc", "#fca5a5", "#fcd34d", "#c4b5fd", "#86efac", "#f9a8d4"]

const MAX_NAME_LENGTH = 120

function cleanName(name: string, fallback: string) {
  const trimmed = name.trim().slice(0, MAX_NAME_LENGTH)
  return trimmed || fallback
}

async function writeFileAtomic(path: string, data: string | Buffer) {
  const temp = `${path}.${process.pid}.${randomUUID()}.tmp`
  await writeFile(temp, data)
  await rename(temp, path)
}

/**
 * Persists projects and boards under a workspace directory:
 *   workspace.json             project/board index
 *   boards/<id>.excalidraw     scene files (standard Excalidraw format)
 *   thumbnails/<id>.png        previews for the project manager
 */
export class WorkspaceStore {
  private workspace: Workspace | null = null
  // Serializes mutations so concurrent IPC calls never interleave index writes
  private queue: Promise<unknown> = Promise.resolve()

  constructor(readonly root: string) {}

  private get indexPath() {
    return join(this.root, "workspace.json")
  }

  private boardPath(boardId: string) {
    return join(this.root, "boards", `${boardId}.excalidraw`)
  }

  private thumbnailPath(boardId: string) {
    return join(this.root, "thumbnails", `${boardId}.png`)
  }

  private async load(): Promise<Workspace> {
    if (this.workspace) return this.workspace

    await mkdir(join(this.root, "boards"), { recursive: true })
    await mkdir(join(this.root, "thumbnails"), { recursive: true })

    try {
      const parsed = JSON.parse(await readFile(this.indexPath, "utf8")) as Workspace
      this.workspace = { version: 1, projects: Array.isArray(parsed.projects) ? parsed.projects : [] }
    } catch (error) {
      if ((error as NodeJS.ErrnoException).code !== "ENOENT") {
        // Keep the unreadable index around instead of silently overwriting it
        await copyFile(this.indexPath, `${this.indexPath}.corrupt-${Date.now()}`).catch(() => {})
      }
      this.workspace = { version: 1, projects: [] }
    }
    return this.workspace
  }

  private async persist() {
    await writeFileAtomic(this.indexPath, JSON.stringify(this.workspace, null, 2))
  }

  private mutate<T>(fn: (workspace: Workspace) => Promise<T> | T): Promise<T> {
    const run = this.queue.then(async () => {
      const workspace = await this.load()
      const result = await fn(workspace)
      await this.persist()
      return result
    })
    this.queue = run.catch(() => {})
    return run
  }

  private locate(workspace: Workspace, boardId: string): BoardLocation {
    for (const project of workspace.projects) {
      const board = project.boards.find((b) => b.id === boardId)
      if (board) return { project, board }
    }
    throw new Error(`Board not found: ${boardId}`)
  }

  private findProject(workspace: Workspace, projectId: string): Project {
    const project = workspace.projects.find((p) => p.id === projectId)
    if (!project) throw new Error(`Project not found: ${projectId}`)
    return project
  }

  async getWorkspace(): Promise<Workspace> {
    await this.queue
    return structuredClone(await this.load())
  }

  createProject(name: string) {
    return this.mutate((workspace) => {
      const now = Date.now()
      const project: Project = {
        id: randomUUID(),
        name: cleanName(name, "Proyecto sin título"),
        color: PROJECT_COLORS[workspace.projects.length % PROJECT_COLORS.length],
        createdAt: now,
        updatedAt: now,
        boards: [],
      }
      workspace.projects.push(project)
      return structuredClone(project)
    })
  }

  updateProject(projectId: string, changes: { name?: string; color?: string }) {
    return this.mutate((workspace) => {
      const project = this.findProject(workspace, projectId)
      if (changes.name !== undefined) project.name = cleanName(changes.name, project.name)
      if (changes.color !== undefined && /^#[0-9a-f]{6}$/i.test(changes.color)) project.color = changes.color
      project.updatedAt = Date.now()
      return structuredClone(project)
    })
  }

  deleteProject(projectId: string) {
    return this.mutate(async (workspace) => {
      const project = this.findProject(workspace, projectId)
      workspace.projects = workspace.projects.filter((p) => p.id !== projectId)
      await Promise.all(project.boards.map((board) => this.removeBoardFiles(board.id)))
    })
  }

  createBoard(projectId: string, name: string) {
    return this.mutate((workspace) => {
      const project = this.findProject(workspace, projectId)
      const now = Date.now()
      const board: Board = { id: randomUUID(), name: cleanName(name, "Pizarra sin título"), createdAt: now, updatedAt: now }
      project.boards.push(board)
      project.updatedAt = now
      return structuredClone(board)
    })
  }

  renameBoard(boardId: string, name: string) {
    return this.mutate((workspace) => {
      const { board } = this.locate(workspace, boardId)
      board.name = cleanName(name, board.name)
      board.updatedAt = Date.now()
      return structuredClone(board)
    })
  }

  duplicateBoard(boardId: string) {
    return this.mutate(async (workspace) => {
      const { project, board } = this.locate(workspace, boardId)
      const now = Date.now()
      const copy: Board = { id: randomUUID(), name: cleanName(`${board.name} (copia)`, board.name), createdAt: now, updatedAt: now }
      await copyFile(this.boardPath(board.id), this.boardPath(copy.id)).catch(() => {})
      await copyFile(this.thumbnailPath(board.id), this.thumbnailPath(copy.id)).catch(() => {})
      project.boards.splice(project.boards.indexOf(board) + 1, 0, copy)
      project.updatedAt = now
      return structuredClone(copy)
    })
  }

  moveBoard(boardId: string, targetProjectId: string) {
    return this.mutate((workspace) => {
      const { project, board } = this.locate(workspace, boardId)
      const target = this.findProject(workspace, targetProjectId)
      if (target === project) return
      project.boards = project.boards.filter((b) => b.id !== boardId)
      target.boards.push(board)
      project.updatedAt = target.updatedAt = Date.now()
    })
  }

  deleteBoard(boardId: string) {
    return this.mutate(async (workspace) => {
      const { project } = this.locate(workspace, boardId)
      project.boards = project.boards.filter((b) => b.id !== boardId)
      project.updatedAt = Date.now()
      await this.removeBoardFiles(boardId)
    })
  }

  private async removeBoardFiles(boardId: string) {
    await rm(this.boardPath(boardId), { force: true })
    await rm(this.thumbnailPath(boardId), { force: true })
  }

  async loadBoard(boardId: string): Promise<string | null> {
    this.locate(await this.load(), boardId)
    try {
      return await readFile(this.boardPath(boardId), "utf8")
    } catch (error) {
      if ((error as NodeJS.ErrnoException).code === "ENOENT") return null
      throw error
    }
  }

  async boardScenePath(boardId: string) {
    this.locate(await this.load(), boardId)
    return this.boardPath(boardId)
  }

  saveBoard(boardId: string, scene: string, thumbnail?: string) {
    return this.mutate(async (workspace) => {
      const { project, board } = this.locate(workspace, boardId)
      JSON.parse(scene) // reject malformed payloads before touching the file
      await writeFileAtomic(this.boardPath(boardId), scene)
      if (thumbnail === "") {
        await rm(this.thumbnailPath(boardId), { force: true })
      } else if (thumbnail?.startsWith("data:image/png;base64,")) {
        await writeFileAtomic(this.thumbnailPath(boardId), Buffer.from(thumbnail.split(",")[1], "base64"))
      }
      board.updatedAt = project.updatedAt = Date.now()
      return structuredClone(board)
    })
  }

  async getThumbnail(boardId: string): Promise<string | null> {
    try {
      const data = await readFile(this.thumbnailPath(boardId))
      return `data:image/png;base64,${data.toString("base64")}`
    } catch {
      return null
    }
  }

  importBoard(projectId: string, name: string, scene: string) {
    return this.mutate(async (workspace) => {
      const project = this.findProject(workspace, projectId)
      const parsed = JSON.parse(scene)
      if (parsed?.type !== "excalidraw" || !Array.isArray(parsed.elements)) {
        throw new Error(`"${name}" no es un archivo de Excalidraw válido`)
      }
      const now = Date.now()
      const board: Board = { id: randomUUID(), name: cleanName(name, "Pizarra importada"), createdAt: now, updatedAt: now }
      await writeFileAtomic(this.boardPath(board.id), scene)
      project.boards.push(board)
      project.updatedAt = now
      return structuredClone(board)
    })
  }

  async loadLibrary(): Promise<string | null> {
    try {
      return await readFile(join(this.root, "library.excalidrawlib"), "utf8")
    } catch {
      return null
    }
  }

  async saveLibrary(library: string) {
    JSON.parse(library)
    await mkdir(this.root, { recursive: true })
    await writeFileAtomic(join(this.root, "library.excalidrawlib"), library)
  }
}
