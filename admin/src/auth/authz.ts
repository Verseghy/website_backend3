/**
 * The roles, permissions and access map that `/auth/me` reports (ADR 0007
 * §1, §2, §7), as values the admin can check at run time.
 *
 * Why this module exists: the generated bindings `Role`, `Permission` and
 * `AccessAction` are types only, and a type says nothing about a body that
 * arrives over the network. `parseMeResponse` (`./auth-provider.ts`) uses
 * the readers here to accept only the roles, permissions and actions that
 * this build knows.
 *
 * The convention: the records below are typed `Record<Role, true>`,
 * `Record<Permission, true>` and `Record<AccessAction, true>` over the
 * generated unions. The Rust enums are the contract (ADR 0005); when one of
 * them gains or loses a value, the regenerated binding makes its record
 * incomplete or adds an unknown key, and `npm run typecheck` fails here
 * until the record follows. The identifiers stay in English exactly as the
 * server sends them (ADR 0021 §6); their Hungarian labels belong to the
 * screens that show them.
 *
 * The admin UI only hides what an account may not use; the server decides
 * every access itself (ADR 0007 §7).
 */
import type { AccessAction } from "@/bindings/AccessAction"
import type { MeResponse } from "@/bindings/MeResponse"
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
 * Every action of the access map, in the server's order (ADR 0007 §7):
 * ra-core's five, then backend3's four.
 */
export const KNOWN_ACCESS_ACTIONS: Readonly<Record<AccessAction, true>> = {
  list: true,
  show: true,
  create: true,
  edit: true,
  delete: true,
  publish: true,
  restore: true,
  purge: true,
  history: true,
}

/**
 * Whether `value` is the name of an access action this build knows.
 *
 * @param value - any value, such as the `action` that ra-core passes to
 *   `canAccess`.
 * @returns `true` for one of {@link KNOWN_ACCESS_ACTIONS}'s keys.
 */
export function isAccessAction(value: unknown): value is AccessAction {
  // `Object.hasOwn`, not `in`: an action such as "toString" must not be
  // found on the prototype.
  return typeof value === "string" && Object.hasOwn(KNOWN_ACCESS_ACTIONS, value)
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

/**
 * Reads the `access` member of a `/auth/me` answer: the account's actions
 * per ra-core resource (ADR 0007 §7).
 *
 * @param value - the member's parsed JSON value.
 * @returns the map itself, or `undefined` when the value is not a plain
 *   object whose every member is a list of known actions, each once. The
 *   server lists the actions in its own order and leaves out a resource
 *   with none.
 *
 * The parsed object is returned, not a copy: `JSON.parse` keeps a member
 * named `__proto__` as an own property, whereas assigning it to a fresh
 * object would set that object's prototype instead. Readers look a resource
 * up with {@link accessActionsOf}, never with `in` or a bare index.
 */
export function readAccess(value: unknown): MeResponse["access"] | undefined {
  if (typeof value !== "object" || value === null || Array.isArray(value)) {
    return undefined
  }
  // A plain object only (what JSON.parse makes, or one without a
  // prototype): a class instance or a boxed value is no access map.
  const prototype: unknown = Object.getPrototypeOf(value)
  if (prototype !== Object.prototype && prototype !== null) {
    return undefined
  }
  for (const actions of Object.values(value)) {
    if (readKnownList(actions, KNOWN_ACCESS_ACTIONS) === undefined) {
      return undefined
    }
  }
  return value as MeResponse["access"]
}

/**
 * The actions an access map lists for `resource`.
 *
 * @param access - an access map that {@link readAccess} accepted.
 * @param resource - an ra-core resource name, of any spelling.
 * @returns the resource's actions, or an empty list when the map has no own
 *   member of that name: a name such as `constructor` or `__proto__` is
 *   never found on the object's prototype.
 */
export function accessActionsOf(
  access: MeResponse["access"],
  resource: string
): readonly AccessAction[] {
  return Object.hasOwn(access, resource) ? (access[resource] ?? []) : []
}
