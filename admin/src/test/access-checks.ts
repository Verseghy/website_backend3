/**
 * A wait for the tests of components that ask `authProvider.canAccess`
 * (ADR 0007 §7): until every question asked so far has its answer and
 * React has rendered it.
 *
 * Why a helper of its own: an access wrapper renders nothing both while
 * its question is open and after a no, so a test that finds no button has
 * proved nothing unless the question was answered first. The tests of the
 * wrappers (`src/auth/access-buttons.test.tsx`) and of the kit's pages
 * that use them (`src/auth/kit-pages-access.test.tsx`) wait here before
 * they look, for a button that must be there and for one that must not.
 */
import type { QueryClient } from "@tanstack/react-query"
import { act, waitFor } from "@testing-library/react"
import { expect } from "vitest"

import { CAN_ACCESS_QUERY_KEY } from "@/auth/access-checks"

/**
 * Waits until every `canAccess` question that ra-core cached in
 * `queryClient` is answered or failed, and the answers are rendered.
 *
 * @param queryClient - the client that ra-core caches the answers in.
 * @param atLeast - how many questions must have been asked; at least one
 *   by default, so the wait never passes before the first is asked.
 */
export async function accessChecksAnswered(
  queryClient: QueryClient,
  atLeast = 1
): Promise<void> {
  await waitFor(() => {
    expect(
      queryClient.getQueryCache().findAll({ queryKey: CAN_ACCESS_QUERY_KEY })
        .length
    ).toBeGreaterThanOrEqual(atLeast)
    // Fetching, not pending: a question that a component asks with
    // `enabled: false` stays pending without ever being asked.
    expect(queryClient.isFetching({ queryKey: CAN_ACCESS_QUERY_KEY })).toBe(0)
  })
  // react-query hands the results to the components on its own timer.
  await act(async () => {
    await new Promise((resolve) => setTimeout(resolve, 0))
  })
}
