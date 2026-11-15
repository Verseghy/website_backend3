/**
 * The access wrappers of `./access-buttons.tsx` against ra-core's real
 * `useCanAccess` (ADR 0007 §7): each button appears only when the access
 * map allows its action, an action on a record asks about that record so a
 * record-level rule decides it, a create question never takes the record
 * of the surrounding context, a bulk delete asks about every selected
 * record the list has loaded, a button stays hidden while its question is
 * open and when it failed (without leaving the page), and the buttons
 * follow a changed `/auth/me` answer after a `403`.
 *
 * The auth provider answers with `decideAccess` (`./access.ts`) over an
 * invented account and a probe record-level rule on the probe resource
 * `probe-posts`, which stands in for the posts rule of a later slice: a
 * published record needs `publish posts`. Its `resources.probe-posts.*`
 * labels are the only missing Hungarian keys the tests let through.
 */
import { QueryClient } from "@tanstack/react-query"
import { act, render, screen, waitFor } from "@testing-library/react"
import {
  CoreAdminContext,
  ListContextProvider,
  RecordContextProvider,
  ResourceContextProvider,
  TestMemoryRouter,
  testDataProvider,
  useList,
  useRecordContext,
  type AuthProvider,
  type Identifier,
  type RaRecord,
} from "ra-core"
import type { ReactNode } from "react"
import { describe, expect, it, vi } from "vitest"

import type { AccessAction } from "@/bindings/AccessAction"
import type { MeResponse } from "@/bindings/MeResponse"
import { createI18nProvider } from "@/i18n/provider"
import { accessChecksAnswered } from "@/test/access-checks"

import { decideAccess, type AccessSubject, type RecordRule } from "./access"
import {
  CanAccessBulkDeleteButton,
  CanAccessCreateButton,
  CanAccessDeleteButton,
  CanAccessEditButton,
  CanAccessPurgeButton,
  CanAccessRestoreButton,
  RecordAccessGate,
  ResourceAccessGate,
  SelectedRecordsAccessGate,
} from "./access-buttons"
import { CAN_ACCESS_QUERY_KEY, invalidateAccessChecks } from "./access-checks"
import { createAuthProvider } from "./auth-provider"
import { createSessionExpiry } from "./session-expiry"

/** The probe resource; no screen of backend3 registers it. */
const RESOURCE = "probe-posts"

/** A draft and a published record of the probe resource. */
const DRAFT = { id: 7, title: "Vázlat", published: false }
const PUBLISHED = { id: 8, title: "Megjelent", published: true }

/**
 * An invented account with `actions` on the probe resource.
 *
 * @param actions - the access map's actions on {@link RESOURCE}.
 * @param publishes - whether the account holds `publish posts`.
 * @returns the account as `/auth/me` would answer it.
 */
function subject(
  actions: readonly AccessAction[],
  publishes = false
): AccessSubject {
  return {
    pid: "0b7c4a52-7e3d-4c55-9a7f-4b3c2d1e0f9a",
    permissions: publishes ? ["edit posts", "publish posts"] : ["edit posts"],
    is_admin: false,
    access: { [RESOURCE]: [...actions] },
  }
}

/** A writer: everything on the probe resource but publish and purge. */
const WRITER = subject(["list", "show", "create", "edit", "delete", "restore"])
/** A reader: the list and the records only. */
const READER = subject(["list", "show"])
/** A supervisor: the writer's actions, and `publish posts`. */
const SUPERVISOR = subject(
  ["list", "show", "create", "edit", "delete", "publish", "restore"],
  true
)
/** An admin: every action, purge included. */
const ADMIN: AccessSubject = {
  ...subject(
    ["list", "show", "create", "edit", "delete", "publish", "restore", "purge"],
    true
  ),
  is_admin: true,
}

/**
 * The probe record-level rule: a published record needs `publish posts`,
 * whatever the action, as the posts rule of ADR 0007 §5 reads.
 */
const PROBE_RULES: Readonly<Record<string, RecordRule>> = {
  [RESOURCE]: (_action, record, account) =>
    record.published !== true || account.permissions.includes("publish posts"),
}

/** The parameters ra-core passes to `canAccess`. */
interface Question {
  readonly action: string
  readonly resource: string
  readonly record?: unknown
}

