/**
 * The activity tracker of the sliding session refresh (ADR 0006 §1): when
 * it calls the refresh, and which events count as activity.
 */
import fc from "fast-check"
import { describe, expect, it, vi } from "vitest"

import { hangingFetch } from "@/test/hanging-fetch"

import {
  ACTIVITY_EVENTS,
  createActivityRefresh,
  listenForActivity,
  REFRESH_INTERVAL_MS,
} from "./activity-refresh"
import { createAuthProvider } from "./auth-provider"
import { createSessionExpiry } from "./session-expiry"

const MINUTE = 60 * 1000

/**
 * A tracker over a settable time and a refresh stub that resolves at once.
 *
 * @param refresh - the refresh stub; one that resolves by default.
 * @returns the activity handler, the stub and a setter for the time.
 */
function tracker(
  refresh = vi.fn<() => Promise<unknown>>(async () => "refreshed")
) {
  let time = 1_800_000_000_000
  const onActivity = createActivityRefresh({ refresh, now: () => time })
  return {
    onActivity,
    refresh,
    advance(ms: number) {
      time += ms
    },
  }
}

/** Lets the refresh's promise chain settle. */
async function settle(): Promise<void> {
  await new Promise((resolve) => setTimeout(resolve, 0))
}

describe("createActivityRefresh", () => {
  it("waits 10 minutes between two calls (ADR 0006 §1)", () => {
    expect(REFRESH_INTERVAL_MS).toBe(10 * MINUTE)
  })

  it("refreshes on the first activity, then at most every interval", async () => {
    const { onActivity, refresh, advance } = tracker()
    onActivity()
    expect(refresh).toHaveBeenCalledTimes(1)
    await settle()

    // Within the interval, however much activity: no call.
    for (const step of [1, MINUTE, 8 * MINUTE]) {
      advance(step)
      onActivity()
      onActivity()
    }
    await settle()
    expect(refresh).toHaveBeenCalledTimes(1)

    // Ten minutes after the first call: the next activity calls again.
    advance(MINUTE - 1)
    onActivity()
    expect(refresh).toHaveBeenCalledTimes(2)
    await settle()

    // A long pause without activity calls nothing; the next activity does.
    advance(3 * 60 * MINUTE)
    await settle()
    expect(refresh).toHaveBeenCalledTimes(2)
    onActivity()
    expect(refresh).toHaveBeenCalledTimes(3)
  })

  it("starts no second call while one is open", async () => {
    let resolve: (() => void) | undefined
    const refresh = vi.fn<() => Promise<unknown>>(
      () =>
        new Promise<void>((done) => {
          resolve = done
        })
    )
    const { onActivity, advance } = tracker(refresh)
    onActivity()
    await settle()
    expect(refresh).toHaveBeenCalledTimes(1)
    advance(REFRESH_INTERVAL_MS * 2)
    onActivity()
    await settle()
    expect(refresh).toHaveBeenCalledTimes(1)
    resolve?.()
    await settle()
    onActivity()
    await settle()
    expect(refresh).toHaveBeenCalledTimes(2)
  })

  it("counts the interval from the call, also when it failed", async () => {
    const refresh = vi.fn<() => Promise<unknown>>(async () => {
      throw new TypeError("Failed to fetch")
    })
    const { onActivity, advance } = tracker(refresh)
    onActivity()
    await settle()
    // The rejection is caught, and the next keystroke does not retry.
    advance(MINUTE)
    onActivity()
    await settle()
    expect(refresh).toHaveBeenCalledTimes(1)
    advance(REFRESH_INTERVAL_MS)
    onActivity()
    await settle()
    expect(refresh).toHaveBeenCalledTimes(2)
  })

  it("calls again after the interval when the auth provider's call never got an answer", async () => {
    // A stalled connection: the request never settles by itself. The auth
    // provider gives up after its timeout (shortened here), so the open
    // call closes and the next activity after the interval calls again.
    const { fetch, signals } = hangingFetch()
    const provider = createAuthProvider({
      apiBaseUrl: "/api/admin",
      fetch,
      refreshTimeoutMs: 20,
      sessionExpiry: createSessionExpiry(),
    })
    const { onActivity, refresh, advance } = tracker(
      vi.fn<() => Promise<unknown>>(() => provider.refreshSession())
    )
    onActivity()
    expect(fetch).toHaveBeenCalledTimes(1)
    await vi.waitFor(() => {
      expect(signals[0]?.aborted).toBe(true)
    })
    await settle()

    // Within the interval, no call, although the first one has closed.
    advance(MINUTE)
    onActivity()
    expect(fetch).toHaveBeenCalledTimes(1)

    advance(REFRESH_INTERVAL_MS)
    onActivity()
    expect(refresh).toHaveBeenCalledTimes(2)
    expect(fetch).toHaveBeenCalledTimes(2)
    await vi.waitFor(() => {
      expect(signals[1]?.aborted).toBe(true)
    })
  })

  it("survives a refresh that throws before it returns a promise", async () => {
    const refresh = vi.fn<() => Promise<unknown>>(() => {
      throw new Error("synchronous")
    })
    const { onActivity, advance } = tracker(refresh)
    expect(() => onActivity()).not.toThrow()
    await settle()
    advance(REFRESH_INTERVAL_MS)
    onActivity()
    await settle()
    expect(refresh).toHaveBeenCalledTimes(2)
  })

  it("refuses an interval that would refresh on every keystroke or never", () => {
    for (const intervalMs of [
      0,
      -1,
      0.5,
      Number.NaN,
      Number.POSITIVE_INFINITY,
    ]) {
      expect(() =>
        createActivityRefresh({
          refresh: async () => undefined,
          now: () => 0,
          intervalMs,
        })
      ).toThrow(RangeError)
    }
    expect(() =>
      createActivityRefresh({
        refresh: async () => undefined,
        now: () => 0,
        intervalMs: 1,
      })
    ).not.toThrow()
  })

  it("calls exactly when the interval has passed since the previous call, for any activity", async () => {
    // A model of the rule over any sequence of gaps between activities:
    // a call on the first activity, and on each one at least the interval
    // after the previous call.
    await fc.assert(
      fc.asyncProperty(
        fc.array(fc.integer({ min: 0, max: 25 * MINUTE }), { maxLength: 40 }),
        fc.integer({ min: 1, max: 20 * MINUTE }),
        async (gaps, intervalMs) => {
          let time = 0
          const refresh = vi.fn<() => Promise<unknown>>(async () => undefined)
          const onActivity = createActivityRefresh({
            refresh,
            now: () => time,
            intervalMs,
          })
          let expected = 0
          let lastCall: number | undefined
          for (const gap of gaps) {
            time += gap
            if (lastCall === undefined || time - lastCall >= intervalMs) {
              expected += 1
              lastCall = time
            }
            onActivity()
            // Each call settles before the next activity.
            await settle()
          }
          expect(refresh).toHaveBeenCalledTimes(expected)
        }
      )
    )
  })
})

