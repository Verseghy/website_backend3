import { resolve } from "node:path"
import tailwindcss from "@tailwindcss/vite"
import react from "@vitejs/plugin-react"
import { defineConfig, loadEnv } from "vite"

import { parseReportMissingKeys } from "./src/i18n/report-missing-keys.ts"
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
 * - `VITE_SOURCE_COMMIT` is validated here, at build time, so a malformed value
 *   fails the image build instead of shipping a broken AGPL source link
 *   (ADR 0012 §4). The Containerfile passes the full commit SHA; an empty or
 *   unset value means a local build and links to `master`.
 * - `VITE_REPORT_MISSING_KEYS` is validated here too: `1` makes a production
 *   build report every missing Hungarian string, which the CI E2E build uses
 *   (ADR 0021 §7); `0` or unset keeps production silent. A typo fails the
 *   build instead of silently switching the check off.
 */
export default defineConfig(({ mode }) => {
  // loadEnv reads `.env*` files and the process environment; only `VITE_`
  // variables are ever exposed to client code.
  const env = loadEnv(mode, import.meta.dirname, "VITE_")
  parseSourceCommit(env.VITE_SOURCE_COMMIT)
  parseReportMissingKeys(env.VITE_REPORT_MISSING_KEYS)

  return {
    base: "/admin/",
    plugins: [react(), tailwindcss()],
    resolve: {
      alias: {
        "@": resolve(import.meta.dirname, "./src"),
      },
    },
    server: {
      proxy: {
        // The admin API (ADR 0004 §4).
        "/api": "http://localhost:5150",
        // Media URLs are root-relative `/storage/...` (ADR 0015, ADR 0023 §6),
        // so previews in the media library need the same origin in development.
        "/storage": "http://localhost:5150",
      },
    },
  }
})
