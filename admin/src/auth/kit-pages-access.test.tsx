/**
 * The kit's own pages offer only what the account may do (ADR 0007 §7):
 * the list's „Új” button, the empty list page's invitation with its „Új”,
 * the edit page's „Törlés”, the show page's „Szerkesztés” and the bulk
 * toolbar's „Törlés” are backend3's access wrappers
 * (`./access-buttons.tsx`, listed in the kit's `LOCAL_CHANGES.md`), so a
 * screen that keeps the kit's default actions shows them only when
 * `canAccess` allows them, a record-level rule included.
 *
 * The test mounts the kit's `<Admin>` with backend3's layout and data
 * provider, a probe resource whose list, edit and show pages are the
 * kit's own with their default actions (or, for the empty page, the kit's
 * `Empty` as its list page, which a screen renders for a list without
 * records), and an auth provider that answers with `decideAccess`
 * (`./access.ts`) over an invented account and a probe record-level rule
 * (a published record needs `publish posts`). No screen of backend3
 * registers the probe, so its `resources.probe-posts.*` labels are the
 * only missing Hungarian keys the test lets through.
 */
import { QueryClient } from "@tanstack/react-query"
import { render, screen, waitFor, within } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import { Resource, type AuthProvider, type RaRecord } from "ra-core"
import type { ComponentType } from "react"
import { RouterProvider, createMemoryRouter } from "react-router-dom"
import { describe, expect, it, vi } from "vitest"

import type { AccessAction } from "@/bindings/AccessAction"
import {
  Admin,
  DataTable,
  Empty,
  Edit,
  List,
  Show,
  SimpleForm,
  SimpleShowLayout,
  TextField,
  TextInput,
} from "@/components/admin"
import { createDataProvider } from "@/data-provider/data-provider"
import { createI18nProvider } from "@/i18n/provider"
import { hu } from "@/i18n/hu"
import { createAppLayout } from "@/layout/AppLayout"
import { ADMIN_BASENAME, adminRoutes } from "@/router"
import { accessChecksAnswered } from "@/test/access-checks"

import { decideAccess, type AccessSubject, type RecordRule } from "./access"
import { createSessionExpiry } from "./session-expiry"

/** The probe resource; no screen of backend3 registers it. */
const RESOURCE = "probe-posts"

/** The probe records: two drafts and a published one. */
const RECORDS: readonly RaRecord[] = [
  { id: 1, title: "Első", published: false },
  { id: 2, title: "Második", published: false },
  { id: 3, title: "Harmadik", published: true },
]

/** A published record needs `publish posts`, whatever the action. */
const PROBE_RULES: Readonly<Record<string, RecordRule>> = {
  [RESOURCE]: (_action, record, account) =>
    record.published !== true || account.permissions.includes("publish posts"),
}

/**
 * An invented account that holds `edit posts` and `actions` on the probe.
 *
 * @param actions - the access map's actions on {@link RESOURCE}.
 * @returns the account as `/auth/me` would answer it.
 */
function account(actions: readonly AccessAction[]): AccessSubject {
  return {
    pid: "0b7c4a52-7e3d-4c55-9a7f-4b3c2d1e0f9a",
    permissions: ["edit posts"],
    is_admin: false,
    access: { [RESOURCE]: [...actions] },
  }
}

/** The list screen: the kit's `List` with its default actions. */
function ProbeList() {
  return (
    <List>
      <DataTable>
        <DataTable.Col source="title" />
      </DataTable>
    </List>
  )
}

/** The edit screen: the kit's `Edit` with its default actions. */
function ProbeEdit() {
  return (
    <Edit>
      <SimpleForm>
        <TextInput source="title" />
      </SimpleForm>
    </Edit>
  )
}

/** The show screen: the kit's `Show` with its default actions. */
function ProbeShow() {
  return (
    <Show>
      <SimpleShowLayout>
        <TextField source="title" />
      </SimpleShowLayout>
    </Show>
  )
}

/**
 * Mounts the admin with the probe resource at `path`, signed in as
 * `subject`.
 *
 * @param subject - the signed-in account.
 * @param path - the page to open, below `/admin`.
 * @param list - the resource's list page; the kit's list by default.
 * @returns the query client the answers are cached in, and the missing
 *   keys reported.
 */
