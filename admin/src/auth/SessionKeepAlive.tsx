/**
 * Keeps the session alive while the user works: the activity tracker of
 * `activity-refresh.ts`, mounted for as long as a signed-in page is shown
 * (ADR 0006 §1, "Sliding refresh, explicit").
 *
 * The layout renders it, and ra-core renders the layout only with a session
 * (`requireAuth` in `App.tsx`), so the tracker starts after the sign-in and
 * stops with the sign-out, when the sign-in page replaces the layout. It
 * renders nothing.
 */
import { useEffect } from "react"

import {
  createActivityRefresh,
  listenForActivity,
  type ActivityEventTarget,
} from "./activity-refresh"

/** What {@link SessionKeepAlive} listens on and calls. */
export interface SessionKeepAliveProps {
  /** Where the activity happens; `main.tsx` passes `window`. */
  readonly target: ActivityEventTarget
  /** The current time in milliseconds; `main.tsx` passes `Date.now`. */
  readonly now: () => number
  /** The explicit refresh; the auth provider's `refreshSession`. */
  readonly refresh: () => Promise<unknown>
}

/**
 * Listens for typing and clicking on `target` and refreshes the session at
 * most every 10 minutes.
 *
 * @param props - see {@link SessionKeepAliveProps}.
 */
export function SessionKeepAlive({
  target,
  now,
  refresh,
}: SessionKeepAliveProps) {
  useEffect(
    () => listenForActivity(target, createActivityRefresh({ refresh, now })),
    [target, now, refresh]
  )
  return null
}
