import { resolve } from "node:path"
import tailwindcss from "@tailwindcss/vite"
import react from "@vitejs/plugin-react"
import { defineConfig, loadEnv } from "vite"

import {
  LICENCE_NOTICES_BANNER,
  LICENCE_NOTICES_FILE,
  licenceNotices,
} from "./scripts/licence-notices.mjs"
import { DEV_BACKEND, devProxy } from "./scripts/dev-proxy.mjs"
import { parseReportMissingKeys } from "./src/i18n/report-missing-keys.ts"
import { includeClipboardRecorder } from "./src/recorder/recorder-flag.ts"
import { parseSourceCommit } from "./src/source-link/source-commit.ts"

/**
 * Vite configuration of the backend3 admin UI.
 *
 * - `base: "/admin/"`: backend3 serves the built files under `/admin`, and
 *   hashed assets under `/admin/assets/` (ADR 0004 §3). Vite also prefixes the
 *   public-directory URLs in `index.html` with this base.
 * - Development: the Vite dev server serves the UI and proxies the API to a
 *   locally running backend3, so the browser sees one origin and the
 *   `SameSite=Strict` session cookie behaves as in production (ADR 0004 §4).
 *   The proxy gives the requests of the dev server's own pages backend3's
 *   origin, which backend3's `Origin` check of cookie-authenticated writes
 *   expects; every other origin passes unchanged and is refused there
 *   (`scripts/dev-proxy.mjs`, ADR 0044).
 * - `VITE_SOURCE_COMMIT` is validated here, at build time, so a malformed value
 *   fails the image build instead of shipping a broken AGPL source link
 *   (ADR 0012 §4). The Containerfile passes the full commit SHA; an empty or
 *   unset value means a local build and links to `master`.
 * - `VITE_REPORT_MISSING_KEYS` is validated here too: `1` makes a production
 *   build report every missing Hungarian string, which the CI E2E build uses
 *   (ADR 0021 §7); `0` or unset keeps production silent. A typo fails the
 *   build instead of silently switching the check off.
 * - `VITE_CLIPBOARD_RECORDER` decides whether the development-only clipboard
 *   recorder exists (`src/recorder/recorder-flag.ts`): always on the dev
 *   server, in a build only when it is `1`. The decision reaches the app as
 *   the constant `__CLIPBOARD_RECORDER__` rather than as the raw
 *   `import.meta.env` value, because it depends on the Vite command as well
 *   as on the variable, and is validated here first. In a build the constant
 *   is a literal, so the bundler drops the branch of `main.tsx` that it
 *   guards, with its dynamic import (`src/recorder/production-bundle.test.ts`).
 * - `build.license` writes the licence notices of the bundled packages to
 *   `dist/licenses.txt`, which backend3 serves at `/admin/licenses.txt`
 *   (ADR 0038). Vite lists the packages of the JavaScript chunks; the
 *   `licenceNotices` plugin adds those that arrive through a stylesheet
 *   `@import` or as an asset, then the licence files of the code copied into
 *   `src/`, and every chunk starts with a comment that names the file
 *   (`scripts/licence-notices.mjs`).
 */
export default defineConfig(({ command, mode }) => {
  // loadEnv reads `.env*` files and the process environment; only `VITE_`
  // variables are ever exposed to client code.
  const env = loadEnv(mode, import.meta.dirname, "VITE_")
  parseSourceCommit(env.VITE_SOURCE_COMMIT)
  parseReportMissingKeys(env.VITE_REPORT_MISSING_KEYS)
  const clipboardRecorder = includeClipboardRecorder({
    build: command === "build",
    flag: env.VITE_CLIPBOARD_RECORDER,
  })

  return {
    base: "/admin/",
    plugins: [
      react(),
      tailwindcss(),
      licenceNotices({ fileName: LICENCE_NOTICES_FILE }),
    ],
    build: {
      // Vite's default is `.vite/license.md`; the dist root keeps the URL
      // plain, and `.txt` makes every browser show the file (ADR 0038).
      license: { fileName: LICENCE_NOTICES_FILE },
      rolldownOptions: {
        // Added after minification, so the comment survives it.
        output: { postBanner: LICENCE_NOTICES_BANNER },
      },
    },
    define: {
      __CLIPBOARD_RECORDER__: JSON.stringify(clipboardRecorder),
    },
    resolve: {
      alias: {
        "@": resolve(import.meta.dirname, "./src"),
      },
    },
    server: {
      // The admin API (ADR 0004 §4), with the `Origin` of the dev server's
      // own pages translated into backend3's (ADR 0044), and the
      // root-relative `/storage/...` media URLs (ADR 0015, ADR 0023 §6),
      // which previews in the media library need on the same origin.
      proxy: devProxy(DEV_BACKEND),
    },
  }
})
