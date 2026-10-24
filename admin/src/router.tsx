/**
 * Browser-history routing for the admin (ADR 0004 §3).
 *
 * Why this exists: outside a router, ra-core 5.15.5 creates a hash router
 * (`RouterWrapper` in ra-core's `routing/adapters/reactRouterProvider.tsx`
 * returns its `InternalRouter`, built with `createHashRouter`, only when
 * `useInRouterContext()` is false). With hash URLs the server never sees a
 * client route, and backend3's `index.html` fallback would go untested. So the
 * admin root is mounted inside a react-router 7 browser router, which makes
 * `useInRouterContext()` true and ra-core renders its children unwrapped.
 *
 * The `/admin` prefix is set exactly once, as `basename="/admin"` on the admin
 * root. The router itself gets no basename, so links are not prefixed twice:
 * ra-core's `AdminRouter` stores the basename in its `BasenameContext` only
 * when it is inside a router, and `useCreatePath`, `useRedirect` and the auth
 * hooks build their paths from it. ra-core's `LinkBase` does not: it is a
 * plain router link. So a link to a fixed admin path is written as
 * `` to={`${useBasename()}/about`} ``, never `to="/about"`; the copied kit
 * components were patched accordingly (`src/components/admin/LOCAL_CHANGES.md`)
 * and `router.test.ts` rejects any new root-absolute `to`.
 *
 * A data router (`createBrowserRouter`) rather than `<BrowserRouter>` is used
 * because ra-core blocks navigation away from unsaved forms only under a data
 * router (`useCanBlock` checks react-router's data-router contexts).
 */
import type { ReactNode } from "react"
import { createBrowserRouter, type RouteObject } from "react-router-dom"

/** The path the admin is served under, without a trailing slash. */
export const ADMIN_BASENAME = "/admin"

/**
 * The route table: one splat route that hands everything under `/admin` to
 * ra-core, which declares its own resource routes relative to it.
 *
 * @param adminElement - the admin root (`<AdminApp>`).
 * @returns the route objects, shared by the browser router and test routers.
 */
export function adminRoutes(adminElement: ReactNode): RouteObject[] {
  return [{ path: `${ADMIN_BASENAME}/*`, element: adminElement }]
}

/**
 * Creates the browser-history data router of the running app.
 *
 * @param adminElement - the admin root (`<AdminApp>`).
 * @returns a react-router 7 data router for `<RouterProvider>`.
 */
export function createAdminRouter(adminElement: ReactNode) {
  return createBrowserRouter(adminRoutes(adminElement))
}
