/**
 * The dashboard placeholder.
 *
 * ADR 0019 gives the dashboard usage statistics and permission-gated widgets;
 * they arrive in a later slice. Until then this screen is the admin's landing
 * page and the target of the E2E routing checks.
 */
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card"
import { useT } from "@/i18n/keys"

/** The admin's landing page. */
export function Dashboard() {
  const t = useT()
  return (
    <div className="flex flex-col gap-4 py-4" data-testid="dashboard">
      <Card>
        <CardHeader>
          <CardTitle>
            <h1>{t("dashboard.welcome")}</h1>
          </CardTitle>
        </CardHeader>
        <CardContent>
          <p>{t("dashboard.placeholder")}</p>
        </CardContent>
      </Card>
    </div>
  )
}
