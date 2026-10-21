import "@testing-library/jest-dom/vitest"
import "@/i18n"
import { cleanup } from "@testing-library/react"
import i18n from "i18next"
import { afterEach, beforeEach } from "vitest"

// jsdom lacks the layout APIs Recharts and the theme provider rely on.
class ResizeObserverStub {
  observe() {}
  unobserve() {}
  disconnect() {}
}
globalThis.ResizeObserver ??= ResizeObserverStub

// Used by Radix menus.
Element.prototype.scrollIntoView ??= () => {}
Element.prototype.hasPointerCapture ??= () => false
Element.prototype.releasePointerCapture ??= () => {}

window.matchMedia ??= (query: string) =>
  ({
    matches: false,
    media: query,
    onchange: null,
    addEventListener: () => {},
    removeEventListener: () => {},
    addListener: () => {},
    removeListener: () => {},
    dispatchEvent: () => false,
  }) as MediaQueryList

beforeEach(async () => {
  localStorage.clear()
  await i18n.changeLanguage("en")
})

afterEach(() => {
  cleanup()
})
