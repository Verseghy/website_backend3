/**
 * The composition root of the admin UI.
 *
 * This is the only module that reads the build environment
 * (`import.meta.env`) or touches `window` directly. Everything below it gets
 * its configuration as explicit arguments, so it can be tested with stubs.
 */
import { lazy, StrictMode } from "react"
import { createRoot } from "react-dom/client"
import { Route, RouterProvider } from "react-router-dom"
import { I18nContextProvider } from "ra-core"

import "./index.css"
import { AdminApp } from "@/App"
import { createDataProvider } from "@/data-provider/data-provider"
import { createI18nProvider } from "@/i18n/provider"
import { shouldReportMissingKeys } from "@/i18n/report-missing-keys"
import { createAdminRouter } from "@/router"
import { ChunkLoadReloadPrompt, reloadPage } from "@/update-prompt/UpdatePrompt"

const root = document.getElementById("root")
if (root === null) {
  throw new Error(
    'admin/index.html must contain <div id="root"></div>; the admin mounts there.'
  )
}

// Silent in production builds, except the CI E2E build, which sets
// VITE_REPORT_MISSING_KEYS=1 so that any fallback to English fails the run
// (ADR 0021 §7).
const i18nProvider = createI18nProvider({
  reportMissingKeys: shouldReportMissingKeys({
    production: import.meta.env.PROD,
    flag: import.meta.env.VITE_REPORT_MISSING_KEYS,
  }),
})
const dataProvider = createDataProvider({
  apiBaseUrl: "/api/admin",
  resources: {},
})

/**
 * Builds the route of the development-only clipboard recorder
 * (`/admin/clipboard-recorder`, editor.md §13.1). The page is lazy, so even a
 * build that has it loads its code only when the route opens; the kit's
 * layout supplies the Suspense fallback and the chunk-load reload prompt.
 *
 * @returns the recorder's `<Route>`, for `<AdminApp developmentRoutes>`.
 */
function clipboardRecorderRoute() {
  const ClipboardRecorder = lazy(async () => {
    const recorder = await import("@/recorder/ClipboardRecorderPage")
    return {
      default: function ClipboardRecorderRoute() {
        return (
          <recorder.ClipboardRecorderPage
            userAgent={window.navigator.userAgent}
            now={() => new Date()}
            saveFile={(fileName, contents) =>
              recorder.saveTextFile(window, fileName, contents)
            }
          />
        )
      },
    }
  })
  return <Route path="/clipboard-recorder" element={<ClipboardRecorder />} />
}

// `__CLIPBOARD_RECORDER__` is a constant that vite.config.ts defines: `false`
// in every build unless VITE_CLIPBOARD_RECORDER=1. The bundler then drops this
// expression and `clipboardRecorderRoute`, which only it calls, and with them
// the dynamic import, so the default build emits no recorder code at all
// (src/recorder/production-bundle.test.ts checks a real build).
const developmentRoutes = __CLIPBOARD_RECORDER__
  ? clipboardRecorderRoute()
  : undefined

const router = createAdminRouter(
  <AdminApp
    dataProvider={dataProvider}
    i18nProvider={i18nProvider}
    sourceCommit={import.meta.env.VITE_SOURCE_COMMIT}
    developmentRoutes={developmentRoutes}
  />
)

document.title = i18nProvider.translate("app.title")

createRoot(root).render(
  <StrictMode>
    <RouterProvider router={router} />
    {/* Outside <Admin>, so it needs its own i18n context. */}
    <I18nContextProvider value={i18nProvider}>
      <ChunkLoadReloadPrompt target={window} onReload={reloadPage} />
    </I18nContextProvider>
  </StrictMode>
)
