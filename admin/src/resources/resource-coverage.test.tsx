/**
 * The "Resource coverage" check of ADR 0021 §7: every resource the admin
 * registers has its Hungarian name, `resources.<name>.name`, in both of
 * polyglot's forms (one |||| other), and a label for the field that names
 * its records (`recordRepresentation`). Also: every registered resource has
 * data-provider handlers in `src/resources/handlers.ts`, and every set of
 * handlers serves a registered resource, so `main.tsx` cannot leave a screen
 * without its reads.
 *
 * Why it reads the registered resources from ra-core: the resources are the
 * `<Resource>` elements of `src/App.tsx`, and a slice adds one there. The
 * test mounts `AdminApp` as `main.tsx` does and reads ra-core's own resource
 * definitions through a probe page (`developmentRoutes`, which renders inside
 * the layout), so it sees every resource a slice registers, whatever its
 * screens look up. The list columns' labels are checked by each screen's own
 * test, which the i18n provider fails on a missing key.
 */
import { QueryClient } from "@tanstack/react-query"
import { render, screen } from "@testing-library/react"
import { useResourceDefinitions } from "ra-core"
import { Route, RouterProvider, createMemoryRouter } from "react-router-dom"
import { describe, expect, it, vi } from "vitest"

import { AdminApp } from "@/App"
import { createAuthProvider } from "@/auth/auth-provider"
import { createSessionExpiry } from "@/auth/session-expiry"
import type { MeResponse } from "@/bindings/MeResponse"
import { createDataProvider } from "@/data-provider/data-provider"
import { hu } from "@/i18n/hu"
import { lookupMessage } from "@/i18n/messages"
import { createI18nProvider } from "@/i18n/provider"
import { createResourceHandlers } from "@/resources/handlers"
import { adminRoutes } from "@/router"

/** What the probe page reports about one registered resource. */
interface RegisteredResource {
  /** The ra-core resource name. */
  readonly name: string
  /** The record field that names a record, when it is a field name. */
  readonly recordRepresentation: string | null
}

/** The probe page's path below the admin's basename. */
const PROBE_PATH = "/resource-probe"

/** An invented account; the probe page needs a session, and no access. */
const ME: MeResponse = {
  pid: "6f1d2c3b-4a5e-4f60-8b7a-9c0d1e2f3a4b",
  name: "Próba Olvasó",
  email: "olvaso@example.test",
  scope: "admin",
  roles: [],
  permissions: [],
  is_admin: false,
  access: {},
}

/**
 * Renders ra-core's resource definitions as JSON, for the test to read.
 *
 * @returns a `<pre>` with one {@link RegisteredResource} per resource.
 */
function ResourceProbe() {
  const definitions = useResourceDefinitions()
  const registered: RegisteredResource[] = Object.values(definitions).map(
    (definition) => ({
      name: definition.name,
      recordRepresentation:
        typeof definition.recordRepresentation === "string"
          ? definition.recordRepresentation
          : null,
    })
  )
  return <pre data-testid="resource-probe">{JSON.stringify(registered)}</pre>
}

/**
 * Mounts the admin on the probe page and reads the registered resources.
 *
 * @returns the resources, sorted by name.
 */
async function registeredResources(): Promise<RegisteredResource[]> {
  const fetch = vi.fn<typeof globalThis.fetch>(
    async () =>
      new Response(JSON.stringify(ME), {
        status: 200,
        headers: { "Content-Type": "application/json" },
      })
  )
  // One retry queue for both providers and the app, as main.tsx has it.
  const sessionExpiry = createSessionExpiry()
  const router = createMemoryRouter(
    adminRoutes(
      <AdminApp
        dataProvider={createDataProvider({
          apiBaseUrl: "/api/admin",
          resources: createResourceHandlers(),
          fetch,
          sessionExpiry,
        })}
        authProvider={createAuthProvider({
          apiBaseUrl: "/api/admin",
          fetch,
          sessionExpiry,
        })}
        i18nProvider={createI18nProvider({ reportMissingKeys: true })}
        queryClient={new QueryClient()}
        sourceCommit={undefined}
        developmentRoutes={
          <Route path={PROBE_PATH} element={<ResourceProbe />} />
        }
        activityTarget={new EventTarget()}
        now={() => 0}
        sessionExpiry={sessionExpiry}
      />
    ),
    { initialEntries: [`/admin${PROBE_PATH}`] }
  )
  render(<RouterProvider router={router} />)
  const probe = await screen.findByTestId("resource-probe")
  const registered = JSON.parse(probe.textContent) as RegisteredResource[]
  return [...registered].sort((a, b) => a.name.localeCompare(b.name))
}

describe("the registered resources", () => {
  it("have a Hungarian name in both forms and a label for the field that names a record (ADR 0021 §7)", async () => {
    const registered = await registeredResources()
    // Guards against a probe that reads nothing: the labels are registered.
    expect(registered.map(({ name }) => name)).toContain("labels")

    const problems = registered.flatMap(({ name, recordRepresentation }) => {
      const found: string[] = []
      const label = lookupMessage(hu, `resources.${name}.name`)
      const forms = label?.split(" |||| ") ?? []
      if (forms.length !== 2 || forms.some((form) => form.trim() === "")) {
        found.push(
          `resources.${name}.name must be "<one> |||| <other>" (found ${JSON.stringify(label)})`
        )
      }
      if (
        recordRepresentation !== null &&
        lookupMessage(
          hu,
          `resources.${name}.fields.${recordRepresentation}`
        ) === undefined
      ) {
        found.push(
          `resources.${name}.fields.${recordRepresentation} is missing; recordRepresentation names that field`
        )
      }
      return found
    })
    expect(
      problems,
      `Add these to admin/src/i18n/hu.ts:\n${problems.join("\n")}`
    ).toEqual([])
  })

  it("are exactly the resources that src/resources/handlers.ts serves", async () => {
    const registered = (await registeredResources()).map(({ name }) => name)
    const served = Object.keys(createResourceHandlers()).sort((a, b) =>
      a.localeCompare(b)
    )
    expect(
      served,
      "Each <Resource> of src/App.tsx needs its handlers in src/resources/handlers.ts, and each handler set its <Resource>."
    ).toEqual(registered)
  })
})
