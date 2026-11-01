/**
 * backend3's layout: the kit's `Layout` with the footer below the page
 * content, and without the kit's user menu for now.
 *
 * The footer goes into the kit layout's `footer` slot, after its `<main>`,
 * not into the page content: a `<footer>` inside `<main>` is a generic element
 * for assistive technology (HTML-AAM), and the page footer must be the
 * contentinfo landmark. The slot is a local change in the kit's `layout.tsx`
 * (see `src/components/admin/LOCAL_CHANGES.md`).
 *
 * The user menu stays out (`userMenu={null}`, the kit layout's other local
 * change). The kit shows it as soon as an auth provider exists, and its only
 * entry is „Kijelentkezés”. Signing out needs the server's
 * `POST /api/admin/auth/logout`, which ends the session; without it the
 * cookie would stay valid behind a sign-out that only looked done. The logout
 * slice adds the endpoint and brings the menu back with it.
 */
import type { CoreLayoutProps } from "ra-core"

import { Layout } from "@/components/admin/layout"

import { AppFooter } from "./AppFooter"

/**
 * Builds the layout component for one build's source commit.
 *
 * ra-core expects a component type for `layout`, so the commit is bound here
 * once at start-up instead of being read from the environment inside.
 *
 * @param sourceCommit - the build's raw `VITE_SOURCE_COMMIT`.
 * @returns a layout component for `<Admin layout>`.
 */
export function createAppLayout(sourceCommit: string | undefined) {
  function AppLayout({ children }: CoreLayoutProps) {
    return (
      <Layout
        footer={<AppFooter sourceCommit={sourceCommit} />}
        userMenu={null}
      >
        {children}
      </Layout>
    )
  }
  return AppLayout
}
