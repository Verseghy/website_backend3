/**
 * Shared setup of the unit tests.
 *
 * - Registers the jest-dom matchers (`toBeInTheDocument` …).
 * - Unmounts rendered trees after each test.
 * - Fails any test that writes to `console.error` without expecting it. The
 *   i18n provider reports a missing Hungarian key through `console.error`
 *   (ADR 0021 §3), so a screen that would show English fails its test. React
 *   reports its own runtime warnings the same way.
 *
 * A test that expects an error message replaces `console.error` with
 * `vi.spyOn(console, "error").mockImplementation(...)`; `restoreMocks` undoes it.
 */
import "@testing-library/jest-dom/vitest"
import { cleanup } from "@testing-library/react"
import { afterEach, beforeEach, vi } from "vitest"

// jsdom implements neither matchMedia nor ResizeObserver. The kit's theme
// provider and sidebar call matchMedia, and Base UI popups observe sizes, so
// both get inert stand-ins: no media query matches, nothing is ever resized.
// Files that run in the `node` environment have no `window` at all.
const hasWindow = typeof window !== "undefined"
if (hasWindow && typeof window.matchMedia !== "function") {
  window.matchMedia = (query: string): MediaQueryList => ({
    matches: false,
    media: query,
    onchange: null,
    addEventListener: () => {},
    removeEventListener: () => {},
    addListener: () => {},
    removeListener: () => {},
    dispatchEvent: () => false,
  })
}
if (hasWindow && typeof window.ResizeObserver !== "function") {
  window.ResizeObserver = class {
    observe() {}
    unobserve() {}
    disconnect() {}
  }
}

let unexpected: unknown[][] = []

beforeEach(() => {
  unexpected = []
  vi.spyOn(console, "error").mockImplementation((...args: unknown[]) => {
    unexpected.push(args)
  })
})

afterEach(() => {
  if (hasWindow) {
    cleanup()
  }
  // A test that installed its own spy replaced ours, so `unexpected` stays empty.
  const calls = unexpected
  unexpected = []
  if (calls.length > 0) {
    throw new Error(
      `Unexpected console.error during the test:\n${calls.map((args) => args.map(String).join(" ")).join("\n")}`
    )
  }
})