/**
 * An auth provider whose `canAccess` answers with `decideAccess` over
 * `account` and the probe rule, watched.
 *
 * @param account - the signed-in account.
 * @returns the provider and its watched `canAccess`.
 */
function probeProvider(account: AccessSubject) {
  const canAccess = vi.fn(async (question: Question) =>
    decideAccess(account, question, PROBE_RULES)
  )
  const authProvider: AuthProvider = {
    login: async () => undefined,
    logout: async () => undefined,
    checkAuth: async () => undefined,
    checkError: async () => undefined,
    canAccess,
  }
  return { authProvider, canAccess }
}

/**
 * A query client for one test.
 *
 * @returns a client that does not retry: a failed question is answered at
 *   once instead of after react-query's retries.
 */
function newQueryClient(): QueryClient {
  return new QueryClient({ defaultOptions: { queries: { retry: false } } })
}

/**
 * Renders `ui` under ra-core with `authProvider`, inside the probe
 * resource's context, in a memory router at `/`.
 *
 * @param ui - the element under test.
 * @param authProvider - the provider ra-core asks.
 * @param queryClient - the client ra-core caches the answers in; a new
 *   one by default.
 * @returns the query client, the missing keys reported, the current path
 *   and a rerender that keeps the providers.
 */
function mount(
  ui: ReactNode,
  authProvider: AuthProvider,
  queryClient: QueryClient = newQueryClient()
) {
  const missingKeys: string[] = []
  const i18nProvider = createI18nProvider({
    reportMissingKeys: true,
    onMissingKey: (key) => {
      missingKeys.push(key)
    },
  })
  let pathname = ""
  const wrap = (element: ReactNode) => (
    <TestMemoryRouter
      locationCallback={(location) => {
        pathname = location.pathname
      }}
    >
      <CoreAdminContext
        authProvider={authProvider}
        dataProvider={testDataProvider()}
        i18nProvider={i18nProvider}
        queryClient={queryClient}
      >
        <ResourceContextProvider value={RESOURCE}>
          {element}
        </ResourceContextProvider>
      </CoreAdminContext>
    </TestMemoryRouter>
  )
  const { rerender } = render(wrap(ui))
  return {
    queryClient,
    missingKeys,
    path: () => pathname,
    rerender: (next: ReactNode) => rerender(wrap(next)),
  }
}

/**
 * The missing Hungarian keys other than the probe resource's own labels.
 *
 * @param missingKeys - the keys the provider reported.
 * @returns the keys that are not about {@link RESOURCE}.
 */
function foreignMissingKeys(missingKeys: readonly string[]): string[] {
  return missingKeys.filter((key) => !key.startsWith(`resources.${RESOURCE}.`))
}

/** Shows the id of the record context, for a child's view of it. */
function RecordId() {
  const record = useRecordContext()
  return <output data-testid="record-id">{String(record?.id ?? "none")}</output>
}

describe("CanAccessCreateButton", () => {
  it("shows the kit's create link when the access map allows create", async () => {
    const { authProvider, canAccess } = probeProvider(WRITER)
    const { queryClient, missingKeys } = mount(
      <CanAccessCreateButton />,
      authProvider
    )
    await accessChecksAnswered(queryClient)
    expect(screen.getByRole("link")).toHaveAttribute(
      "href",
      `/${RESOURCE}/create`
    )
    expect(canAccess).toHaveBeenCalledWith(
      expect.objectContaining({ action: "create", resource: RESOURCE })
    )
    expect(foreignMissingKeys(missingKeys)).toEqual([])
  })

  it("renders nothing when the access map does not offer create", async () => {
    const { authProvider } = probeProvider(READER)
    const { queryClient } = mount(<CanAccessCreateButton />, authProvider)
    await accessChecksAnswered(queryClient)
    expect(screen.queryByRole("link")).toBeNull()
  })

  it("asks without the surrounding record and gives that record back to its children", async () => {
    // A published record in the context would make the probe rule refuse
    // the writer; create is about the resource, so it must not reach it.
    const { authProvider, canAccess } = probeProvider(WRITER)
    const { queryClient } = mount(
      <RecordContextProvider value={PUBLISHED}>
        <ResourceAccessGate action="create">
          <RecordId />
        </ResourceAccessGate>
      </RecordContextProvider>,
      authProvider
    )
    await accessChecksAnswered(queryClient)
    expect(screen.getByTestId("record-id")).toHaveTextContent(
      String(PUBLISHED.id)
    )
    expect(canAccess).toHaveBeenCalledTimes(1)
    expect(canAccess.mock.calls[0][0].record).toBeUndefined()
  })
})

