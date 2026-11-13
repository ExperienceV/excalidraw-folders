import { useEffect, useRef, useState, type ReactNode } from "react"
import { MoreHorizontal } from "lucide-react"

export interface MenuItem {
  label: string
  icon?: ReactNode
  danger?: boolean
  onSelect: () => void
}

/** Small "⋯" dropdown used by project headers and board cards. */
export function Menu({ items, label = "Más acciones" }: { items: (MenuItem | "separator")[]; label?: string }) {
  const [open, setOpen] = useState(false)
  const rootRef = useRef<HTMLDivElement>(null)

  useEffect(() => {
    if (!open) return
    const onPointerDown = (event: PointerEvent) => {
      if (!rootRef.current?.contains(event.target as Node)) setOpen(false)
    }
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Escape") setOpen(false)
    }
    document.addEventListener("pointerdown", onPointerDown)
    document.addEventListener("keydown", onKeyDown)
    return () => {
      document.removeEventListener("pointerdown", onPointerDown)
      document.removeEventListener("keydown", onKeyDown)
    }
  }, [open])

  return (
    <div className="menu" ref={rootRef} onClick={(event) => event.stopPropagation()} onDoubleClick={(event) => event.stopPropagation()}>
      <button
        type="button"
        className="icon-btn"
        aria-label={label}
        aria-haspopup="menu"
        aria-expanded={open}
        onClick={() => setOpen((value) => !value)}
      >
        <MoreHorizontal size={16} />
      </button>
      {open && (
        <div className="menu-popover" role="menu">
          {items.map((item, index) =>
            item === "separator" ? (
              <hr key={`sep-${index}`} />
            ) : (
              <button
                key={item.label}
                type="button"
                role="menuitem"
                className={item.danger ? "danger" : undefined}
                onClick={() => {
                  setOpen(false)
                  item.onSelect()
                }}
              >
                {item.icon}
                {item.label}
              </button>
            ),
          )}
        </div>
      )}
    </div>
  )
}