function mount(
  subject: AccessSubject,
  path: string,
  list: ComponentType = ProbeList
) {
  const authProvider: AuthProvider = {
    login: async () => undefined,
    logout: async () => undefined,
    checkAuth: async () => undefined,
    checkError: async () => undefined,
    canAccess: async (question) => decideAccess(subject, question, PROBE_RULES),
  }
  const missingKeys: string[] = []
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  })
  // One retry queue for the data provider and the layout's in-place
  // sign-in, as main.tsx has it. The probe handlers answer from RECORDS
  // without a request, so nothing waits and the dialog never opens.
  const sessionExpiry = createSessionExpiry()
  const router = createMemoryRouter(
    adminRoutes(
      <Admin
        basename={ADMIN_BASENAME}
        authProvider={authProvider}
        dataProvider={createDataProvider({
          apiBaseUrl: "/api/admin",
          resources: {
            [RESOURCE]: {
              getList: async () => ({ data: [...RECORDS], total: 3 }),
              getOne: async ({ id }) => {
                const data = RECORDS.find(
                  (record) => String(record.id) === String(id)
                )
                if (data === undefined) {
                  throw new Error(`no probe record ${String(id)}`)
                }
                return { data }
              },
            },
          },
          // The probe handlers never call `request`, so nothing is fetched.
          fetch: vi.fn<typeof globalThis.fetch>(),
          sessionExpiry,
        })}
        i18nProvider={createI18nProvider({
          reportMissingKeys: true,
          onMissingKey: (key) => {
            missingKeys.push(key)
          },
        })}
        queryClient={queryClient}
        // The session's activity tracker (ADR 0006 §1) listens on a target
        // that nothing dispatches on, so it never calls the refresh. The
        // stub auth provider has no session account, and no request waits,
        // so the in-place sign-in is never asked to resume.
        layout={createAppLayout(
          undefined,
          {
            target: new EventTarget(),
            now: () => 0,
            refresh: () => Promise.resolve(),
          },
          {
            expiry: sessionExpiry,
            account: () => undefined,
            resume: () =>
              Promise.reject(
                new Error("no request waits for the in-place sign-in here")
              ),
          }
        )}
        dashboard={() => <div data-testid="dashboard" />}
        disableTelemetry
      >
        <Resource
          name={RESOURCE}
          list={list}
          create={ProbeEdit}
          edit={ProbeEdit}
          show={ProbeShow}
        />
      </Admin>
    ),
    { initialEntries: [`${ADMIN_BASENAME}${path}`] }
  )
  render(<RouterProvider router={router} />)
  return { queryClient, missingKeys }
}

/**
 * The missing keys that are not the probe resource's own labels.
 *
 * @param missingKeys - what the i18n provider reported.
 * @returns the keys that a real screen would lack.
 */
function realMissingKeys(missingKeys: readonly string[]): string[] {
  return missingKeys.filter((key) => !key.startsWith(`resources.${RESOURCE}.`))
}

describe("the kit's list page", () => {
  it("offers „Új” only to an account that may create", async () => {
    const writer = mount(
      account(["list", "show", "create", "edit"]),
      `/${RESOURCE}`
    )
    expect(
      await screen.findByRole("link", { name: hu.ra.action.create })
    ).toHaveAttribute("href", `${ADMIN_BASENAME}/${RESOURCE}/create`)
    expect(realMissingKeys(writer.missingKeys)).toEqual([])
  })

  it("lists without „Új” for an account that may only list", async () => {
    const { queryClient } = mount(account(["list", "show"]), `/${RESOURCE}`)
    expect(await screen.findByText("Első")).toBeInTheDocument()
    await accessChecksAnswered(queryClient)
    expect(screen.queryByRole("link", { name: hu.ra.action.create })).toBeNull()
  })
})

