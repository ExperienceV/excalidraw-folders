import { useCallback, useEffect, useRef, useState, type MutableRefObject } from "react"
import { ArrowLeft, Download } from "lucide-react"
import {
  Excalidraw,
  MainMenu,
  exportToBlob,
  hashElementsVersion,
  serializeAsJSON,
  serializeLibraryAsJSON,
} from "@excalidraw/excalidraw"
import type { ExcalidrawImperativeAPI, ExcalidrawInitialDataState } from "@excalidraw/excalidraw/types"
import type { Board, Project } from "../../../shared/types"
import type { Theme, ThemePreference } from "../theme"
import { useDialogs } from "./Dialogs"
import { PenStatus } from "./PenStatus"

const SAVE_DEBOUNCE_MS = 700
const THUMBNAIL_INTERVAL_MS = 5000
const THUMBNAIL_SIZE = 640

type SaveState = "saved" | "pending" | "saving" | "error"

interface BoardEditorProps {
  project: Project
  board: Board
  theme: Theme
  onThemeChange(preference: ThemePreference): void
  onClose(): Promise<void>
  onRenamed(): Promise<void>
  flushRef: MutableRefObject<(() => Promise<void>) | null>
}

function blobToDataUrl(blob: Blob) {
  return new Promise<string>((resolve, reject) => {
    const reader = new FileReader()
    reader.onload = () => resolve(reader.result as string)
    reader.onerror = () => reject(reader.error)
    reader.readAsDataURL(blob)
  })
}

