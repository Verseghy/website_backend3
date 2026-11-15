/**
 * The „Címkék” list (parity LBL-01; SHR-02, SHR-03) mounted as `main.tsx`
 * mounts the admin: `AdminApp` on a memory router built from the same
 * route table, the real auth provider, and the data provider with the
 * handlers of `src/resources/handlers.ts`, all over one `fetch` stub that
 * plays `/api/admin/auth/me` and `/api/admin/labels`.
 *
 * The stub answers pages of invented labels in a fixed order, whatever the
 * sort: ordering is the server's (`tests/requests/admin_labels.rs`, and the
 * Playwright suite `e2e/labels.spec.ts` against backend3). These tests check
 * what the screen shows and what it asks for. Every Hungarian key the screen
 * looks up must exist: the test setup fails a test on a missing one.
 */
import { QueryClient } from "@tanstack/react-query"
import { render, screen, waitFor, within } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import { RouterProvider, createMemoryRouter } from "react-router-dom"
import { beforeEach, describe, expect, it, vi } from "vitest"

import { AdminApp } from "@/App"
import { createAuthProvider } from "@/auth/auth-provider"
import { createSessionExpiry } from "@/auth/session-expiry"
import type { LabelResponse } from "@/bindings/LabelResponse"
import type { MeResponse } from "@/bindings/MeResponse"
import { createDataProvider } from "@/data-provider/data-provider"
import { hu } from "@/i18n/hu"
import { createI18nProvider } from "@/i18n/provider"
import { createResourceHandlers } from "@/resources/handlers"
import { adminRoutes } from "@/router"
import { accessChecksAnswered } from "@/test/access-checks"

/** 30 invented labels: more than one page of 25. */
const LABELS: readonly LabelResponse[] = Array.from(
  { length: 30 },
  (_, index) => ({
    id: index + 1,
    name: `Próbacímke ${String(index + 1).padStart(2, "0")}`,
    color: index % 2 === 0 ? "#1E88E5" : "#E53935",
  })
)

/** An invented account that may list and read labels. */
const ME: MeResponse = {
  pid: "0b7c4a52-7e3d-4c55-9a7f-4b3c2d1e0f9a",
  name: "Próba Szerkesztő",
  email: "proba@example.test",
  scope: "admin",
  roles: [],
  permissions: ["edit labels"],
  is_admin: false,
  access: { labels: ["list", "show"] },
}

/**
 * A JSON answer.
 *
 * @param status - the HTTP status.
 * @param body - the body.
 * @returns a `Response`.
 */
function json(status: number, body: unknown): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { "Content-Type": "application/json" },
  })
}

/**
 * The admin API as the screen sees it: `/auth/me` answers `me`, and the
 * label list answers the page that `page` and `perPage` ask for, from
 * {@link LABELS}, with the total.
 */
class FakeAdminApi {
  /** The account `/auth/me` answers. */
  me: MeResponse

  /** The `fetch` stub both providers call. */
  readonly fetch = vi.fn<typeof globalThis.fetch>(async (input) => {
    const url = new URL(String(input), "https://backend3.example")
    if (url.pathname === "/api/admin/auth/me") {
      return json(200, this.me)
    }
    if (url.pathname === "/api/admin/labels") {
      const page = Number(url.searchParams.get("page") ?? "1")
      const perPage = Number(url.searchParams.get("perPage") ?? "25")
      return json(200, {
        data: LABELS.slice((page - 1) * perPage, page * perPage),
        total: LABELS.length,
      })
    }
    return json(404, {
      error: "not_found",
      description: "d",
      field: null,
      params: {},
    })
  })

  /**
   * @param me - the account `/auth/me` answers.
   */
  constructor(me: MeResponse) {
    this.me = me
  }

  /**
   * The query strings of the label list's requests so far, in order.
   *
   * @returns each request's query, without the `?`.
   */
  listQueries(): string[] {
    return this.fetch.mock.calls
      .map(([input]) => new URL(String(input), "https://backend3.example"))
      .filter((url) => url.pathname === "/api/admin/labels")
      .map((url) => url.search.replace(/^\?/, ""))
  }
}

