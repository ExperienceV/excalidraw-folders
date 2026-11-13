import { createContext, useCallback, useContext, useEffect, useRef, useState, type ReactNode } from "react"

type DialogRequest =
  | { kind: "prompt"; title: string; label: string; initialValue: string; confirmLabel: string; resolve: (value: string | null) => void }
  | { kind: "confirm"; title: string; message: string; confirmLabel: string; danger: boolean; resolve: (value: boolean) => void }
  | { kind: "choose"; title: string; options: { value: string; label: string; color?: string }[]; resolve: (value: string | null) => void }

interface DialogsApi {
  prompt(options: { title: string; label: string; initialValue?: string; confirmLabel?: string }): Promise<string | null>
  confirm(options: { title: string; message: string; confirmLabel?: string; danger?: boolean }): Promise<boolean>
  choose(options: { title: string; options: { value: string; label: string; color?: string }[] }): Promise<string | null>
}

const DialogsContext = createContext<DialogsApi | null>(null)

export function useDialogs() {
  const context = useContext(DialogsContext)
  if (!context) throw new Error("useDialogs must be used inside DialogsProvider")
  return context
}

export function DialogsProvider({ children }: { children: ReactNode }) {
  const [request, setRequest] = useState<DialogRequest | null>(null)

  const prompt = useCallback<DialogsApi["prompt"]>(
    (options) =>
      new Promise((resolve) =>
        setRequest({
          kind: "prompt",
          title: options.title,
          label: options.label,
          initialValue: options.initialValue ?? "",
          confirmLabel: options.confirmLabel ?? "Guardar",
          resolve,
        }),
      ),
    [],
  )

  const confirm = useCallback<DialogsApi["confirm"]>(
    (options) =>
      new Promise((resolve) =>
        setRequest({
          kind: "confirm",
          title: options.title,
          message: options.message,
          confirmLabel: options.confirmLabel ?? "Aceptar",
          danger: options.danger ?? false,
          resolve,
        }),
      ),
    [],
  )

  const choose = useCallback<DialogsApi["choose"]>(
    (options) => new Promise((resolve) => setRequest({ kind: "choose", ...options, resolve })),
    [],
  )

  const close = (value: unknown) => {
    if (!request) return
    ;(request.resolve as (value: unknown) => void)(value)
    setRequest(null)
  }

  return (
    <DialogsContext.Provider value={{ prompt, confirm, choose }}>
      {children}
      {request && <Modal request={request} onClose={close} />}
    </DialogsContext.Provider>
  )
}

function Modal({ request, onClose }: { request: DialogRequest; onClose: (value: unknown) => void }) {
  const dialogRef = useRef<HTMLDialogElement>(null)
  const [value, setValue] = useState(request.kind === "prompt" ? request.initialValue : "")
  const cancelValue = request.kind === "confirm" ? false : null

  useEffect(() => {
    dialogRef.current?.showModal()
  }, [])

  return (
    <dialog
      ref={dialogRef}
      className="modal"
      onCancel={(event) => {
        event.preventDefault()
        onClose(cancelValue)
      }}
      onClick={(event) => {
        if (event.target === dialogRef.current) onClose(cancelValue)
      }}
    >
      <form
        method="dialog"
        onSubmit={(event) => {
          event.preventDefault()
          if (request.kind === "prompt") {
            if (value.trim()) onClose(value.trim())
          } else if (request.kind === "confirm") {
            onClose(true)
          }
        }}
      >
        <h2>{request.title}</h2>

        {request.kind === "prompt" && (
          <label className="field">
            <span>{request.label}</span>
            <input
              autoFocus
              value={value}
              maxLength={120}
              onChange={(event) => setValue(event.target.value)}
              onFocus={(event) => event.target.select()}
            />
          </label>
        )}

        {request.kind === "confirm" && <p className="modal-message">{request.message}</p>}

        {request.kind === "choose" && (
          <div className="choice-list">
            {request.options.map((option) => (
              <button key={option.value} type="button" className="choice" onClick={() => onClose(option.value)}>
                {option.color && <span className="dot" style={{ background: option.color }} />}
                {option.label}
              </button>
            ))}
          </div>
        )}

        <div className="modal-actions">
          <button type="button" className="btn btn-ghost" onClick={() => onClose(cancelValue)}>
            Cancelar
          </button>
          {request.kind !== "choose" && (
            <button
              type="submit"
              className={`btn ${request.kind === "confirm" && request.danger ? "btn-danger" : "btn-primary"}`}
              disabled={request.kind === "prompt" && !value.trim()}
              autoFocus={request.kind === "confirm"}
            >
              {request.confirmLabel}
            </button>
          )}
        </div>
      </form>
    </dialog>
  )
}
