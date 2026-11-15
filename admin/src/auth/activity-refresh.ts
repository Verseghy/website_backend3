/**
 * The activity tracker of the sliding session refresh (ADR 0006 §1,
 * "Sliding refresh, explicit").
 *
 * Why it exists: the session cookie lasts two hours, and the server renews
 * it by itself only on a request (the passive refresh, with less than an
 * hour left). An editor who types a long post for two hours without saving
 * sends no request, so the session would end under the open form. The
 * admin therefore calls `POST /api/admin/auth/refresh` when the user types
 * or clicks, and at least {@link REFRESH_INTERVAL_MS} after its previous
 * call, so the session ends between 1 h 50 min and 2 h after the last
 * activity, or at the server's absolute limit.
 *
 * The rules this module carries:
 *
 * - The first activity after the tracker starts refreshes at once: nothing
 *   says how much of the session is left after a page load, and one extra
 *   call is cheaper than a session that ends 50 minutes after the last
 *   keystroke.
 * - The interval counts from the previous call, not its answer, so a server
 *   that refuses or fails is asked again only after another interval, and
 *   no second call starts while one is open. The refresh must therefore
 *   settle: the auth provider's `refreshSession` gives up on an answer that
 *   has not come after `REFRESH_TIMEOUT_MS` (30 seconds), so a stalled
 *   request is closed long before the interval has passed, and the first
 *   activity after it calls again: one stalled request never stops the
 *   refresh for good.
 * - A failed or refused refresh changes nothing here: the tracker never
 *   signs out. A session that has ended meets its `401` on the next real
 *   request, where the auth provider's `checkError` handles it.
 * - Only typing (`keydown`) and pressing a pointer (`pointerdown`) count,
 *   in the capture phase, so a component that stops an event's propagation
 *   cannot hide it. Moving the mouse or scrolling is not activity
 *   (ADR 0006 §1 names typing and clicking).
 *
 * Nothing here reads `window` or the clock: the event target and `now`
 * arrive as arguments (`admin/CLAUDE.md`, "Configuration is explicit"), so
 * the tests drive the tracker with a stub target and a fake time.
 */

/** The least time between two refresh calls: 10 minutes (ADR 0006 §1). */
export const REFRESH_INTERVAL_MS = 10 * 60 * 1000

/** The events that count as user activity: typing and clicking. */
export const ACTIVITY_EVENTS = ["keydown", "pointerdown"] as const

/** The part of `window` that the tracker listens on. */
export type ActivityEventTarget = Pick<
  EventTarget,
  "addEventListener" | "removeEventListener"
>

/** Options of {@link createActivityRefresh}. */
export interface ActivityRefreshOptions {
  /**
   * Calls the explicit refresh; the auth provider's `refreshSession`. Its
   * outcome is not read, and a rejection is caught. It must settle in
   * bounded time, because no further call starts while it is open;
   * `refreshSession` times out after `REFRESH_TIMEOUT_MS`.
   */
  readonly refresh: () => Promise<unknown>
  /** The current time in milliseconds; `Date.now` in the app. */
  readonly now: () => number
  /**
   * The least time between two calls, in milliseconds; at least 1.
   * {@link REFRESH_INTERVAL_MS} unless a test says otherwise.
   */
  readonly intervalMs?: number
}

/**
 * Builds the tracker's activity handler.
 *
 * @param options - see {@link ActivityRefreshOptions}.
 * @returns a function to call on every user activity; it calls `refresh`
 *   on the first activity and then whenever at least the interval has
 *   passed since its previous call and that call has settled.
 * @throws {RangeError} when `intervalMs` is not a finite number of at least
 *   1, which would refresh on every keystroke or never.
 */
export function createActivityRefresh(
  options: ActivityRefreshOptions
): () => void {
  const intervalMs = options.intervalMs ?? REFRESH_INTERVAL_MS
  if (!Number.isFinite(intervalMs) || intervalMs < 1) {
    throw new RangeError(
      `intervalMs must be a finite number of milliseconds, at least 1; got ${String(intervalMs)}. The admin uses REFRESH_INTERVAL_MS (10 minutes, ADR 0006 §1).`
    )
  }
  const { refresh, now } = options
  let lastCall: number | undefined
  let open = false
  return () => {
    const time = now()
    if (open || (lastCall !== undefined && time - lastCall < intervalMs)) {
      return
    }
    lastCall = time
    open = true
    // The refresh is best effort (see the module documentation): its
    // outcome, a rejection and even a synchronous throw are dropped, and
    // only the open call is tracked. It starts here, synchronously, so the
    // request leaves with the event that caused it.
    let pending: Promise<unknown>
    try {
      pending = Promise.resolve(refresh())
    } catch {
      pending = Promise.resolve()
    }
    void pending
      .catch(() => undefined)
      .finally(() => {
        open = false
      })
  }
}

/**
 * Calls `onActivity` for every {@link ACTIVITY_EVENTS} event on `target`.
 *
 * @param target - what to listen on; the app passes `window`.
 * @param onActivity - the handler, such as {@link createActivityRefresh}'s.
 * @returns a function that removes the listeners again.
 */
export function listenForActivity(
  target: ActivityEventTarget,
  onActivity: () => void
): () => void {
  const listener = () => onActivity()
  // Capture: a handler that stops propagation further down cannot hide the
  // activity. Passive: the listener never prevents the default action.
  const options: AddEventListenerOptions = { capture: true, passive: true }
  for (const type of ACTIVITY_EVENTS) {
    target.addEventListener(type, listener, options)
  }
  return () => {
    for (const type of ACTIVITY_EVENTS) {
      target.removeEventListener(type, listener, { capture: true })
    }
  }
}