/**
 * Mounts the admin at `entry`, signed in as `me`.
 *
 * @param entry - the initial path.
 * @param me - the signed-in account.
 * @returns the fake API, the memory router and the query client that
 *   ra-core caches its answers in.
 */
function mountAdmin(entry: string, me: MeResponse = ME) {
  const api = new FakeAdminApi(me)
  const queryClient = new QueryClient()
  // One retry queue for both providers and the app, as main.tsx has it.
  const sessionExpiry = createSessionExpiry()
  const router = createMemoryRouter(
    adminRoutes(
      <AdminApp
        dataProvider={createDataProvider({
          apiBaseUrl: "/api/admin",
          resources: createResourceHandlers(),
          fetch: api.fetch,
          sessionExpiry,
        })}
        authProvider={createAuthProvider({
          apiBaseUrl: "/api/admin",
          fetch: api.fetch,
          sessionExpiry,
        })}
        i18nProvider={createI18nProvider({ reportMissingKeys: true })}
        queryClient={queryClient}
        sourceCommit={undefined}
        // A target that no test dispatches on, so the session's activity
        // tracker stays quiet.
        activityTarget={new EventTarget()}
        now={() => 0}
        sessionExpiry={sessionExpiry}
      />
    ),
    { initialEntries: [entry] }
  )
  render(<RouterProvider router={router} />)
  return { api, router, queryClient }
}

/** The default first request: page 1 of 25, by name, A to Zs. */
const FIRST_PAGE = "page=1&perPage=25&sort=name&order=ASC"

beforeEach(() => {
  // The kit's <Admin> keeps ra-core's list parameters in localStorage
  // (localStorageStore), which every test of this file shares: a sort that
  // one test chose would otherwise open the next one's list.
  localStorage.clear()
})

