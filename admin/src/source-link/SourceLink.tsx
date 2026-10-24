/**
 * The link to the source of the running admin: the AGPL-3.0 §13 source offer
 * (ADR 0012 §4).
 *
 * The commit is an explicit prop: the composition root reads
 * `import.meta.env.VITE_SOURCE_COMMIT` once and passes it down, so this
 * component (and its test) never depends on the build environment.
 */
import type { ReactNode } from "react"

import { useT } from "@/i18n/keys"

import { sourceTreeUrl } from "./source-commit"

/** Props of {@link SourceLink}. */
export interface SourceLinkProps {
  /**
   * The raw `VITE_SOURCE_COMMIT` of the build: a full commit SHA, or empty or
   * `undefined` for a local build (which links to `master`).
   */
  readonly sourceCommit: string | undefined
  /** The link text; already translated by the caller. */
  readonly children: ReactNode
  /** Extra classes for the anchor. */
  readonly className?: string
}

/**
 * Renders an anchor to `https://github.com/Verseghy/website_backend3/tree/<commit>`
 * (or `/tree/master` for a local build), opening in a new tab.
 *
 * @param props - see {@link SourceLinkProps}.
 * @throws {InvalidSourceCommitError} when `sourceCommit` is malformed; the
 *   build already rejects such a value, so this only fires in a broken setup.
 */
export function SourceLink({
  sourceCommit,
  children,
  className,
}: SourceLinkProps) {
  const t = useT()
  return (
    <a
      className={className}
      href={sourceTreeUrl(sourceCommit)}
      title={t("app.footer.source_code_title")}
      rel="noopener noreferrer"
      target="_blank"
    >
      {children}
    </a>
  )
}
