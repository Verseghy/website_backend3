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
  /**
   * `1` puts the development-only clipboard recorder into a build (the CI E2E
   * build sets it); `0` or unset leaves it out. The dev server always has it.
   * The app never reads this value: it reads `__CLIPBOARD_RECORDER__`.
   */
  readonly VITE_CLIPBOARD_RECORDER?: string
}

interface ImportMeta {
  readonly env: ImportMetaEnv
}

/**
 * Whether this build includes the clipboard recorder: a constant that
 * `vite.config.ts` defines from `VITE_CLIPBOARD_RECORDER`. Only `main.tsx`
 * reads it; the unit tests do not define it.
 */
declare const __CLIPBOARD_RECORDER__: boolean
