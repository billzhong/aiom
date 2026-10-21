import { createContext } from "react"

export type Theme = "dark" | "light" | "system"

export const THEME_STORAGE_KEY = "theme"

export interface ThemeState {
  /** The chosen theme. */
  theme: Theme
  setTheme: (theme: Theme) => void
}

export const ThemeContext = createContext<ThemeState | undefined>(undefined)