describe("CanAccessEditButton", () => {
  it("shows the edit link of the record in the context when the account may edit it", async () => {
    const { authProvider, canAccess } = probeProvider(WRITER)
    const { queryClient, missingKeys } = mount(
      <RecordContextProvider value={DRAFT}>
        <CanAccessEditButton />
      </RecordContextProvider>,
      authProvider
    )
    await accessChecksAnswered(queryClient)
    expect(screen.getByRole("link")).toHaveAttribute(
      "href",
      `/${RESOURCE}/${DRAFT.id}`
    )
    // The record goes with the question.
    expect(canAccess).toHaveBeenCalledWith(
      expect.objectContaining({
        action: "edit",
        resource: RESOURCE,
        record: DRAFT,
      })
    )
    expect(foreignMissingKeys(missingKeys)).toEqual([])
  })

  it("lets the record-level rule decide: a published record needs publish posts", async () => {
    const writer = probeProvider(WRITER)
    const first = mount(
      <CanAccessEditButton record={PUBLISHED} />,
      writer.authProvider
    )
    await accessChecksAnswered(first.queryClient)
    expect(screen.queryByRole("link")).toBeNull()
    first.rerender(<CanAccessEditButton record={DRAFT} />)
    await accessChecksAnswered(first.queryClient)
    expect(screen.getByRole("link")).toHaveAttribute(
      "href",
      `/${RESOURCE}/${DRAFT.id}`
    )
  })

  it("shows a published record's edit link to an account that publishes", async () => {
    const { authProvider } = probeProvider(SUPERVISOR)
    const { queryClient } = mount(
      <CanAccessEditButton record={PUBLISHED} />,
      authProvider
    )
    await accessChecksAnswered(queryClient)
    expect(screen.getByRole("link")).toHaveAttribute(
      "href",
      `/${RESOURCE}/${PUBLISHED.id}`
    )
  })

  it("renders nothing for an account the access map refuses edit", async () => {
    const { authProvider } = probeProvider(READER)
    const { queryClient } = mount(
      <CanAccessEditButton record={DRAFT} />,
      authProvider
    )
    await accessChecksAnswered(queryClient)
    expect(screen.queryByRole("link")).toBeNull()
  })

  it("renders nothing and asks nothing without a record", async () => {
    const { authProvider, canAccess } = probeProvider(WRITER)
    mount(<CanAccessEditButton />, authProvider)
    await act(async () => {
      await new Promise((resolve) => setTimeout(resolve, 0))
    })
    expect(screen.queryByRole("link")).toBeNull()
    expect(canAccess).not.toHaveBeenCalled()
  })
})

describe("CanAccessDeleteButton", () => {
  it("shows the kit's delete button for a record the account may delete", async () => {
    const { authProvider, canAccess } = probeProvider(WRITER)
    const { queryClient, missingKeys } = mount(
      <RecordContextProvider value={DRAFT}>
        <CanAccessDeleteButton />
      </RecordContextProvider>,
      authProvider
    )
    await accessChecksAnswered(queryClient)
    expect(screen.getByRole("button")).toBeEnabled()
    expect(canAccess).toHaveBeenCalledWith(
      expect.objectContaining({ action: "delete", record: DRAFT })
    )
    expect(foreignMissingKeys(missingKeys)).toEqual([])
  })

  it("hides it for a published record of an account that does not publish, and for an account without delete", async () => {
    for (const [account, record] of [
      [WRITER, PUBLISHED],
      [READER, DRAFT],
    ] as const) {
      const { authProvider } = probeProvider(account)
      const { queryClient } = mount(
        <CanAccessDeleteButton record={record} />,
        authProvider
      )
      await accessChecksAnswered(queryClient)
      expect(screen.queryByRole("button")).toBeNull()
    }
  })
})

/** Three records of the probe list: two drafts and a published one. */
const LISTED: readonly RaRecord[] = [
  { id: 1, title: "Első", published: false },
  { id: 2, title: "Második", published: false },
  { id: 3, title: "Harmadik", published: true },
]

