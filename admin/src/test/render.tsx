/**
 * Render helpers for unit tests: the Hungarian i18n provider (reporting
 * missing keys, as in development) and, when asked, ra-core's memory router.
 */
import type { ReactElement } from "react"
import { render, type RenderResult } from "@testing-library/react"
import { I18nContextProvider, TestMemoryRouter } from "ra-core"

import { createI18nProvider } from "@/i18n/provider"

/** Options of {@link renderHungarian}. */
export interface RenderHungarianOptions {
  /** Wrap the element in ra-core's `TestMemoryRouter` at this entry. */
  readonly route?: string
}

/**
 * Renders `ui` inside the admin's i18n context. A key missing from `hu.ts`
 * reaches `console.error`, which the test setup turns into a failure.
 *
 * @param ui - the element under test.
 * @param options - see {@link RenderHungarianOptions}.
 * @returns Testing Library's render result.
 */
export function renderHungarian(
  ui: ReactElement,
  options: RenderHungarianOptions = {}
): RenderResult {
  const i18nProvider = createI18nProvider({ reportMissingKeys: true })
  const wrapped = (
    <I18nContextProvider value={i18nProvider}>{ui}</I18nContextProvider>
  )
  return render(
    options.route === undefined ? (
      wrapped
    ) : (
      <TestMemoryRouter initialEntries={[options.route]}>
        {wrapped}
      </TestMemoryRouter>
    )
  )
}
