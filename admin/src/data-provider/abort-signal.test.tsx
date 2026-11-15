/**
 * ra-core's query signal through backend3's data provider: the provider
 * declares `supportAbortSignal`, so ra-core hands react-query's signal to a
 * query handler, and a query that waits for the in-place sign-in stops
 * waiting when the screen that asked unmounts (ADR 0006, "Follow-up work";
 * `src/auth/session-expiry.ts`). Without the flag ra-core passes no signal,
 * and the dialog would stay open for a screen that is gone.
 *
 * Everything is real but the network and the auth calls: ra-core's
 * `useGetList` and its query client, the data provider with a slice-style
 * handler, the retry queue and the dialog.
 */
import { render, screen, waitFor } from "@testing-library/react"
import {
  CoreAdminContext,
  TestMemoryRouter,
  useGetList,
  type DataProvider,
} from "ra-core"
import { describe, expect, it, vi } from "vitest"

import { refusalError } from "@/api/error-envelope"
import { SessionExpiryDialog } from "@/auth/SessionExpiryDialog"
import { createSessionExpiry, type SessionExpiry } from "@/auth/session-expiry"
import { createI18nProvider } from "@/i18n/provider"

import { createDataProvider } from "./data-provider"

/** The probe screen: one list query, and nothing else. */
function ProbeList() {
  const { data } = useGetList("probes")
  return <p data-testid="probe-list">{data?.length ?? "…"}</p>
}

/** Props of {@link Admin}. */
interface AdminProps {
  /** The data provider under test. */
  readonly dataProvider: DataProvider
  /** The retry queue that it sends through. */
  readonly sessionExpiry: SessionExpiry
  /** The dialog's sign-out stand-in. */
  readonly signOut: () => Promise<void>
  /** Whether the probe screen is shown; `false` is a navigation away. */
  readonly screenShown: boolean
}

/**
 * The admin's context with the probe screen and the dialog beside it, as
 * the layout places it.
 *
 * @param props - see {@link AdminProps}.
 */
function Admin({
  dataProvider,
  sessionExpiry,
  signOut,
  screenShown,
}: AdminProps) {
  return (
    <TestMemoryRouter initialEntries={["/probes"]}>
      <CoreAdminContext
        dataProvider={dataProvider}
        i18nProvider={createI18nProvider({ reportMissingKeys: true })}
      >
        {screenShown && <ProbeList />}
        <SessionExpiryDialog
          expiry={sessionExpiry}
          account={() => undefined}
          resume={vi.fn()}
          signOut={signOut}
        />
      </CoreAdminContext>
    </TestMemoryRouter>
  )
}

describe("a query whose screen unmounts while it waits for the in-place sign-in", () => {
  it("gets ra-core's signal, leaves the queue when the screen unmounts, and closes the dialog", async () => {
    const signals: (AbortSignal | undefined)[] = []
    // The session has ended: every request answers 401 unauthorized.
    const fetch = vi.fn<typeof globalThis.fetch>(
      async () =>
        new Response(
          JSON.stringify({
            error: "unauthorized",
            description: "No valid session",
            field: null,
            params: {},
          }),
          { status: 401, headers: { "Content-Type": "application/json" } }
        )
    )
    const sessionExpiry = createSessionExpiry()
    const dataProvider = createDataProvider({
      apiBaseUrl: "/api/admin",
      fetch,
      sessionExpiry,
      resources: {
        probes: {
          // What a slice's list handler does: pass ra-core's signal on.
          async getList(params, context) {
            signals.push(params.signal)
            const response = await context.request("/probes", {
              signal: params.signal,
            })
            const body: unknown = await response.json()
            if (!response.ok) {
              throw refusalError(response.status, body)
            }
            return body as { data: { id: number }[]; total: number }
          },
        },
      },
    })
    const signOut = vi.fn(async () => undefined)
    const view = render(
      <Admin
        dataProvider={dataProvider}
        sessionExpiry={sessionExpiry}
        signOut={signOut}
        screenShown
      />
    )

    // The query met its 401 and waits for the sign-in.
    expect(await screen.findByRole("dialog")).toBeInTheDocument()
    expect(signals).toHaveLength(1)
    const [signal] = signals
    expect(signal).toBeInstanceOf(AbortSignal)
    expect(signal?.aborted).toBe(false)

    // ra-core logs a rejected data-provider call outside production unless
    // it is an `AbortError` that is `instanceof DOMException`. Under jsdom
    // the abort's reason comes from another realm than the global
    // `DOMException`, so ra-core logs it here, as a browser never does;
    // this test owns the console spy and accepts exactly that.
    const consoleError = vi
      .spyOn(console, "error")
      .mockImplementation(() => undefined)

    // The screen goes away: react-query cancels its query.
    view.rerender(
      <Admin
        dataProvider={dataProvider}
        sessionExpiry={sessionExpiry}
        signOut={signOut}
        screenShown={false}
      />
    )
    await waitFor(() => expect(signal?.aborted).toBe(true))
    await waitFor(() => expect(sessionExpiry.isWaiting()).toBe(false))
    await waitFor(() =>
      expect(screen.queryByRole("dialog")).not.toBeInTheDocument()
    )
    // Leaving was not a sign-in or a give-up, and nothing was sent again.
    expect(sessionExpiry.round()).toBe(0)
    expect(fetch).toHaveBeenCalledTimes(1)
    expect(signOut).not.toHaveBeenCalled()
    for (const call of consoleError.mock.calls) {
      expect(call).toEqual([expect.objectContaining({ name: "AbortError" })])
    }
  })
})