describe("the „Címkék” list", () => {
  it("lists the labels by name, 25 to a page, with a swatch of each colour", async () => {
    const { api } = mountAdmin("/admin/labels")
    expect(await screen.findByText("Próbacímke 01")).toBeInTheDocument()

    expect(screen.getByRole("heading", { name: "Címkék" })).toBeInTheDocument()
    expect(new Set(api.listQueries())).toEqual(new Set([FIRST_PAGE]))

    const table = screen.getByRole("table")
    // The header row and 25 label rows.
    expect(within(table).getAllByRole("row")).toHaveLength(26)
    expect(
      within(table).getByRole("columnheader", {
        name: hu.resources.labels.fields.name,
      })
    ).toBeInTheDocument()
    expect(
      within(table).getByRole("columnheader", {
        name: hu.resources.labels.fields.color,
      })
    ).toBeInTheDocument()
    expect(within(table).queryByText("Próbacímke 26")).toBeNull()

    // The first label's colour: a swatch filled with it, and its text.
    const firstRow = within(table).getAllByRole("row")[1]
    const swatch = within(firstRow).getByTestId("label-color-swatch")
    expect(swatch).toHaveStyle({ backgroundColor: "#1E88E5" })
    expect(swatch).toHaveAttribute("aria-hidden", "true")
    expect(within(firstRow).getByText("#1E88E5")).toBeInTheDocument()

    // The pagination: 25 rows on offer, the range of the first page.
    expect(screen.getByRole("combobox")).toHaveTextContent("25")
    expect(screen.getByText("1–25 / 30")).toBeInTheDocument()
  })

  it("offers no export, no „Új” and no row selection", async () => {
    const { router } = mountAdmin("/admin/labels")
    const cell = await screen.findByText("Próbacímke 01")
    expect(
      screen.queryByRole("button", { name: hu.ra.action.export })
    ).toBeNull()
    expect(screen.queryByRole("link", { name: hu.ra.action.create })).toBeNull()
    expect(
      within(screen.getByRole("table")).queryAllByRole("checkbox")
    ).toEqual([])
    // A row leads nowhere yet: the forms come with their own slice.
    await userEvent.click(cell)
    expect(router.state.location.pathname).toBe("/admin/labels")
  })

  it("sorts by a column header, and a second click turns the order round", async () => {
    const { api, router } = mountAdmin("/admin/labels")
    await screen.findByText("Próbacímke 01")

    await userEvent.click(
      screen.getByRole("button", { name: hu.resources.labels.fields.color })
    )
    await waitFor(() => {
      expect(api.listQueries()).toContain(
        "page=1&perPage=25&sort=color&order=ASC"
      )
    })

    await userEvent.click(
      screen.getByRole("button", { name: hu.resources.labels.fields.color })
    )
    await waitFor(() => {
      expect(api.listQueries()).toContain(
        "page=1&perPage=25&sort=color&order=DESC"
      )
    })

    // Another column sorts A to Zs again. That page may come from ra-core's
    // cache, so the list's address says what it shows.
    await userEvent.click(
      screen.getByRole("button", { name: hu.resources.labels.fields.name })
    )
    await waitFor(() => {
      const shown = new URLSearchParams(router.state.location.search)
      expect([
        shown.get("sort"),
        shown.get("order"),
        shown.get("page"),
      ]).toEqual(["name", "ASC", "1"])
    })
    // Every request asked for one of the two sortable fields of the
    // server's allow-list (domain::labels::LIST), never for `id` alone.
    for (const query of api.listQueries()) {
      expect(new URLSearchParams(query).get("sort")).toMatch(/^(name|color)$/)
    }
  })

  it("pages forward with the next-page link, keeping the order", async () => {
    const { api } = mountAdmin("/admin/labels")
    await screen.findByText("Próbacímke 01")

    // The kit renders its page links as Base UI buttons: the anchor carries
    // `role="button"`, so it is found by that role.
    await userEvent.click(
      screen.getByRole("button", { name: hu.ra.navigation.next })
    )
    expect(await screen.findByText("Próbacímke 26")).toBeInTheDocument()
    expect(api.listQueries()).toContain("page=2&perPage=25&sort=name&order=ASC")
    expect(screen.getByText("26–30 / 30")).toBeInTheDocument()
    expect(screen.queryByText("Próbacímke 01")).toBeNull()
  })

  it("opens the page size a URL asks for", async () => {
    const { api } = mountAdmin("/admin/labels?page=1&perPage=10")
    expect(await screen.findByText("Próbacímke 10")).toBeInTheDocument()
    expect(screen.queryByText("Próbacímke 11")).toBeNull()
    expect(api.listQueries()).toContain("page=1&perPage=10&sort=name&order=ASC")
  })
})

describe("the sidebar's „Címkék” entry", () => {
  it("leads to the list for an account the access map lets list labels", async () => {
    const { router } = mountAdmin("/admin")
    await screen.findByTestId("dashboard")
    const sidebar = document.querySelector('[data-sidebar="content"]')
    expect(sidebar).not.toBeNull()
    const link = await within(sidebar as HTMLElement).findByRole("link", {
      name: "Címkék",
    })
    expect(link).toHaveAttribute("href", "/admin/labels")
    await userEvent.click(link)
    expect(await screen.findByText("Próbacímke 01")).toBeInTheDocument()
    expect(router.state.location.pathname).toBe("/admin/labels")
  })

  it("is absent for an account the map does not let list labels", async () => {
    // `edit posts` reads labels only through the post form's picker: the
    // access map offers the screens to `edit labels` alone (ADR 0007 §4).
    const { api, queryClient } = mountAdmin("/admin", {
      ...ME,
      permissions: ["edit posts"],
      access: {},
    })
    await screen.findByTestId("dashboard")
    // A missing entry proves nothing until the sidebar's question about the
    // labels has its answer.
    await accessChecksAnswered(queryClient)
    const sidebar = document.querySelector('[data-sidebar="content"]')
    expect(sidebar).not.toBeNull()
    expect(
      within(sidebar as HTMLElement)
        .queryAllByRole("link")
        .map((link) => link.getAttribute("href"))
    ).toEqual(["/admin/"])
    expect(api.listQueries()).toEqual([])
  })
})
