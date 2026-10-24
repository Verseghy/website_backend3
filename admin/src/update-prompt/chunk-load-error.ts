/**
 * Detecting chunk-load errors after a deploy (ADR 0004 §5).
 *
 * An editor's open tab keeps the old JavaScript after a new image goes live.
 * The old hashed chunks no longer exist, so the next lazy import fails. The
 * admin then asks the editor to reload; `index.html` is served `no-cache`, so
 * the reload fetches the new version.
 *
 * Browsers word the failure differently, and Vite reports failed preloads
 * through its own `vite:preloadError` event. This module recognises all of
 * them and is free of React so it can be unit-tested directly.
 */

/**
 * Messages of failed dynamic imports and preloads, by engine. Matched as
 * substrings because the browsers append the module URL.
 */
const CHUNK_LOAD_MESSAGES = [
  // Chromium: TypeError
  "Failed to fetch dynamically imported module",
  // Firefox: TypeError
  "error loading dynamically imported module",
  // Safari: TypeError
  "Importing a module script failed",
  // Vite's preload helper, for a stylesheet a chunk depends on
  "Unable to preload CSS for",
] as const

/**
 * Tells whether an error (or rejection reason) is a failed chunk load.
 *
 * @param reason - anything thrown or rejected; non-errors are never chunk-load
 *   errors.
 * @returns `true` for a failed dynamic import or Vite preload.
 */
export function isChunkLoadError(reason: unknown): boolean {
  if (typeof reason !== "object" || reason === null) {
    return false
  }
  const { name, message } = reason as { name?: unknown; message?: unknown }
  if (name === "ChunkLoadError") {
    return true
  }
  return (
    typeof message === "string" &&
    CHUNK_LOAD_MESSAGES.some((known) => message.includes(known))
  )
}

/** The window events the listener needs; a narrow interface keeps tests simple. */
export type ChunkLoadEventTarget = Pick<
  Window,
  "addEventListener" | "removeEventListener"
>

/**
 * Calls `onChunkLoadError` whenever the page reports a failed chunk load.
 *
 * It listens to Vite's `vite:preloadError` (dispatched for failed preloads and
 * failed dynamic imports wrapped by Vite's preload helper) and to
 * `unhandledrejection` (a bare `import()` that nobody awaited). It never calls
 * `preventDefault()`, so the import still rejects and the caller's own error
 * handling runs; the prompt is an addition, not a replacement.
 *
 * @param target - the window to listen on.
 * @param onChunkLoadError - called with each chunk-load error.
 * @returns a function that removes both listeners.
 */
export function listenForChunkLoadErrors(
  target: ChunkLoadEventTarget,
  onChunkLoadError: (error: unknown) => void
): () => void {
  const onPreloadError = (event: Event) => {
    // Vite puts the original error on `payload`; any preload failure counts.
    onChunkLoadError((event as Event & { payload?: unknown }).payload)
  }
  const onRejection = (event: PromiseRejectionEvent) => {
    if (isChunkLoadError(event.reason)) {
      onChunkLoadError(event.reason)
    }
  }
  target.addEventListener("vite:preloadError", onPreloadError)
  target.addEventListener("unhandledrejection", onRejection)
  return () => {
    target.removeEventListener("vite:preloadError", onPreloadError)
    target.removeEventListener("unhandledrejection", onRejection)
  }
}
