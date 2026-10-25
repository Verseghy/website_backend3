/**
 * The admin root: the kit's `<Admin>` configured for backend3.
 *
 * Every dependency (providers, source commit) arrives as a prop from the
 * composition root (`main.tsx`), so tests mount the same component with stubs
 * and a memory router.
 */
import { useMemo, type ReactNode } from "react"
import { CustomRoutes, type DataProvider, type I18nProvider } from "ra-core"
import { Route } from "react-router-dom"

import { AboutPage } from "@/about/AboutPage"
import { Admin } from "@/components/admin"
import { Dashboard } from "@/dashboard/Dashboard"
import { ABOUT_PATH } from "@/layout/AppFooter"
import { createAppLayout } from "@/layout/AppLayout"
import { ADMIN_BASENAME } from "@/router"

/** Props of {@link AdminApp}. */
export interface AdminAppProps {
  /** The data provider (`createDataProvider`). */
  readonly dataProvider: DataProvider
  /** The Hungarian i18n provider (`createI18nProvider`). */
  readonly i18nProvider: I18nProvider
  /** The build's raw `VITE_SOURCE_COMMIT`, for the source links. */
  readonly sourceCommit: string | undefined
  /**
   * Extra `<Route>` elements that only development builds register, inside
   * the layout (the clipboard recorder). `main.tsx` builds them behind a
   * build-time constant, so a production build passes nothing and carries
   * none of their code.
   */
  readonly developmentRoutes?: ReactNode
}

/**
 * Renders the admin. It must be mounted inside a react-router 7 router (see
 * `router.tsx`); otherwise ra-core would fall back to a hash router.
 *
 * @param props - see {@link AdminAppProps}.
 */
export function AdminApp({
  dataProvider,
  i18nProvider,
  sourceCommit,
  developmentRoutes,
}: AdminAppProps) {
  const layout = useMemo(() => createAppLayout(sourceCommit), [sourceCommit])
  return (
    <Admin
      basename={ADMIN_BASENAME}
      dataProvider={dataProvider}
      i18nProvider={i18nProvider}
      layout={layout}
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
