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
import { QueryClient } from "@tanstack/react-query"
import { I18nContextProvider } from "ra-core"

import "./index.css"
import { AdminApp } from "@/App"
import { invalidateAccessChecks } from "@/auth/access-checks"
import { createAuthProvider } from "@/auth/auth-provider"
import { createSessionExpiry } from "@/auth/session-expiry"
import { createDataProvider } from "@/data-provider/data-provider"
import { createI18nProvider } from "@/i18n/provider"
import { shouldReportMissingKeys } from "@/i18n/report-missing-keys"
import { createResourceHandlers } from "@/resources/handlers"
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
// The browser's fetch, bound to window: called unbound, it throws
// "Illegal invocation" in Chromium. Both providers call it.
const browserFetch = window.fetch.bind(window)
// One retry queue for the in-place sign-in (ADR 0006, "Follow-up work"):
// both providers send through it, and the app's layout mounts the dialog
// that answers it. A provider with a queue of its own would hand its 401s
// back at once, because no dialog is attached to that queue.
const sessionExpiry = createSessionExpiry()
const dataProvider = createDataProvider({
  apiBaseUrl: "/api/admin",
  // Every resource's handlers send through the provider's `request`, which
  // calls `browserFetch` and holds a 401 for the in-place sign-in.
  resources: createResourceHandlers(),
  fetch: browserFetch,
  sessionExpiry,
})
// ra-core's react-query client, made here so the auth provider can ask it to
// re-ask its cached access questions when the account's access changed
// (ADR 0007 §7, `src/auth/access-checks.ts`). The defaults are the ones
// ra-core would choose itself.
const queryClient = new QueryClient()
const authProvider = createAuthProvider({
  apiBaseUrl: "/api/admin",
  fetch: browserFetch,
  sessionExpiry,
  onAccessChange: () => {
    void invalidateAccessChecks(queryClient)
  },
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
    authProvider={authProvider}
    i18nProvider={i18nProvider}
    queryClient={queryClient}
    sourceCommit={import.meta.env.VITE_SOURCE_COMMIT}
    developmentRoutes={developmentRoutes}
    // The session's activity tracker (ADR 0006 §1) listens on the window and
    // reads the browser's clock; one function for the app's lifetime, so
    // the tracker is never restarted by a new identity.
    activityTarget={window}
    now={Date.now}
    sessionExpiry={sessionExpiry}
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
