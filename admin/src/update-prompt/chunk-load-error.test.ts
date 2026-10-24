import { describe, expect, it, vi } from "vitest"

import { isChunkLoadError, listenForChunkLoadErrors } from "./chunk-load-error"

describe("isChunkLoadError", () => {
  it.each([
    [
      "Chromium",
      "Failed to fetch dynamically imported module: https://x/admin/assets/a-1.js",
    ],
    [
      "Firefox",
      "error loading dynamically imported module: https://x/admin/assets/a-1.js",
    ],
    ["Safari", "Importing a module script failed."],
    ["Vite CSS preload", "Unable to preload CSS for /admin/assets/a-1.css"],
  ])("recognises the %s message", (_engine, message) => {
    expect(isChunkLoadError(new TypeError(message))).toBe(true)
  })

  it("recognises an error named ChunkLoadError", () => {
    const error = new Error("Loading chunk 3 failed")
    error.name = "ChunkLoadError"
    expect(isChunkLoadError(error)).toBe(true)
  })

  it.each([
    ["an unrelated error", new Error("Network request failed")],
    ["a string", "Failed to fetch dynamically imported module"],
    ["null", null],
    ["undefined", undefined],
    ["an object without a message", { name: "TypeError" }],
  ])("rejects %s", (_label, reason) => {
    expect(isChunkLoadError(reason)).toBe(false)
  })
})

describe("listenForChunkLoadErrors", () => {
  it("reports Vite's preload errors with their payload", () => {
    const target = new EventTarget() as unknown as Window
    const onError = vi.fn()
    listenForChunkLoadErrors(target, onError)

    const event = new Event("vite:preloadError", {
      cancelable: true,
    }) as Event & { payload?: unknown }
    event.payload = new TypeError(
      "Failed to fetch dynamically imported module: /admin/assets/x.js"
    )
    target.dispatchEvent(event)

    expect(onError).toHaveBeenCalledWith(event.payload)
    // The import must still reject so the caller's own error handling runs.
    expect(event.defaultPrevented).toBe(false)
  })

  it("reports unhandled rejections only when they are chunk-load errors", () => {
    const target = new EventTarget() as unknown as Window
    const onError = vi.fn()
    listenForChunkLoadErrors(target, onError)

    const rejection = (reason: unknown) =>
      Object.assign(new Event("unhandledrejection"), { reason })
    target.dispatchEvent(rejection(new Error("something else")))
    target.dispatchEvent(
      rejection(new TypeError("Importing a module script failed."))
    )

    expect(onError).toHaveBeenCalledTimes(1)
  })

  it("stops listening after cleanup", () => {
    const target = new EventTarget() as unknown as Window
    const onError = vi.fn()
    const stop = listenForChunkLoadErrors(target, onError)
    stop()
    target.dispatchEvent(new Event("vite:preloadError"))
    expect(onError).not.toHaveBeenCalled()
  })
})
