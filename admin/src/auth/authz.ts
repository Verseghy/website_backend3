/**
 * The roles and permissions that `/auth/me` reports (ADR 0007 §1, §2, §7),
 * as values the admin can check at run time.
 *
 * Why this module exists: the generated bindings `Role` and `Permission`
 * are types only, and a type says nothing about a body that arrives over
 * the network. `parseMeResponse` (`./auth-provider.ts`) uses the readers
 * here to accept only the roles and permissions that this build knows.
 *
 * The convention: the two records below are typed `Record<Role, true>` and
 * `Record<Permission, true>` over the generated unions. The Rust enums are
 * the contract (ADR 0005); when one of them gains or loses a value, the
 * regenerated binding makes its record incomplete or adds an unknown key,
 * and `npm run typecheck` fails here until the record follows. The
 * identifiers stay in English exactly as the server stores them (ADR 0021
 * §6); their Hungarian labels belong to the screens that show them.
 *
 * The admin UI only hides what an account may not use; the server decides
 * every access itself (ADR 0007 §7).
 */
import type { Permission } from "@/bindings/Permission"
import type { Role } from "@/bindings/Role"

/** Every role, in the server's id order (ADR 0007 §3). */
export const KNOWN_ROLES: Readonly<Record<Role, true>> = {
  writer: true,
  secretary: true,
  supervisor: true,
  admin: true,
}

/** Every permission, in the order of ADR 0007 §1. */
export const KNOWN_PERMISSIONS: Readonly<Record<Permission, true>> = {
  "edit posts": true,
  "publish posts": true,
  "edit labels": true,
  "edit authors": true,
  "edit events": true,
  "edit canteens": true,
  "edit colleagues": true,
  "edit users": true,
  "edit pages": true,
  "edit menu": true,
  "manage media": true,
  "view logs": true,
  "edit notices": true,
  "edit banners": true,
  "edit site settings": true,
  "edit diaktoll": true,
}

/**
 * Reads a list whose every entry is a key of `known`, each at most once.
 *
 * @param value - the parsed JSON value to read.
 * @param known - the accepted entries, as the keys of a record.
 * @returns the entries in their order, or `undefined` when `value` is not an
 *   array, holds anything but a known entry, or holds one twice.
 */
function readKnownList<T extends string>(
  value: unknown,
  known: Readonly<Record<T, true>>
): T[] | undefined {
  if (!Array.isArray(value)) {
    return undefined
  }
  const seen = new Set<unknown>()
  for (const entry of value) {
    // `Object.hasOwn`, not `in`: an entry such as "toString" must not be
    // found on the prototype.
    if (
      typeof entry !== "string" ||
      !Object.hasOwn(known, entry) ||
      seen.has(entry)
    ) {
      return undefined
    }
    seen.add(entry)
  }
  return value as T[]
}

/**
 * Reads the `roles` member of a `/auth/me` answer.
 *
 * @param value - the member's parsed JSON value.
 * @returns the roles, or `undefined` when the value is not a list of known
 *   roles, each once. The server sends each role once, in id order.
 */
export function readRoles(value: unknown): Role[] | undefined {
  return readKnownList(value, KNOWN_ROLES)
}

/**
 * Reads the `permissions` member of a `/auth/me` answer.
 *
 * @param value - the member's parsed JSON value.
 * @returns the permissions, or `undefined` when the value is not a list of
 *   known permissions, each once. The server sends the account's effective
 *   permissions, each once, in the order of ADR 0007 §1.
 */
export function readPermissions(value: unknown): Permission[] | undefined {
  return readKnownList(value, KNOWN_PERMISSIONS)
}