export function BoardEditor({ project, board, theme, onThemeChange, onClose, onRenamed, flushRef }: BoardEditorProps) {
  const dialogs = useDialogs()
  const [initialData, setInitialData] = useState<ExcalidrawInitialDataState | null>(null)
  const [loadError, setLoadError] = useState<string | null>(null)
  const [saveState, setSaveState] = useState<SaveState>("saved")

  const apiRef = useRef<ExcalidrawImperativeAPI | null>(null)
  const lastSignatureRef = useRef<string | null>(null)
  const saveTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null)
  const lastThumbnailAtRef = useRef(0)
  const savingRef = useRef<Promise<void>>(Promise.resolve())

  useEffect(() => {
    let active = true
    void (async () => {
      try {
        const [scene, library] = await Promise.all([window.boards.loadBoard(board.id), window.boards.loadLibrary()])
        const parsed = scene ? JSON.parse(scene) : {}
        const libraryItems = library ? JSON.parse(library).libraryItems ?? [] : []
        if (!active) return
        setInitialData({
          elements: parsed.elements ?? [],
          appState: { ...parsed.appState, name: board.name },
          files: parsed.files ?? {},
          libraryItems,
          scrollToContent: true,
        })
      } catch (error) {
        if (active) setLoadError(error instanceof Error ? error.message : String(error))
      }
    })()
    return () => {
      active = false
    }
    // Only the board identity matters; renames must not reload the scene
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [board.id])

  const save = useCallback(
    async (forceThumbnail: boolean) => {
      const api = apiRef.current
      if (!api) return
      const elements = api.getSceneElements()
      const appState = api.getAppState()
      const files = api.getFiles()
      const scene = serializeAsJSON(elements, appState, files, "local")

      let thumbnail: string | undefined
      if (forceThumbnail || Date.now() - lastThumbnailAtRef.current > THUMBNAIL_INTERVAL_MS) {
        lastThumbnailAtRef.current = Date.now()
        if (elements.length === 0) {
          thumbnail = "" // tells the main process to drop a stale preview
        } else {
          const blob = await exportToBlob({
            elements,
            appState: { ...appState, exportBackground: true, exportWithDarkMode: false },
            files,
            mimeType: "image/png",
            maxWidthOrHeight: THUMBNAIL_SIZE,
          })
          thumbnail = await blobToDataUrl(blob)
        }
      }

      setSaveState("saving")
      try {
        await window.boards.saveBoard(board.id, scene, thumbnail)
        setSaveState((state) => (state === "saving" ? "saved" : state))
      } catch (error) {
        console.error("Could not save board", error)
        setSaveState("error")
      }
    },
    [board.id],
  )

  // Saves are chained so two never write the same file concurrently
  const queueSave = useCallback(
    (forceThumbnail: boolean) => {
      savingRef.current = savingRef.current.then(() => save(forceThumbnail))
      return savingRef.current
    },
    [save],
  )

  const flush = useCallback(async () => {
    if (saveTimerRef.current) {
      clearTimeout(saveTimerRef.current)
      saveTimerRef.current = null
      await queueSave(true)
    } else {
      await savingRef.current
    }
  }, [queueSave])

  // Boards imported or created elsewhere have content but no preview yet
  useEffect(() => {
    if (!initialData?.elements?.length) return
    let active = true
    const timer = setTimeout(async () => {
      if (active && !(await window.boards.getThumbnail(board.id))) void queueSave(true)
    }, 800)
    return () => {
      active = false
      clearTimeout(timer)
    }
  }, [initialData, board.id, queueSave])

  useEffect(() => {
    flushRef.current = flush
    return () => {
      if (flushRef.current === flush) flushRef.current = null
    }
  }, [flush, flushRef])

  const handleChange: NonNullable<React.ComponentProps<typeof Excalidraw>["onChange"]> = (elements, appState, files) => {
    // onChange fires for every pointer move; only persist actual content changes
    const signature = `${hashElementsVersion(elements)}:${appState.viewBackgroundColor}:${Object.keys(files).length}`
    if (lastSignatureRef.current === null) {
      lastSignatureRef.current = signature
      return
    }
    if (appState.theme !== theme) onThemeChange(appState.theme)
    if (signature === lastSignatureRef.current) return
    lastSignatureRef.current = signature

    setSaveState("pending")
    if (saveTimerRef.current) clearTimeout(saveTimerRef.current)
    saveTimerRef.current = setTimeout(() => {
      saveTimerRef.current = null
      void queueSave(false)
    }, SAVE_DEBOUNCE_MS)
  }

  const renameBoard = async () => {
    const name = await dialogs.prompt({ title: "Renombrar pizarra", label: "Nombre", initialValue: board.name })
    if (!name) return
    await window.boards.renameBoard(board.id, name)
    await onRenamed()
  }

  return (
    <div className="editor">
      <header className="editor-bar">
        <button type="button" className="btn btn-ghost btn-sm" onClick={() => void onClose()}>
          <ArrowLeft size={15} /> Proyectos
        </button>
        <div className="breadcrumb">
          <span className="dot" style={{ background: project.color }} />
          <span className="breadcrumb-project">{project.name}</span>
          <span aria-hidden="true">/</span>
          <button type="button" className="breadcrumb-board" title="Renombrar" onClick={() => void renameBoard()}>
            {board.name}
          </button>
        </div>
        <div className="editor-bar-end">
          <PenStatus />
          <span className={`save-state save-${saveState}`} role="status">
            {saveState === "saved" && "Guardado"}
            {saveState === "pending" && "Cambios sin guardar"}
            {saveState === "saving" && "Guardando…"}
            {saveState === "error" && "Error al guardar"}
          </span>
        </div>
      </header>

      <div className="canvas">
        {loadError ? (
          <div className="fatal">
            <h1>No se pudo abrir la pizarra</h1>
            <p>{loadError}</p>
          </div>
        ) : initialData ? (
          <Excalidraw
            initialData={initialData}
            excalidrawAPI={(api) => {
              apiRef.current = api
            }}
            onChange={handleChange}
            onLibraryChange={(items) => window.boards.saveLibrary(serializeLibraryAsJSON(items))}
            name={board.name}
            theme={theme}
            langCode="es-ES"
            autoFocus
            UIOptions={{
              canvasActions: {
                loadScene: false,
                saveToActiveFile: false,
                export: false,
                saveAsImage: true,
                clearCanvas: true,
                changeViewBackgroundColor: true,
                toggleTheme: true,
              },
            }}
          >
            <MainMenu>
              <MainMenu.Item icon={<ArrowLeft size={14} />} onSelect={() => void onClose()}>
                Volver a proyectos
              </MainMenu.Item>
              <MainMenu.Separator />
              <MainMenu.DefaultItems.SaveAsImage />
              <MainMenu.Item
                icon={<Download size={14} />}
                onSelect={() => void flush().then(() => window.boards.exportBoard(board.id))}
              >
                Exportar .excalidraw
              </MainMenu.Item>
              <MainMenu.DefaultItems.SearchMenu />
              <MainMenu.DefaultItems.Help />
              <MainMenu.DefaultItems.ClearCanvas />
              <MainMenu.Separator />
              <MainMenu.DefaultItems.ToggleTheme />
              <MainMenu.DefaultItems.ChangeCanvasBackground />
            </MainMenu>
          </Excalidraw>
        ) : null}
      </div>
    </div>
  )
}
