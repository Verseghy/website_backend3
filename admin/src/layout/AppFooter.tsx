/**
 * The footer under every admin page: the about page and the source link that
 * AGPL-3.0 §13 requires (ADR 0012 §4).
 */
import { LinkBase, useBasename } from "ra-core"

import { useT } from "@/i18n/keys"
import { SourceLink } from "@/source-link/SourceLink"

/** The client route of the about page, relative to the admin basename. */
export const ABOUT_PATH = "/about"

/** Props of {@link AppFooter}. */
export interface AppFooterProps {
  /** The build's raw `VITE_SOURCE_COMMIT`; see `SourceLink`. */
  readonly sourceCommit: string | undefined
}

/**
 * Renders the footer. It must be rendered inside the admin: ra-core's
 * `LinkBase` is a plain router link and does not add the admin basename, so
 * the about link prefixes `useBasename()` itself (see `router.tsx`).
 *
 * @param props - see {@link AppFooterProps}.
 */
export function AppFooter({ sourceCommit }: AppFooterProps) {
  const t = useT()
  const basename = useBasename()
  const linkClass = "underline-offset-4 hover:underline"
  return (
    <footer className="mt-auto flex justify-center gap-4 py-4 text-xs text-muted-foreground">
      <LinkBase className={linkClass} to={`${basename}${ABOUT_PATH}`}>
        {t("app.footer.about")}
      </LinkBase>
      <SourceLink className={linkClass} sourceCommit={sourceCommit}>
        {t("app.footer.source_code")}
      </SourceLink>
    </footer>
  )
}
