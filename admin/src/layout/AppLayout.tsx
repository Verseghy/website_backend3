/**
 * backend3's layout: the kit's `Layout` with the footer below the page
 * content, and with backend3's own user menu in the header.
 *
 * The footer goes into the kit layout's `footer` slot, after its `<main>`,
 * not into the page content: a `<footer>` inside `<main>` is a generic element
 * for assistive technology (HTML-AAM), and the page footer must be the
 * contentinfo landmark. The slot is a local change in the kit's `layout.tsx`
 * (see `src/components/admin/LOCAL_CHANGES.md`).
 *
 * The user menu goes into the layout's `userMenu` slot, the kit layout's
 * other local change, in place of the kit's `UserMenu`: its „Kijelentkezés”
 * must end the session on the server (`POST /api/admin/auth/logout`, ADR 0006
 * §2), and the kit's could not ask for that (`AppUserMenu.tsx` says why).
 *
 * The layout also mounts the session's activity tracker
 * (`SessionKeepAlive`, ADR 0006 §1): ra-core renders the layout only with a
 * session, so the tracker runs exactly while a signed-in page is shown.
 */
import type { CoreLayoutProps } from "ra-core"

import {
  SessionKeepAlive,
  type SessionKeepAliveProps,
} from "@/auth/SessionKeepAlive"
import { Layout } from "@/components/admin/layout"

import { AppFooter } from "./AppFooter"
import { AppUserMenu } from "./AppUserMenu"

/**
 * Builds the layout component for one build's source commit and the
 * session's activity tracker.
 *
 * ra-core expects a component type for `layout`, so both are bound here
 * once at start-up instead of being read from the environment inside.
 *
 * @param sourceCommit - the build's raw `VITE_SOURCE_COMMIT`.
 * @param keepAlive - where the activity tracker listens, its clock and the
 *   refresh it calls. The same values are passed to every render, so the
 *   tracker is not restarted while the layout stays mounted.
 * @returns a layout component for `<Admin layout>`.
 */
export function createAppLayout(
  sourceCommit: string | undefined,
  keepAlive: SessionKeepAliveProps
) {
  function AppLayout({ children }: CoreLayoutProps) {
    return (
      <>
        <SessionKeepAlive
          target={keepAlive.target}
          now={keepAlive.now}
          refresh={keepAlive.refresh}
        />
        <Layout
          footer={<AppFooter sourceCommit={sourceCommit} />}
          userMenu={<AppUserMenu />}
        >
          {children}
        </Layout>
      </>
    )
  }
  return AppLayout
}