/**
 * A list context over {@link LISTED} with `selected` selected.
 *
 * @param props - `selected`: the selected ids; `children`: what renders in
 *   the list.
 * @returns the list context provider.
 */
function ProbeList({
  selected,
  children,
}: {
  readonly selected: readonly Identifier[]
  readonly children: ReactNode
}) {
  const list = useList({ data: [...LISTED], resource: RESOURCE })
  return (
    <ListContextProvider value={{ ...list, selectedIds: [...selected] }}>
      {children}
    </ListContextProvider>
  )
}

describe("CanAccessBulkDeleteButton", () => {
  it("shows the kit's bulk delete when the account may delete every selected record", async () => {
    const { authProvider, canAccess } = probeProvider(WRITER)
    const { queryClient, missingKeys } = mount(
      <ProbeList selected={[1, 2]}>
        <CanAccessBulkDeleteButton />
      </ProbeList>,
      authProvider
    )
    await accessChecksAnswered(queryClient)
    expect(screen.getByRole("button")).toBeEnabled()
    // The resource first, then each selected record.
    const records = canAccess.mock.calls.map(([question]) => question.record)
    expect(records).toEqual(
      expect.arrayContaining([undefined, LISTED[0], LISTED[1]])
    )
    expect(records).not.toContain(LISTED[2])
    expect(foreignMissingKeys(missingKeys)).toEqual([])
  })

  it("hides it while a selected record is refused, and shows it once that record is deselected", async () => {
    const { authProvider } = probeProvider(WRITER)
    const { queryClient, rerender } = mount(
      <ProbeList selected={[1, 3]}>
        <CanAccessBulkDeleteButton />
      </ProbeList>,
      authProvider
    )
    await accessChecksAnswered(queryClient)
    expect(screen.queryByRole("button")).toBeNull()
    rerender(
      <ProbeList selected={[1]}>
        <CanAccessBulkDeleteButton />
      </ProbeList>
    )
    await accessChecksAnswered(queryClient)
    expect(screen.getByRole("button")).toBeEnabled()
  })

  it("asks about no record when the resource itself refuses delete", async () => {
    const { authProvider, canAccess } = probeProvider(READER)
    const { queryClient } = mount(
      <ProbeList selected={[1, 2]}>
        <CanAccessBulkDeleteButton />
      </ProbeList>,
      authProvider
    )
    await accessChecksAnswered(queryClient)
    expect(screen.queryByRole("button")).toBeNull()
    expect(canAccess).toHaveBeenCalledTimes(1)
    expect(canAccess.mock.calls[0][0].record).toBeUndefined()
  })

  it("leaves a selected record the list has not loaded to the resource's answer", async () => {
    const { authProvider } = probeProvider(WRITER)
    const { queryClient } = mount(
      <ProbeList selected={[99]}>
        <SelectedRecordsAccessGate action="delete">
          <output data-testid="gated" />
        </SelectedRecordsAccessGate>
      </ProbeList>,
      authProvider
    )
    // No record to ask about: the children render at once.
    expect(screen.getByTestId("gated")).toBeInTheDocument()
    expect(
      queryClient.getQueryCache().findAll({ queryKey: CAN_ACCESS_QUERY_KEY })
    ).toEqual([])
  })
})

