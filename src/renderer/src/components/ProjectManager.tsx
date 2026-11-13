import { useEffect, useMemo, useState } from "react"
import {
  ArrowRightLeft,
  Clock,
  Copy,
  Download,
  FolderOpen,
  Monitor,
  Moon,
  Palette,
  Pencil,
  Plus,
  Search,
  Sun,
  Trash2,
  Upload,
} from "lucide-react"
import type { Board, Project, Workspace } from "../../../shared/types"
import { pluralize, timeAgo } from "../format"
import type { ThemePreference } from "../theme"
import { useDialogs } from "./Dialogs"
import { Menu } from "./Menu"

export type Selection = { type: "recent" } | { type: "project"; id: string }

const PROJECT_COLORS = ["#c4ed62", "#7dd3fc", "#fca5a5", "#fcd34d", "#c4b5fd", "#86efac", "#f9a8d4"]
const RECENT_LIMIT = 24

interface ProjectManagerProps {
  workspace: Workspace
  selection: Selection
  onSelect(selection: Selection): void
  onOpenBoard(boardId: string): void
  onChanged(): Promise<void>
  themePreference: ThemePreference
  onThemePreferenceChange(preference: ThemePreference): void
}

type BoardEntry = { board: Board; project: Project }

export function ProjectManager({
  workspace,
  selection,
  onSelect,
  onOpenBoard,
  onChanged,
  themePreference,
  onThemePreferenceChange,
}: ProjectManagerProps) {
  const dialogs = useDialogs()
  const [query, setQuery] = useState("")
  const [notice, setNotice] = useState<string | null>(null)

  const project =
    selection.type === "project" ? workspace.projects.find((p) => p.id === selection.id) ?? null : null

  // A deleted project falls back to the recent view
  useEffect(() => {
    if (selection.type === "project" && !project) onSelect({ type: "recent" })
  }, [selection, project, onSelect])

  useEffect(() => {
    if (!notice) return
    const timer = setTimeout(() => setNotice(null), 4000)
    return () => clearTimeout(timer)
  }, [notice])

  const entries = useMemo<BoardEntry[]>(() => {
    const all = project
      ? project.boards.map((board) => ({ board, project }))
      : workspace.projects
          .flatMap((p) => p.boards.map((board) => ({ board, project: p })))
          .sort((a, b) => b.board.updatedAt - a.board.updatedAt)
          .slice(0, RECENT_LIMIT)
    const needle = query.trim().toLowerCase()
    return needle ? all.filter((entry) => entry.board.name.toLowerCase().includes(needle)) : all
  }, [workspace, project, query])

  const run = async (action: () => Promise<unknown>) => {
    try {
      await action()
    } catch (error) {
      setNotice(error instanceof Error ? error.message.replace(/^Error invoking remote method '[^']+': (Error: )?/, "") : String(error))
    } finally {
      await onChanged()
    }
  }

  const createProject = async () => {
    const name = await dialogs.prompt({ title: "Nuevo proyecto", label: "Nombre", confirmLabel: "Crear" })
    if (!name) return
    await run(async () => {
      const created = await window.boards.createProject(name)
      onSelect({ type: "project", id: created.id })
    })
  }

  const createBoard = async (target: Project) => {
    const name = await dialogs.prompt({
      title: "Nueva pizarra",
      label: "Nombre",
      initialValue: `Pizarra ${target.boards.length + 1}`,
      confirmLabel: "Crear",
    })
    if (!name) return
    await run(async () => {
      const board = await window.boards.createBoard(target.id, name)
      onOpenBoard(board.id)
    })
  }

  const renameProject = async (target: Project) => {
    const name = await dialogs.prompt({ title: "Renombrar proyecto", label: "Nombre", initialValue: target.name })
    if (name) await run(() => window.boards.updateProject(target.id, { name }))
  }

  const recolorProject = async (target: Project) => {
    const color = await dialogs.choose({
      title: "Color del proyecto",
      options: PROJECT_COLORS.map((value) => ({ value, label: value === target.color ? `${value} (actual)` : value, color: value })),
    })
    if (color) await run(() => window.boards.updateProject(target.id, { color }))
  }

  const deleteProject = async (target: Project) => {
    const ok = await dialogs.confirm({
      title: `¿Eliminar "${target.name}"?`,
      message: `Se eliminarán ${pluralize(target.boards.length, "pizarra", "pizarras")} de forma permanente.`,
      confirmLabel: "Eliminar proyecto",
      danger: true,
    })
    if (ok) await run(() => window.boards.deleteProject(target.id))
  }

  const boardActions = (entry: BoardEntry) => [
    {
      label: "Renombrar",
      icon: <Pencil size={14} />,
      onSelect: async () => {
        const name = await dialogs.prompt({ title: "Renombrar pizarra", label: "Nombre", initialValue: entry.board.name })
        if (name) await run(() => window.boards.renameBoard(entry.board.id, name))
      },
    },
    { label: "Duplicar", icon: <Copy size={14} />, onSelect: () => run(() => window.boards.duplicateBoard(entry.board.id)) },
    ...(workspace.projects.length > 1
      ? [
          {
            label: "Mover a…",
            icon: <ArrowRightLeft size={14} />,
            onSelect: async () => {
              const target = await dialogs.choose({
                title: "Mover pizarra a",
                options: workspace.projects
                  .filter((p) => p.id !== entry.project.id)
                  .map((p) => ({ value: p.id, label: p.name, color: p.color })),
              })
              if (target) await run(() => window.boards.moveBoard(entry.board.id, target))
            },
          },
        ]
      : []),
    {
      label: "Exportar .excalidraw",
      icon: <Download size={14} />,
      onSelect: () =>
        run(async () => {
          if (await window.boards.exportBoard(entry.board.id)) setNotice(`"${entry.board.name}" exportada`)
        }),
    },
    "separator" as const,
    {
      label: "Eliminar",
      icon: <Trash2 size={14} />,
      danger: true,
      onSelect: async () => {
        const ok = await dialogs.confirm({
          title: `¿Eliminar "${entry.board.name}"?`,
          message: "Esta acción no se puede deshacer.",
          confirmLabel: "Eliminar",
          danger: true,
        })
        if (ok) await run(() => window.boards.deleteBoard(entry.board.id))
      },
    },
  ]

  const totalBoards = workspace.projects.reduce((sum, p) => sum + p.boards.length, 0)

  return (
    <div className="manager">
      <aside className="sidebar">
        <div className="brand">
          <span className="brand-mark" aria-hidden="true" />
          Boards
        </div>

        <nav className="sidebar-nav" aria-label="Proyectos">
          <button
            type="button"
            className="nav-item"
            aria-current={selection.type === "recent" ? "page" : undefined}
            onClick={() => onSelect({ type: "recent" })}
          >
            <Clock size={15} />
            <span className="nav-label">Recientes</span>
            <span className="nav-count">{totalBoards}</span>
          </button>

          <div className="sidebar-heading">
            <span>Proyectos</span>
            <button type="button" className="icon-btn" aria-label="Nuevo proyecto" title="Nuevo proyecto" onClick={createProject}>
              <Plus size={15} />
            </button>
          </div>

          {workspace.projects.map((p) => (
            <button
              key={p.id}
              type="button"
              className="nav-item"
              aria-current={project?.id === p.id ? "page" : undefined}
              onClick={() => onSelect({ type: "project", id: p.id })}
            >
              <span className="dot" style={{ background: p.color }} />
              <span className="nav-label">{p.name}</span>
              <span className="nav-count">{p.boards.length}</span>
            </button>
          ))}

          {workspace.projects.length === 0 && <p className="sidebar-empty">Aún no tienes proyectos.</p>}
        </nav>

        <div className="sidebar-footer">
          <div className="theme-switch" role="group" aria-label="Tema">
            {([
              ["light", <Sun size={14} key="l" />, "Claro"],
              ["system", <Monitor size={14} key="s" />, "Sistema"],
              ["dark", <Moon size={14} key="d" />, "Oscuro"],
            ] as const).map(([value, icon, title]) => (
              <button
                key={value}
                type="button"
                title={title}
                aria-label={title}
                aria-pressed={themePreference === value}
                onClick={() => onThemePreferenceChange(value)}
              >
                {icon}
              </button>
            ))}
          </div>
          <button type="button" className="link-btn" onClick={() => void window.boards.openWorkspaceFolder()}>
            <FolderOpen size={14} /> Carpeta de datos
          </button>
        </div>
      </aside>

      <main className="content">
        {workspace.projects.length === 0 ? (
          <div className="welcome">
            <span className="brand-mark large" aria-hidden="true" />
            <h1>Tus pizarras, organizadas</h1>
            <p>Crea un proyecto para agrupar pizarras relacionadas. Todo se guarda localmente en tu equipo.</p>
            <button type="button" className="btn btn-primary" onClick={createProject}>
              <Plus size={16} /> Crear primer proyecto
            </button>
          </div>
        ) : (
          <>
            <header className="content-header">
              <div className="content-title">
                {project && <span className="dot large" style={{ background: project.color }} />}
                <div>
                  <h1>{project ? project.name : "Recientes"}</h1>
                  <p>
                    {project
                      ? `${pluralize(project.boards.length, "pizarra", "pizarras")} · actualizado ${timeAgo(project.updatedAt)}`
                      : "Las últimas pizarras que editaste"}
                  </p>
                </div>
                {project && (
                  <Menu
                    label="Opciones del proyecto"
                    items={[
                      { label: "Renombrar", icon: <Pencil size={14} />, onSelect: () => void renameProject(project) },
                      { label: "Cambiar color", icon: <Palette size={14} />, onSelect: () => void recolorProject(project) },
                      "separator",
                      { label: "Eliminar proyecto", icon: <Trash2 size={14} />, danger: true, onSelect: () => void deleteProject(project) },
                    ]}
                  />
                )}
              </div>

              <div className="content-actions">
                <label className="search">
                  <Search size={15} aria-hidden="true" />
                  <input
                    type="search"
                    placeholder="Buscar pizarras"
                    aria-label="Buscar pizarras"
                    value={query}
                    onChange={(event) => setQuery(event.target.value)}
                  />
                </label>
                {project && (
                  <>
                    <button
                      type="button"
                      className="btn btn-ghost"
                      onClick={() =>
                        run(async () => {
                          const imported = await window.boards.importBoards(project.id)
                          if (imported.length) setNotice(`${pluralize(imported.length, "pizarra importada", "pizarras importadas")}`)
                        })
                      }
                    >
                      <Upload size={15} /> Importar
                    </button>
                    <button type="button" className="btn btn-primary" onClick={() => createBoard(project)}>
                      <Plus size={16} /> Nueva pizarra
                    </button>
                  </>
                )}
              </div>
            </header>

            {entries.length > 0 ? (
              <div className="board-grid">
                {entries.map((entry) => (
                  <BoardCard
                    key={entry.board.id}
                    entry={entry}
                    showProject={!project}
                    onOpen={() => onOpenBoard(entry.board.id)}
                    actions={boardActions(entry)}
                  />
                ))}
                {project && !query && (
                  <button type="button" className="board-card board-card-new" onClick={() => createBoard(project)}>
                    <Plus size={22} />
                    Nueva pizarra
                  </button>
                )}
              </div>
            ) : (
              <div className="empty">
                {query ? (
                  <p>Ninguna pizarra coincide con “{query}”.</p>
                ) : project ? (
                  <>
                    <p>Este proyecto todavía no tiene pizarras.</p>
                    <button type="button" className="btn btn-primary" onClick={() => createBoard(project)}>
                      <Plus size={16} /> Crear pizarra
                    </button>
                  </>
                ) : (
                  <p>Todavía no has creado pizarras. Elige un proyecto en la barra lateral para empezar.</p>
                )}
              </div>
            )}
          </>
        )}
      </main>

      {notice && (
        <div className="toast" role="status">
          {notice}
        </div>
      )}
    </div>
  )
}

function BoardCard({
  entry,
  showProject,
  onOpen,
  actions,
}: {
  entry: BoardEntry
  showProject: boolean
  onOpen(): void
  actions: Parameters<typeof Menu>[0]["items"]
}) {
  const [thumbnail, setThumbnail] = useState<string | null>(null)

  useEffect(() => {
    let active = true
    void window.boards.getThumbnail(entry.board.id).then((url) => {
      if (active) setThumbnail(url)
    })
    return () => {
      active = false
    }
  }, [entry.board.id, entry.board.updatedAt])

  return (
    <article className="board-card">
      <button type="button" className={`board-thumb${thumbnail ? " has-image" : ""}`} onClick={onOpen} aria-label={`Abrir ${entry.board.name}`}>
        {thumbnail ? <img src={thumbnail} alt="" draggable={false} /> : <span className="thumb-empty">Pizarra vacía</span>}
      </button>
      <div className="board-meta">
        <div className="board-text" onClick={onOpen}>
          <h3 title={entry.board.name}>{entry.board.name}</h3>
          <p>
            {showProject && (
              <>
                <span className="dot small" style={{ background: entry.project.color }} />
                {entry.project.name} ·{" "}
              </>
            )}
            {timeAgo(entry.board.updatedAt)}
          </p>
        </div>
        <Menu items={actions} label={`Opciones de ${entry.board.name}`} />
      </div>
    </article>
  )
}