describe("listenForActivity", () => {
  it("counts typing and clicking, not moving or scrolling", () => {
    expect(ACTIVITY_EVENTS).toEqual(["keydown", "pointerdown"])
    const target = new EventTarget()
    const onActivity = vi.fn()
    listenForActivity(target, onActivity)
    for (const type of ["pointermove", "scroll", "keyup", "focus"]) {
      target.dispatchEvent(new Event(type))
    }
    expect(onActivity).not.toHaveBeenCalled()
    target.dispatchEvent(new Event("keydown"))
    target.dispatchEvent(new Event("pointerdown"))
    expect(onActivity).toHaveBeenCalledTimes(2)
  })

  it("sees an event that a handler below stops", () => {
    const button = document.createElement("button")
    document.body.append(button)
    button.addEventListener("pointerdown", (event) => event.stopPropagation())
    const onActivity = vi.fn()
    const stop = listenForActivity(window, onActivity)
    try {
      button.dispatchEvent(new Event("pointerdown", { bubbles: true }))
      expect(onActivity).toHaveBeenCalledTimes(1)
    } finally {
      stop()
      button.remove()
    }
  })

  it("stops listening when told to", () => {
    const target = new EventTarget()
    const onActivity = vi.fn()
    const stop = listenForActivity(target, onActivity)
    stop()
    target.dispatchEvent(new Event("keydown"))
    target.dispatchEvent(new Event("pointerdown"))
    expect(onActivity).not.toHaveBeenCalled()
  })
})
