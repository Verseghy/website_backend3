/**
 * The access case table, `tests/fixtures/authz_cases.json`, against the
 * admin's side of ADR 0007 §7: `authProvider.canAccess` answering from a
 * `/auth/me` answer that holds the case's access list, with the TypeScript
 * mirror of the record-level rules (`./access.ts`).
 *
 * The Rust test `tests/domain/authz_cases.rs` runs the same file against
 * the server: it checks that each case's `access` is what `/auth/me` sends
 * that account and that `expected` is the server's own answer. This test
 * checks that the admin answers `expected` too, so a rule that changes on
 * one side only fails one of the two (ADR 0007 §7, "so the two cannot
 * drift"). The file's description says what each member holds.
 */
import { readFileSync } from "node:fs"
import { resolve } from "node:path"

import { describe, expect, it, vi } from "vitest"

import type { MeResponse } from "@/bindings/MeResponse"

import { RECORD_RULES } from "./access"
import { createAuthProvider } from "./auth-provider"
import { readAccess, readPermissions } from "./authz"

/** One case of the table, as the file holds it. */
interface AccessCase {
  readonly name: string
  readonly permissions: MeResponse["permissions"]
  readonly is_admin: boolean
  readonly resource: string
  readonly action: string
  readonly record: Readonly<Record<string, unknown>> | null
  readonly access: MeResponse["access"][string]
  readonly expected: boolean
}

/** The members every case has, and no other. */
const CASE_MEMBERS = [
  "name",
  "permissions",
  "is_admin",
  "resource",
  "action",
  "record",
  "access",
  "expected",
]

/**
 * Reads one case of the file, failing the test with the case's position
 * when it is not what the file's description says.
 *
 * @param value - the parsed case.
 * @param index - its position in the file, for the message.
 * @returns the case.
 */
function readCase(value: unknown, index: number): AccessCase {
  const where = `case ${index} of tests/fixtures/authz_cases.json`
  expect(typeof value === "object" && value !== null, where).toBe(true)
  const member = value as Record<string, unknown>
  expect(Object.keys(member).sort(), where).toEqual([...CASE_MEMBERS].sort())
  const { name, is_admin, resource, action, record, expected } = member
  expect(typeof name === "string" && name.trim() !== "", where).toBe(true)
  const label = `${where} (${String(name)})`
  const permissions = readPermissions(member.permissions)
  expect(permissions, `${label}: permissions`).toBeDefined()
  expect(typeof is_admin, label).toBe("boolean")
  expect(typeof resource, label).toBe("string")
  expect(typeof action, label).toBe("string")
  expect(
    record === null || (typeof record === "object" && !Array.isArray(record)),
    `${label}: record is an object or null`
  ).toBe(true)
  // The access list goes through the same reader as /auth/me's answer.
  const access = readAccess({ [String(resource)]: member.access })
  expect(access, `${label}: access`).toBeDefined()
  expect(typeof expected, label).toBe("boolean")
  return {
    name: name as string,
    permissions: permissions ?? [],
    is_admin: is_admin as boolean,
    resource: resource as string,
    action: action as string,
    record: record as AccessCase["record"],
    access: member.access as AccessCase["access"],
    expected: expected as boolean,
  }
}

/** Every case of the file, read and checked. */
function readCases(): AccessCase[] {
  const text = readFileSync(
    resolve(import.meta.dirname, "../../../tests/fixtures/authz_cases.json"),
    "utf8"
  )
  const file = JSON.parse(text) as { description?: unknown; cases?: unknown }
  expect(Object.keys(file).sort()).toEqual(["cases", "description"])
  expect(typeof file.description).toBe("string")
  expect(Array.isArray(file.cases)).toBe(true)
  return (file.cases as unknown[]).map(readCase)
}

/**
 * The `/auth/me` answer for a case's account: an invented account with the
 * case's permissions and admin flag, whose access map holds the case's list
 * for the case's resource.
 *
 * @param entry - the case.
 * @returns the answer, as JSON text, so that a resource such as `__proto__`
 *   arrives as the network would bring it.
 */
function meText(entry: AccessCase): string {
  const me: Omit<MeResponse, "access"> = {
    pid: "0b7c4a52-7e3d-4c55-9a7f-4b3c2d1e0f9a",
    name: "Próba Fiók",
    email: "proba@example.test",
    scope: "admin",
    roles: entry.is_admin ? ["admin"] : [],
    permissions: entry.permissions,
    is_admin: entry.is_admin,
  }
  // JSON.stringify of an object built with a computed key keeps a
  // `__proto__` member as an own member, as the server would send it.
  const access =
    entry.access.length === 0 ? {} : { [entry.resource]: entry.access }
  return JSON.stringify({ ...me, access })
}

const CASES = readCases()

describe("the access case table", () => {
  it("names each case once", () => {
    const names = CASES.map((entry) => entry.name)
    expect(new Set(names).size).toBe(names.length)
    expect(names.length).toBeGreaterThan(0)
  })

  it.each(CASES.map((entry) => [entry.name, entry] as const))(
    "%s",
    async (_name, entry) => {
      const fetch = vi.fn<typeof globalThis.fetch>(
        async () =>
          new Response(meText(entry), {
            status: 200,
            headers: { "Content-Type": "application/json" },
          })
      )
      const provider = createAuthProvider({ apiBaseUrl: "/api/admin", fetch })
      await provider.checkAuth({})
      const answer = await provider.canAccess?.({
        action: entry.action,
        resource: entry.resource,
        ...(entry.record === null ? {} : { record: entry.record }),
      } as never)
      expect(answer).toBe(entry.expected)
      // canAccess answered from the answer checkAuth kept.
      expect(fetch).toHaveBeenCalledTimes(1)
    }
  )

  it("has a case with a record for every record-level rule of the mirror", () => {
    for (const resource of Object.keys(RECORD_RULES)) {
      expect(
        CASES.some(
          (entry) => entry.resource === resource && entry.record !== null
        ),
        `add a case with a record on ${resource} to tests/fixtures/authz_cases.json`
      ).toBe(true)
    }
  })
})