describe("CanAccessRestoreButton and CanAccessPurgeButton", () => {
  it("show the trash's buttons in the context of the record they were checked for", async () => {
    const { authProvider, canAccess } = probeProvider(ADMIN)
    const { queryClient } = mount(
      <>
        <CanAccessRestoreButton record={DRAFT}>
          <span data-testid="restore">
            <RecordId />
          </span>
        </CanAccessRestoreButton>
        <CanAccessPurgeButton record={PUBLISHED}>
          <span data-testid="purge">
            <RecordId />
          </span>
        </CanAccessPurgeButton>
      </>,
      authProvider
    )
    await accessChecksAnswered(queryClient)
    expect(screen.getByTestId("restore")).toHaveTextContent(String(DRAFT.id))
    expect(screen.getByTestId("purge")).toHaveTextContent(String(PUBLISHED.id))
    expect(canAccess).toHaveBeenCalledWith(
      expect.objectContaining({ action: "restore", record: DRAFT })
    )
    expect(canAccess).toHaveBeenCalledWith(
      expect.objectContaining({ action: "purge", record: PUBLISHED })
    )
  })

  it("hide purge from every account but the admin, and restore where the record-level rule refuses", async () => {
    const { authProvider } = probeProvider(WRITER)
    const { queryClient } = mount(
      <RecordContextProvider value={PUBLISHED}>
        <CanAccessPurgeButton>
          <span data-testid="purge" />
        </CanAccessPurgeButton>
        <CanAccessRestoreButton>
          <span data-testid="restore-published" />
        </CanAccessRestoreButton>
        <CanAccessRestoreButton record={DRAFT}>
          <span data-testid="restore-draft" />
        </CanAccessRestoreButton>
      </RecordContextProvider>,
      authProvider
    )
    await accessChecksAnswered(queryClient)
    expect(screen.queryByTestId("purge")).toBeNull()
    expect(screen.queryByTestId("restore-published")).toBeNull()
    expect(screen.getByTestId("restore-draft")).toBeInTheDocument()
  })
})

describe("an access question", () => {
  it("hides the button while it is open", async () => {
    const authProvider: AuthProvider = {
      ...probeProvider(ADMIN).authProvider,
      canAccess: () => new Promise<boolean>(() => {}),
    }
    mount(
      <RecordAccessGate action="edit" record={DRAFT}>
        <span data-testid="gated" />
      </RecordAccessGate>,
      authProvider
    )
    await act(async () => {
      await new Promise((resolve) => setTimeout(resolve, 0))
    })
    expect(screen.queryByTestId("gated")).toBeNull()
  })

  it("that fails hides the button and keeps the page, unlike ra-core's CanAccess", async () => {
    const authProvider: AuthProvider = {
      ...probeProvider(ADMIN).authProvider,
      canAccess: async () => {
        throw new Error("no /auth/me answer")
      },
    }
    const { queryClient, path } = mount(
      <>
        <CanAccessCreateButton />
        <CanAccessEditButton record={DRAFT} />
      </>,
      authProvider
    )
    await accessChecksAnswered(queryClient)
    expect(screen.queryByRole("link")).toBeNull()
    expect(path()).toBe("/")
  })
})

describe("with backend3's auth provider", () => {
  it("follows a changed /auth/me answer after a 403, without signing out", async () => {
    let me: MeResponse = {
      pid: "0b7c4a52-7e3d-4c55-9a7f-4b3c2d1e0f9a",
      name: "Próba Szerkesztő",
      email: "proba@example.test",
      scope: "admin",
      roles: [],
      permissions: ["edit posts"],
      is_admin: false,
      access: { [RESOURCE]: ["list", "create", "edit"] },
    }
    const fetch = vi.fn<typeof globalThis.fetch>(
      async () =>
        new Response(JSON.stringify(me), {
          status: 200,
          headers: { "Content-Type": "application/json" },
        })
    )
    const queryClient = newQueryClient()
    const authProvider = createAuthProvider({
      apiBaseUrl: "/api/admin",
      fetch,
      // Every answer here is a 200, so nothing waits for the in-place sign-in.
      sessionExpiry: createSessionExpiry(),
      onAccessChange: () => {
        void invalidateAccessChecks(queryClient)
      },
    })
    await authProvider.checkAuth({})
    mount(
      <>
        <CanAccessCreateButton />
        <CanAccessEditButton record={DRAFT} />
      </>,
      authProvider,
      queryClient
    )
    await accessChecksAnswered(queryClient)
    expect(
      screen.getAllByRole("link").map((link) => link.getAttribute("href"))
    ).toEqual([`/${RESOURCE}/create`, `/${RESOURCE}/${DRAFT.id}`])

    // The account loses create in another session; a call is refused 403.
    me = { ...me, access: { [RESOURCE]: ["list", "edit"] } }
    await act(async () => {
      await expect(
        authProvider.checkError({ status: 403, message: "errors.forbidden" })
      ).resolves.toBeUndefined()
    })
    await waitFor(() => {
      expect(
        screen.getAllByRole("link").map((link) => link.getAttribute("href"))
      ).toEqual([`/${RESOURCE}/${DRAFT.id}`])
    })
  })
})
