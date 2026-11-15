/**
 * A `fetch` stub for a request that never gets an answer, like one on a
 * stalled connection: the tests of the session refresh's timeout use it
 * (`src/auth/auth-provider.test.ts`, `src/auth/activity-refresh.test.ts`).
 *
 * Why it exists: neither the browser nor backend3 ends a stalled request by
 * itself, so code that must go on after one bounds it with an
 * `AbortSignal`. The stub settles only when that signal aborts, and then
 * rejects with the abort's reason, as the browser's `fetch` does; a request
 * without a signal is refused at once, so a test of unbounded code fails
 * instead of hanging.
 */
import { vi } from "vitest"

/** What {@link hangingFetch} returns. */
export interface HangingFetch {
  /** The stub, to pass where a `fetch` is expected. */
  readonly fetch: ReturnType<typeof vi.fn<typeof globalThis.fetch>>
  /** The signal of every call the stub received, in order. */
  readonly signals: readonly AbortSignal[]
}

/**
 * Builds a `fetch` stub that never answers.
 *
 * @returns the stub and the signals of its calls; see {@link HangingFetch}.
 */
export function hangingFetch(): HangingFetch {
  const signals: AbortSignal[] = []
  const fetch = vi.fn<typeof globalThis.fetch>(
    (_input, init) =>
      new Promise<Response>((_resolve, reject) => {
        const signal = init?.signal
        if (!signal) {
          reject(
            new Error("the request has no signal, so it could hang forever")
          )
          return
        }
        signals.push(signal)
        signal.addEventListener(
          "abort",
          () => {
            reject(signal.reason as Error)
          },
          { once: true }
        )
      })
  )
  return { fetch, signals }
}
