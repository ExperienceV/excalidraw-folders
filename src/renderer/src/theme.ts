import { useEffect, useState } from "react"

export type ThemePreference = "system" | "light" | "dark"
export type Theme = "light" | "dark"

const STORAGE_KEY = "boards:theme"
const darkQuery = window.matchMedia("(prefers-color-scheme: dark)")

function readPreference(): ThemePreference {
  try {
    const value = localStorage.getItem(STORAGE_KEY)
    if (value === "light" || value === "dark" || value === "system") return value
  } catch {
    // storage unavailable: fall back to the system theme
  }
  return "system"
}

export function useTheme() {
  const [preference, setPreferenceState] = useState<ThemePreference>(readPreference)
  const [systemDark, setSystemDark] = useState(darkQuery.matches)

  useEffect(() => {
    const onChange = (event: MediaQueryListEvent) => setSystemDark(event.matches)
    darkQuery.addEventListener("change", onChange)
    return () => darkQuery.removeEventListener("change", onChange)
  }, [])

  const theme: Theme = preference === "system" ? (systemDark ? "dark" : "light") : preference

  useEffect(() => {
    document.documentElement.dataset.theme = theme
  }, [theme])

  const setPreference = (next: ThemePreference) => {
    setPreferenceState(next)
    try {
      localStorage.setItem(STORAGE_KEY, next)
    } catch {
      // ignore
    }
  }

  return { theme, preference, setPreference }
}
