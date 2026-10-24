/**
 * The about page („Névjegy”, `/admin/about`).
 *
 * It states the licence and links to the running source, which makes the
 * AGPL-3.0 §13 offer explicit (ADR 0012 §4). It is also the admin's first
 * client route: ra-core renders its bare `Ready` splash instead of the layout
 * while an admin has no resources and no custom routes, and the E2E routing
 * checks need a real deep link (ADR 0004 §3).
 */
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card"
import { useT } from "@/i18n/keys"
import { SourceLink } from "@/source-link/SourceLink"
import { parseSourceCommit } from "@/source-link/source-commit"

/** Props of {@link AboutPage}. */
export interface AboutPageProps {
  /** The build's raw `VITE_SOURCE_COMMIT`; see `SourceLink`. */
  readonly sourceCommit: string | undefined
}

/**
 * Renders the about page.
 *
 * @param props - see {@link AboutPageProps}.
 */
export function AboutPage({ sourceCommit }: AboutPageProps) {
  const t = useT()
  const commit = parseSourceCommit(sourceCommit)
  return (
    <div className="flex flex-col gap-4 py-4" data-testid="about-page">
      <Card>
        <CardHeader>
          <CardTitle>
            <h1>{t("ra.message.about")}</h1>
          </CardTitle>
        </CardHeader>
        <CardContent className="flex flex-col gap-2">
          <p>{t("app.about.intro")}</p>
          <p>{t("app.about.licence")}</p>
          <p>
            {commit === null
              ? t("app.about.local_build")
              : t("app.about.commit", { commit })}
          </p>
          <p>
            <SourceLink
              className="text-primary underline-offset-4 hover:underline"
              sourceCommit={sourceCommit}
            >
              {t("app.about.source")}
            </SourceLink>
          </p>
        </CardContent>
      </Card>
    </div>
  )
}
