/**
 * The data-provider handlers of every resource the admin registers, by
 * ra-core resource name: what `main.tsx` passes to `createDataProvider` as
 * its `resources`.
 *
 * Why one function: the provider needs every resource's handlers in one
 * table, and `main.tsx`, the tests of the whole admin and the resource
 * coverage check (`resource-coverage.test.tsx`) must all build the same
 * one. The handlers take no `fetch` of their own: they send through the
 * provider's `request`, which calls the `fetch` that `main.tsx` gives the
 * provider and holds a `401` for the in-place sign-in (admin/CLAUDE.md,
 * "Data provider"). A slice that adds a resource adds its line here, next
 * to its `<Resource>` in `src/App.tsx`.
 */
import type { ResourceHandlers } from "@/data-provider/data-provider"

import { LABELS_RESOURCE, createLabelHandlers } from "./labels/label-handlers"

/**
 * Builds the handlers of every resource.
 *
 * @returns the handlers by resource name.
 */
export function createResourceHandlers(): Readonly<
  Record<string, ResourceHandlers>
> {
  return {
    [LABELS_RESOURCE]: createLabelHandlers(),
  }
}
