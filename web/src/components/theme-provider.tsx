import { type ReactNode, useEffect, useMemo, useState, useSyncExternalStore } from "react"

import { type Theme, THEME_STORAGE_KEY, ThemeContext } from "@/lib/theme"

const DARK_QUERY = "(prefers-color-scheme: dark)"

function subscribeToSystemTheme(onChange: () => void) {
  const query = window.matchMedia(DARK_QUERY)
  query.addEventListener("change", onChange)
  return () => query.removeEventListener("change", onChange)
}

const systemPrefersDark = () => window.matchMedia(DARK_QUERY).matches

function storedTheme(): Theme {
  try {
    const value = localStorage.getItem(THEME_STORAGE_KEY)
    return value === "dark" || value === "light" ? value : "system"
  } catch {
    return "system"
  }
}

export function ThemeProvider({ children }: { children: ReactNode }) {
  const [theme, setThemeState] = useState<Theme>(storedTheme)
  // Follows the OS setting live while the theme is "system".
  const prefersDark = useSyncExternalStore(subscribeToSystemTheme, systemPrefersDark)
  const resolvedTheme = theme === "system" ? (prefersDark ? "dark" : "light") : theme

  useEffect(() => {
    const root = document.documentElement
    root.classList.remove("light", "dark")
    root.classList.add(resolvedTheme)
  }, [resolvedTheme])

  const value = useMemo(
    () => ({
      theme,
      setTheme: (next: Theme) => {
        try {
          localStorage.setItem(THEME_STORAGE_KEY, next)
        } catch {
          // The theme can still change when the browser blocks storage or its quota is full.
        }
        setThemeState(next)
      },
    }),
    [theme],
  )

  return <ThemeContext.Provider value={value}>{children}</ThemeContext.Provider>
}
