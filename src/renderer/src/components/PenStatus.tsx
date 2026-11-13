import { useEffect, useState } from "react"
import { PenLine } from "lucide-react"

/**
 * Shows the live stylus pressure reported by the browser engine. Hidden until a pen
 * is used, so it doubles as a quick check that the tablet's pressure reaches the app.
 */
export function PenStatus() {
  const [pressure, setPressure] = useState<number | null>(null)

  useEffect(() => {
    let frame = 0
    let latest = 0
    const onPointer = (event: PointerEvent) => {
      if (event.pointerType !== "pen") return
      latest = event.buttons ? event.pressure : 0
      if (!frame) {
        frame = requestAnimationFrame(() => {
          frame = 0
          setPressure(latest)
        })
      }
    }
    window.addEventListener("pointerdown", onPointer, { capture: true, passive: true })
    window.addEventListener("pointermove", onPointer, { capture: true, passive: true })
    window.addEventListener("pointerup", onPointer, { capture: true, passive: true })
    return () => {
      cancelAnimationFrame(frame)
      window.removeEventListener("pointerdown", onPointer, { capture: true })
      window.removeEventListener("pointermove", onPointer, { capture: true })
      window.removeEventListener("pointerup", onPointer, { capture: true })
    }
  }, [])

  if (pressure === null) return null

  return (
    <span className="pen-status" title="Presión del lápiz detectada">
      <PenLine size={13} aria-hidden="true" />
      <span className="pen-meter" aria-hidden="true">
        <span style={{ transform: `scaleX(${pressure})` }} />
      </span>
      {Math.round(pressure * 100)}%
    </span>
  )
}
