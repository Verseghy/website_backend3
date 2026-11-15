/**
 * `ResourceList`, the list page of every resource: 25 rows to a page, the
 * list contract's page sizes (exactly 10, 25, 50 and 100 on offer), the
 * filter button and the access-gated „Új”
 * when the list has them, and never the kit's export (parity SHR-02).
 *
 * The test mounts the kit's `<Admin>` with backend3's layout and a probe
 * resource whose list is a `ResourceList`, over a data provider whose
 * `getList` is a spy and an auth provider that answers `canAccess` from a
 * list of allowed actions. No screen of backend3 registers the probe, so
 * its `resources.probe-items.*` labels are the only missing Hungarian keys
 * the test lets through. The last test reads the sources of
 * `src/resources/` for a screen that uses the kit's `List` instead.
 */
import { readdirSync, readFileSync } from "node:fs"
import { join, relative, resolve } from "node:path"

import { QueryClient } from "@tanstack/react-query"
import type { ReactNode } from "react"
import { render, screen, waitFor } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import {
  Resource,
  type AuthProvider,
  type GetListParams,
  type RaRecord,
} from "ra-core"
import { RouterProvider, createMemoryRouter } from "react-router-dom"
import { describe, expect, it, vi } from "vitest"

import { createSessionExpiry } from "@/auth/session-expiry"
import { Admin, DataTable, TextInput, type ListProps } from "@/components/admin"
import { createDataProvider } from "@/data-provider/data-provider"
import { hu } from "@/i18n/hu"
import { createI18nProvider } from "@/i18n/provider"
import { createAppLayout } from "@/layout/AppLayout"
import { ADMIN_BASENAME, adminRoutes } from "@/router"
import { accessChecksAnswered } from "@/test/access-checks"

import { ResourceList } from "./ResourceList"

/** The probe resource; no screen of backend3 registers it. */
const RESOURCE = "probe-items"

/** The probe's records. */
const RECORDS: readonly RaRecord[] = [
  { id: 1, title: "Első" },
  { id: 2, title: "Második" },
]

/** How {@link mount} sets up the probe's list screen. */
interface MountOptions {
  /** The actions the account may take on the probe. */
  readonly allowed: readonly string[]
  /** Whether the probe has a create page. */
  readonly hasCreate?: boolean
  /** Props passed to the probe's `ResourceList`. */
  readonly listProps?: Partial<ListProps> & {
    readonly filters?: ReactNode[]
  }
}

/**
 * Mounts the admin at the probe's list.
 *
 * @param options - see {@link MountOptions}.
 * @returns the `getList` spy, the query client and the missing keys
 *   reported.
 */
function mount({ allowed, hasCreate = false, listProps = {} }: MountOptions) {
  const getList = vi.fn(async (params: GetListParams) => {
    const perPage = params.pagination?.perPage ?? 0
    return { data: RECORDS.slice(0, perPage), total: RECORDS.length }
  })
  const authProvider: AuthProvider = {
    login: async () => undefined,
    logout: async () => undefined,
    checkAuth: async () => undefined,
    checkError: async () => undefined,
    canAccess: async ({ action }) => allowed.includes(action),
  }
  const missingKeys: string[] = []
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  })
  function ProbeList() {
    return (
      <ResourceList {...listProps}>
        <DataTable rowClick={false} bulkActionButtons={false}>
          <DataTable.Col source="title" />
        </DataTable>
      </ResourceList>
    )
  }
  function ProbeCreate() {
    return <div data-testid="probe-create" />
  }
  // The probe's handler answers without a request, so the provider's
  // `fetch` is never called and no request waits for the in-place sign-in.
  const sessionExpiry = createSessionExpiry()
  const router = createMemoryRouter(
    adminRoutes(
      <Admin
        basename={ADMIN_BASENAME}
        authProvider={authProvider}
        dataProvider={createDataProvider({
          apiBaseUrl: "/api/admin",
          resources: { [RESOURCE]: { getList } },
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
        // that nothing dispatches on, so it never calls the refresh; nothing
        // waits for the in-place sign-in, so its dialog never asks.
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
              Promise.reject(new Error("no in-place sign-in in this test")),
          }
        )}
        dashboard={() => <div data-testid="dashboard" />}
        disableTelemetry
      >
        <Resource
          name={RESOURCE}
          list={ProbeList}
          create={hasCreate ? ProbeCreate : undefined}
        />
      </Admin>
    ),
    { initialEntries: [`${ADMIN_BASENAME}/${RESOURCE}`] }
  )
  render(<RouterProvider router={router} />)
  return { getList, queryClient, missingKeys }
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

