/**
 * Makes ra-core ask its access questions again after the account's access
 * changed (ADR 0007 §7).
 *
 * Why this module exists: ra-core caches every `authProvider.canAccess`
 * answer in its react-query client (`useCanAccess`, `useCanAccessResources`),
 * and a mounted component, such as a sidebar entry, keeps showing the
 * cached answer. When a `403` makes the auth provider read `/auth/me` again
 * and the answer differs, the provider calls its `onAccessChange` option;
 * `main.tsx` passes {@link invalidateAccessChecks} over the query client it
 * gives `<Admin>`, so every cached answer is asked again, and answered at
 * once from the new `/auth/me` answer.
 *
 * The query key is ra-core's: 5.15.5 and 5.15.6, whose `src/auth/` is the
 * same file for file, write `["auth", "canAccess", ...]` for both hooks
 * (`useCanAccess.ts`, `useCanAccessResources.ts`), and react-query matches
 * the prefix. `./access-checks.test.tsx` drives the real hooks, so a
 * ra-core release that renames the key fails it.
 */
import type { QueryClient } from "@tanstack/react-query"

/** The prefix of the query key of every cached `canAccess` answer. */
export const CAN_ACCESS_QUERY_KEY = ["auth", "canAccess"] as const

/**
 * Marks every cached `canAccess` answer stale and asks the mounted ones
 * again.
 *
 * @param queryClient - the query client that `<Admin queryClient>` uses.
 * @returns a promise that settles when the active questions are answered;
 *   react-query reports a failed question on the question itself, so the
 *   promise never rejects for one.
 */
export function invalidateAccessChecks(
  queryClient: QueryClient
): Promise<void> {
  return queryClient.invalidateQueries({ queryKey: CAN_ACCESS_QUERY_KEY })
}
