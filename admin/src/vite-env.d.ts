/// <reference types="vite/client" />

/**
 * Build-time variables the admin reads. Only `main.tsx` reads them; it passes
 * the values down as explicit props.
 */
interface ImportMetaEnv {
  /**
   * Full commit SHA of the source the image was built from, passed by the
   * Containerfile (`ARG SOURCE_COMMIT`); empty or unset in local builds.
   * `vite.config.ts` rejects any other value (ADR 0012 §4).
   */
  readonly VITE_SOURCE_COMMIT?: string
  /**
   * `1` makes a production build report every missing Hungarian string (the
   * CI E2E build sets it, ADR 0021 §7); `0` or unset keeps production silent.
   * `vite.config.ts` rejects any other value.
   */
  readonly VITE_REPORT_MISSING_KEYS?: string
}

interface ImportMeta {
  readonly env: ImportMetaEnv
}
