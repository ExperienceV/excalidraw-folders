import { useCallback, useEffect, useRef, useState } from "react"
import type { Workspace } from "../../shared/types"
import { BoardEditor } from "./components/BoardEditor"
import { DialogsProvider } from "./components/Dialogs"
import { ProjectManager, type Selection } from "./components/ProjectManager"
import { useTheme } from "./theme"

export function App() {
  const { theme, preference, setPreference } = useTheme()
  const [workspace, setWorkspace] = useState<Workspace | null>(null)
  const [error, setError] = useState<string | null>(null)
  const [selection, setSelection] = useState<Selection>({ type: "recent" })
  const [openBoardId, setOpenBoardId] = useState<string | null>(null)
  // The editor registers its save routine here so closing the window can flush it
  const flushRef = useRef<(() => Promise<void>) | null>(null)

  const refresh = useCallback(async () => {
    try {
      setWorkspace(await window.boards.getWorkspace())
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err))
    }
  }, [])

  useEffect(() => {
    void refresh()
  }, [refresh])

  useEffect(
    () => window.boards.onFlushRequest(async () => {
      await flushRef.current?.()
    }),
    [],
  )

  const openBoard = useCallback((boardId: string) => setOpenBoardId(boardId), [])

  const closeBoard = useCallback(async () => {
    await flushRef.current?.()
    setOpenBoardId(null)
    await refresh()
  }, [refresh])

  if (error) {
    return (
      <div className="fatal">
        <h1>No se pudo abrir el espacio de trabajo</h1>
        <p>{error}</p>
      </div>
    )
  }

  if (!workspace) return <div className="loading" />

  const location = openBoardId
    ? workspace.projects
        .map((project) => ({ project, board: project.boards.find((b) => b.id === openBoardId) }))
        .find((entry) => entry.board)
    : undefined

  return (
    <DialogsProvider>
      {location?.board ? (
        <BoardEditor
          key={location.board.id}
          project={location.project}
          board={location.board}
          theme={theme}
          onThemeChange={setPreference}
          onClose={closeBoard}
          onRenamed={refresh}
          flushRef={flushRef}
        />
      ) : (
        <ProjectManager
          workspace={workspace}
          selection={selection}
          onSelect={setSelection}
          onOpenBoard={openBoard}
          onChanged={refresh}
          themePreference={preference}
          onThemePreferenceChange={setPreference}
        />
      )}
    </DialogsProvider>
  )
}