describe("the kit's bulk toolbar", () => {
  /**
   * Ticks the row of `title` in the list.
   *
   * @param title - the record's title.
   */
  async function tick(title: string) {
    const row = (await screen.findByText(title)).closest("tr")
    expect(row).not.toBeNull()
    await userEvent.click(within(row as HTMLElement).getByRole("checkbox"))
  }

  it("offers „Törlés” for selected records the account may delete, and hides it when one is refused", async () => {
    mount(account(["list", "show", "edit", "delete"]), `/${RESOURCE}`)
    await tick("Első")
    expect(
      await screen.findByRole("button", { name: hu.ra.action.delete })
    ).toBeEnabled()
    // The published record needs `publish posts`, which the account lacks.
    await tick("Harmadik")
    await waitFor(() => {
      expect(
        screen.queryByRole("button", { name: hu.ra.action.delete })
      ).toBeNull()
    })
    // The rest of the toolbar stays: its export beside the list's own.
    expect(
      screen.getAllByRole("button", { name: hu.ra.action.export })
    ).toHaveLength(2)
  })

  it("has no „Törlés” for an account that may not delete", async () => {
    const { queryClient } = mount(account(["list", "show"]), `/${RESOURCE}`)
    await tick("Második")
    // The toolbar is open: its export beside the list's own.
    await waitFor(() => {
      expect(
        screen.getAllByRole("button", { name: hu.ra.action.export })
      ).toHaveLength(2)
    })
    await accessChecksAnswered(queryClient)
    expect(
      screen.queryByRole("button", { name: hu.ra.action.delete })
    ).toBeNull()
  })
})

describe("the kit's edit page", () => {
  it("offers „Törlés” to an account that may delete the record", async () => {
    const writer = mount(
      account(["list", "show", "edit", "delete"]),
      `/${RESOURCE}/1`
    )
    expect(
      await screen.findByRole("button", { name: hu.ra.action.delete })
    ).toBeEnabled()
    expect(realMissingKeys(writer.missingKeys)).toEqual([])
  })

  it("has no „Törlés” for an account that may only edit", async () => {
    const { queryClient } = mount(
      account(["list", "show", "edit"]),
      `/${RESOURCE}/1`
    )
    expect(await screen.findByDisplayValue("Első")).toBeInTheDocument()
    await accessChecksAnswered(queryClient)
    expect(
      screen.queryByRole("button", { name: hu.ra.action.delete })
    ).toBeNull()
  })
})

describe("the kit's show page", () => {
  it("offers „Szerkesztés” only where the account may edit the record", async () => {
    mount(account(["list", "show", "edit"]), `/${RESOURCE}/2/show`)
    expect(
      await screen.findByRole("link", { name: hu.ra.action.edit })
    ).toHaveAttribute("href", `${ADMIN_BASENAME}/${RESOURCE}/2`)
  })

  it("has no „Szerkesztés” for a published record of an account that does not publish", async () => {
    const { queryClient } = mount(
      account(["list", "show", "edit"]),
      `/${RESOURCE}/3/show`
    )
    expect(await screen.findAllByText("Harmadik")).not.toHaveLength(0)
    await accessChecksAnswered(queryClient)
    expect(screen.queryByRole("link", { name: hu.ra.action.edit })).toBeNull()
  })

  it("has no „Szerkesztés” for an account that may only read", async () => {
    const { queryClient } = mount(
      account(["list", "show"]),
      `/${RESOURCE}/2/show`
    )
    expect(await screen.findAllByText("Második")).not.toHaveLength(0)
    await accessChecksAnswered(queryClient)
    expect(screen.queryByRole("link", { name: hu.ra.action.edit })).toBeNull()
  })
})

describe("the kit's empty list page", () => {
  /**
   * Finds the empty page's heading, which every account sees.
   *
   * @returns the heading, once the page has rendered.
   */
  async function emptyHeading() {
    // `ra.page.empty` names the resource first; its probe label is not
    // Hungarian, so the sentence is found by its fixed end.
    const sentenceEnd = hu.ra.page.empty.replace("%{name}", "")
    return screen.findByText((text) => text.endsWith(sentenceEnd))
  }

  it("invites an account that may create, with „Új”", async () => {
    const writer = mount(
      account(["list", "show", "create", "edit"]),
      `/${RESOURCE}`,
      Empty
    )
    expect(await emptyHeading()).toBeInTheDocument()
    expect(await screen.findByText(hu.ra.page.invite)).toBeInTheDocument()
    expect(
      await screen.findByRole("link", { name: hu.ra.action.create })
    ).toHaveAttribute("href", `${ADMIN_BASENAME}/${RESOURCE}/create`)
    expect(realMissingKeys(writer.missingKeys)).toEqual([])
  })

  it("neither invites nor offers „Új” to an account that may only list", async () => {
    const { queryClient } = mount(
      account(["list", "show"]),
      `/${RESOURCE}`,
      Empty
    )
    expect(await emptyHeading()).toBeInTheDocument()
    await accessChecksAnswered(queryClient)
    expect(screen.queryByText(hu.ra.page.invite)).toBeNull()
    expect(screen.queryByRole("link", { name: hu.ra.action.create })).toBeNull()
  })
})
