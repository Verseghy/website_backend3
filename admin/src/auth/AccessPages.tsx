/**
 * The two pages ra-core sends a refused page to (ADR 0007 §7): „Hozzáférés
 * megtagadva” at `/admin/access-denied` and „Hitelesítési hiba” at
 * `/admin/authentication-error`.
 *
 * Why they exist: every resource page asks `authProvider.canAccess` before
 * it renders (ra-core's `useRequireAccess`, which the list, show, edit and
 * create controllers call). A "no" sends the user to `/access-denied`, and
 * a question that could not be answered (the provider could not read
 * `/auth/me`) to `/authentication-error`. ra-core renders nothing at
 * either path unless `<Admin>` is given a page for it, so a bookmark to a
 * screen the account may not open, or an open screen whose resource a
 * `403` refetch has just taken away, would show an empty content area.
 * `src/App.tsx` passes these pages instead. They sit inside the layout, so
 * the sidebar, which lists only what the account may open, stays usable.
 *
 * The texts are ra-core's own keys (`ra.page.*`, `ra.message.*`), which
 * `hu.ts` already carries in Hungarian (ADR 0021 §2); the buttons reuse
 * `ra.page.dashboard` and `ra.action.back`.
 */
import type { ReactNode } from "react"
import { History, House, Lock, CircleAlert } from "lucide-react"
import { LinkBase, useBasename, useNavigate } from "ra-core"

import { Button, buttonVariants } from "@/components/ui/button"
import { useT, type MessageKey } from "@/i18n/keys"
import { cn } from "@/lib/utils"

/** Props of {@link RefusalPage}. */
interface RefusalPageProps {
  /** The `data-testid` of the page, for the tests. */
  readonly testId: string
  /** The icon above the heading. */
  readonly icon: ReactNode
  /** The heading's key. */
  readonly title: MessageKey
  /** The explanation's key. */
  readonly message: MessageKey
  /** The buttons below the explanation. */
  readonly children: ReactNode
}

/**
 * The layout both pages share, in the style of the kit's not-found page.
 *
 * @param props - see {@link RefusalPageProps}.
 */
function RefusalPage({
  testId,
  icon,
  title,
  message,
  children,
}: RefusalPageProps) {
  const t = useT()
  return (
    <div
      className="flex min-h-[50vh] flex-1 flex-col items-center justify-center gap-2 text-center"
      data-testid={testId}
    >
      {icon}
      <h1 className="text-2xl font-semibold">{t(title)}</h1>
      <p className="max-w-xl text-muted-foreground">{t(message)}</p>
      <div className="mt-3 flex flex-wrap justify-center gap-2">{children}</div>
    </div>
  )
}

/**
 * The link back to the dashboard, which is no resource and always open.
 *
 * @param props - `variant`: the button style.
 */
function DashboardLink({
  variant,
}: {
  readonly variant: "default" | "outline"
}) {
  const t = useT()
  // ra-core's LinkBase adds no basename (`src/router.tsx`).
  const basename = useBasename()
  return (
    <LinkBase className={cn(buttonVariants({ variant }))} to={`${basename}/`}>
      <House />
      {t("ra.page.dashboard")}
    </LinkBase>
  )
}

/**
 * „Hozzáférés megtagadva”: the account may not open the page it asked for.
 *
 * It links to the dashboard only. "Back" would return to the refused page,
 * which ra-core would refuse again at once (it pushes `/access-denied`
 * onto the history), so the button would seem to do nothing.
 */
export function AccessDeniedPage() {
  return (
    <RefusalPage
      testId="access-denied-page"
      icon={<Lock className="h-16 w-16 text-muted-foreground" aria-hidden />}
      title="ra.page.access_denied"
      message="ra.message.access_denied"
    >
      <DashboardLink variant="default" />
    </RefusalPage>
  )
}

/**
 * „Hitelesítési hiba”: whether the account may open the page could not be
 * checked, because `/auth/me` could not be read.
 *
 * „Vissza” returns to the page, which asks again, so it doubles as a retry
 * once the server answers; the dashboard link is there for a visitor who
 * arrived here directly, with nothing to go back to.
 */
export function AuthenticationErrorPage() {
  const t = useT()
  const navigate = useNavigate()
  return (
    <RefusalPage
      testId="authentication-error-page"
      icon={
        <CircleAlert className="h-16 w-16 text-muted-foreground" aria-hidden />
      }
      title="ra.page.authentication_error"
      message="ra.message.authentication_error"
    >
      <Button
        onClick={() => {
          void navigate(-1)
        }}
      >
        <History />
        {t("ra.action.back")}
      </Button>
      <DashboardLink variant="outline" />
    </RefusalPage>
  )
}
