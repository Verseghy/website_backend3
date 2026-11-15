/**
 * The admin root: the kit's `<Admin>` configured for backend3.
 *
 * Every dependency (providers, source commit) arrives as a prop from the
 * composition root (`main.tsx`), so tests mount the same component with stubs
 * and a memory router.
 *
 * Every page needs a session (`requireAuth`, ADR 0006): before it renders
 * the layout, ra-core asks the auth provider's `checkAuth`, and without a
 * session it sends the visitor to the Hungarian sign-in screen
 * (`/admin/login`), which is the only page outside the session. After the
 * sign-in it returns to the page the visitor asked for.
 *
 * The signed-in pages keep the session alive while the user types or
 * clicks (ADR 0006 §1): the layout mounts the activity tracker, which calls
 * the auth provider's `refreshSession`, with the event target and the clock
 * that `main.tsx` passes.
 *
 * A session that ends under a signed-in page is renewed in place (ADR 0006,
 * "Follow-up work"): the layout mounts the session-expiry dialog over the
 * retry queue that `main.tsx` passes, the same queue that both providers
 * send through, and signs in again with the auth provider's
 * `resumeSession`. The page, and any form on it, stays mounted behind it.
 *
 * The sidebar lists only the resources whose list the account may open: the
 * kit's `app-sidebar.tsx` asks `canAccess` for `list`, which the auth
 * provider answers from the access map of `/auth/me` (ADR 0007 §7). The
 * dashboard and the custom routes (the about page) are not resources and
 * ask nothing. A resource page that the map refuses goes to the Hungarian
 * access-denied page, and one whose question could not be answered to the
 * authentication-error page (`src/auth/AccessPages.tsx`).
 */
import type { QueryClient } from "@tanstack/react-query"
import { useCallback, useMemo, type ReactNode } from "react"
import { CustomRoutes, type DataProvider, type I18nProvider } from "ra-core"
import { Route } from "react-router-dom"

import { AboutPage } from "@/about/AboutPage"
import { AccessDeniedPage, AuthenticationErrorPage } from "@/auth/AccessPages"
import type { ActivityEventTarget } from "@/auth/activity-refresh"
import type { BackendAuthProvider } from "@/auth/auth-provider"
import type { SessionExpiry } from "@/auth/session-expiry"
import { createSignInPage } from "@/auth/SignInPage"
import { Admin } from "@/components/admin"
import { Dashboard } from "@/dashboard/Dashboard"
import { ABOUT_PATH } from "@/layout/AppFooter"
import { createAppLayout } from "@/layout/AppLayout"
import { ADMIN_BASENAME } from "@/router"

/** Props of {@link AdminApp}. */
export interface AdminAppProps {
  /** The data provider (`createDataProvider`). */
  readonly dataProvider: DataProvider
  /**
   * The auth provider (`createAuthProvider`), with the session refresh that
   * the activity tracker calls.
   */
  readonly authProvider: BackendAuthProvider
  /** The Hungarian i18n provider (`createI18nProvider`). */
  readonly i18nProvider: I18nProvider
  /**
   * ra-core's react-query client. `main.tsx` makes it, so that the auth
   * provider's `onAccessChange` can invalidate the cached `canAccess`
   * answers in it (`src/auth/access-checks.ts`).
   */
  readonly queryClient: QueryClient
  /** The build's raw `VITE_SOURCE_COMMIT`, for the source links. */
  readonly sourceCommit: string | undefined
  /**
   * Extra `<Route>` elements that only development builds register, inside
   * the layout (the clipboard recorder). `main.tsx` builds them behind a
   * build-time constant, so a production build passes nothing and carries
   * none of their code.
   */
  readonly developmentRoutes?: ReactNode
  /**
   * Where the activity tracker listens for typing and clicking; `main.tsx`
   * passes `window`, tests a stub.
   */
  readonly activityTarget: ActivityEventTarget
  /** The current time in milliseconds; `main.tsx` passes `Date.now`. */
  readonly now: () => number
  /**
   * The retry queue of the in-place sign-in (`createSessionExpiry`): the
   * same queue that `main.tsx` passes to both providers, so that their
   * `401`s wait for the dialog that the layout mounts.
   */
  readonly sessionExpiry: SessionExpiry
}

/**
 * Renders the admin. It must be mounted inside a react-router 7 router (see
 * `router.tsx`); otherwise ra-core would fall back to a hash router.
 *
 * @param props - see {@link AdminAppProps}.
 */
export function AdminApp({
  dataProvider,
  authProvider,
  i18nProvider,
  queryClient,
  sourceCommit,
  developmentRoutes,
  activityTarget,
  now,
  sessionExpiry,
}: AdminAppProps) {
  const refresh = useCallback(
    () => authProvider.refreshSession(),
    [authProvider]
  )
  const account = useCallback(
    () => authProvider.sessionAccount(),
    [authProvider]
  )
  const resume = useCallback(
    (password: string) => authProvider.resumeSession(password),
    [authProvider]
  )
  const layout = useMemo(
    () =>
      createAppLayout(
        sourceCommit,
        { target: activityTarget, now, refresh },
        { expiry: sessionExpiry, account, resume }
      ),
    [sourceCommit, activityTarget, now, refresh, sessionExpiry, account, resume]
  )
  const signInPage = useMemo(
    () => createSignInPage(sourceCommit),
    [sourceCommit]
  )
  return (
    <Admin
      basename={ADMIN_BASENAME}
      dataProvider={dataProvider}
      authProvider={authProvider}
      i18nProvider={i18nProvider}
      queryClient={queryClient}
      layout={layout}
      loginPage={signInPage}
      // Without it ra-core would render the layout, the custom routes and
      // the not-found screen to a visitor without a session, and check
      // the session only on the dashboard and the resource pages.
      requireAuth
      // Where ra-core sends a page that canAccess refused, or could not
      // answer; without them both paths render nothing
      // (`src/auth/AccessPages.tsx`).
      accessDenied={AccessDeniedPage}
      authenticationError={AuthenticationErrorPage}
      dashboard={Dashboard}
      title={i18nProvider.translate("app.title")}
      // The kit otherwise loads a tracking image from marmelab's server in
      // production builds (admin.tsx, AdminUI): that would leak the admin's
      // host name and violate the admin CSP's `img-src 'self'` (ADR 0023 §6).
      disableTelemetry
    >
      <CustomRoutes>
        <Route
          path={ABOUT_PATH}
          element={<AboutPage sourceCommit={sourceCommit} />}
        />
        {developmentRoutes}
      </CustomRoutes>
    </Admin>
  )
}
