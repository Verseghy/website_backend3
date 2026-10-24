/**
 * backend3's layout: the kit's `Layout` with the footer below the page
 * content.
 *
 * The footer goes into the kit layout's `footer` slot, after its `<main>`,
 * not into the page content: a `<footer>` inside `<main>` is a generic element
 * for assistive technology (HTML-AAM), and the page footer must be the
 * contentinfo landmark. The slot is the one local change this needs in the
 * kit's `layout.tsx` (see `src/components/admin/LOCAL_CHANGES.md`).
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
      <Layout footer={<AppFooter sourceCommit={sourceCommit} />}>
        {children}
      </Layout>
    )
  }
  return AppLayout
}