describe("ResourceList", () => {
  it("asks for 25 rows to a page and shows that size in the pagination", async () => {
    const { getList, missingKeys } = mount({ allowed: ["list"] })
    expect(await screen.findByText("Első")).toBeInTheDocument()
    expect(getList).toHaveBeenCalled()
    for (const [params] of getList.mock.calls) {
      expect(params.pagination).toEqual({ page: 1, perPage: 25 })
    }
    expect(screen.getByRole("combobox")).toHaveTextContent("25")
    expect(realMissingKeys(missingKeys)).toEqual([])
  })

  it("offers exactly the list contract's page sizes, 10, 25, 50 and 100, not the kit's 5 or an „all”", async () => {
    mount({ allowed: ["list"] })
    await screen.findByText("Első")
    // Without `rowsPerPageOptions`, the kit's pagination would offer 5, 10,
    // 25 and 50: 5 is refused by the server, and 100 would be missing.
    await userEvent.setup().click(screen.getByRole("combobox"))
    const options = await screen.findAllByRole("option")
    expect(options.map((option) => option.textContent?.trim())).toEqual([
      "10",
      "25",
      "50",
      "100",
    ])
  })

  it("never offers the kit's export, which would ask for 1,000 rows, and no „Új” without a create page", async () => {
    mount({ allowed: ["list", "create"] })
    await screen.findByText("Első")
    expect(
      screen.queryByRole("button", { name: hu.ra.action.export })
    ).toBeNull()
    // No create page: ResourceListActions asks nothing and renders nothing.
    expect(screen.queryByRole("link", { name: hu.ra.action.create })).toBeNull()
  })

  it("offers „Új” for a resource with a create page, to an account that may create", async () => {
    mount({ allowed: ["list", "create"], hasCreate: true })
    expect(
      await screen.findByRole("link", { name: hu.ra.action.create })
    ).toHaveAttribute("href", `${ADMIN_BASENAME}/${RESOURCE}/create`)
  })

  it("has no „Új” for an account that may not create", async () => {
    const refused = mount({ allowed: ["list"], hasCreate: true })
    await screen.findByText("Első")
    await accessChecksAnswered(refused.queryClient)
    expect(screen.queryByRole("link", { name: hu.ra.action.create })).toBeNull()
  })

  it("shows the filter button when the list has filters", async () => {
    const { missingKeys } = mount({
      allowed: ["list"],
      listProps: { filters: [<TextInput key="q" source="q" />] },
    })
    await screen.findByText("Első")
    expect(
      await screen.findByRole("button", { name: hu.ra.action.add_filter })
    ).toBeInTheDocument()
    expect(realMissingKeys(missingKeys)).toEqual([])
  })

  it("keeps a page size the screen asks for", async () => {
    const { getList } = mount({ allowed: ["list"], listProps: { perPage: 10 } })
    await screen.findByText("Első")
    await waitFor(() => {
      expect(getList.mock.calls.at(-1)?.[0].pagination).toEqual({
        page: 1,
        perPage: 10,
      })
    })
  })
})

const SRC = resolve(import.meta.dirname, "..")

/**
 * Lists the TypeScript sources under `dir`, without tests.
 *
 * @param dir - the directory to walk.
 * @returns absolute file paths.
 */
function sources(dir: string): string[] {
  return readdirSync(dir, { withFileTypes: true }).flatMap((entry) => {
    const path = join(dir, entry.name)
    if (entry.isDirectory()) {
      return sources(path)
    }
    return /\.tsx?$/.test(entry.name) && !/\.test\.tsx?$/.test(entry.name)
      ? [path]
      : []
  })
}

/**
 * An import of the kit's `List` component (not `ListPagination`, not the
 * `ListProps` type), from the kit's index or from its `list` file.
 */
const KIT_LIST_IMPORT =
  /import\s*\{[^}]*(?<![\w.])List(?![\w])[^}]*\}\s*from\s*["']@\/components\/admin(?:\/list)?["']/

describe("the resource screens", () => {
  it("use ResourceList, never the kit's List, whose defaults the list contract refuses", () => {
    const resources = join(SRC, "resources")
    const offenders = sources(resources)
      .filter((file) => file !== join(resources, "ResourceList.tsx"))
      .filter((file) => KIT_LIST_IMPORT.test(readFileSync(file, "utf8")))
      .map((file) => relative(SRC, file))
    expect(offenders).toEqual([])
  })

  it("recognises an import of the kit's List, and nothing else", () => {
    expect(
      KIT_LIST_IMPORT.test(
        'import { DataTable, List } from "@/components/admin"'
      )
    ).toBe(true)
    expect(
      KIT_LIST_IMPORT.test(
        'import {\n  List,\n} from "@/components/admin/list"'
      )
    ).toBe(true)
    expect(
      KIT_LIST_IMPORT.test(
        'import { ListPagination, type ListProps } from "@/components/admin"'
      )
    ).toBe(false)
    expect(
      KIT_LIST_IMPORT.test('import { List } from "@/resources/ResourceList"')
    ).toBe(false)
  })

  it("scans the screens (guards against a broken walk)", () => {
    expect(
      sources(join(SRC, "resources")).map((file) => relative(SRC, file))
    ).toContain(join("resources", "labels", "LabelList.tsx"))
  })
})
