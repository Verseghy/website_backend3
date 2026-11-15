/**
 * backend3's ra-core auth provider: the admin's only client of
 * `/api/admin/auth` (ADR 0006).
 *
 * Why hand-written: the session lives in the `__Host-admin_session` cookie,
 * which page scripts cannot read (`HttpOnly`, ADR 0006 §1), so the admin can
 * never inspect a token. It asks the server instead: `login` posts the
 * e-mail address and password to `/auth/login`, which sets the cookie, and
 * `checkAuth` and `getIdentity` read `/auth/me`, which answers the signed-in
 * account or `401`. The browser sends the cookie by itself, because the admin
 * and its API share one origin (ADR 0004 §3).
 *
 * The rules this module carries:
 *
 * - `checkError` signs the user out on `401` only, never on `403` (ADR 0006,
 *   "Follow-up work"): a `403` is a permission refusal that the screen shows
 *   in Hungarian (ADR 0007), and the session is still good.
 * - A failed sign-in shows the generic `errors.invalid_credentials` for every
 *   `401`, whatever the body says, so the screen tells nothing about which
 *   part was wrong (ADR 0006 §4, "Same answer for every failure"). Any other
 *   answer that carries an error envelope shows the envelope's own code
 *   (ADR 0021 §4): a refused body (`422 invalid_body`, `413 body_too_large`),
 *   and a `5xx` whose body is an envelope too. A network failure, and any
 *   answer without a readable envelope (Loco's own `500` body, a proxy's
 *   page, an empty answer, a `200` with a step this build does not know),
 *   shows `auth.sign_in.failed`.
 * - `logout` ends the session on the server only when the user asked to sign
 *   out: the user menu's „Kijelentkezés” passes {@link SIGN_OUT_PARAMS}
 *   (`use-sign-out.ts`), and `logout` then posts `/auth/logout`, which
 *   revokes every session of the account and deletes the cookie (ADR 0006
 *   §2). ra-core also calls `logout`, with `{}`, after `checkAuth` or
 *   `checkError` found no session, and on a route a visitor without one may
 *   not see; the server then has nothing to end, so `logout` only lets
 *   ra-core clear its caches and show the sign-in page. Posting there too
 *   would send a request, and log a `401`, before every sign-in page.
 * - A sign-out ends only on the server's word: `204`, or `401`, which says
 *   that the session had ended already (expired, or revoked from another
 *   tab or device). Any other answer, and a network failure, rejects with a
 *   {@link SignOutError}, so ra-core keeps the user on the page, signed in,
 *   and the user menu shows why: the cookie still holds a valid session, and
 *   a sign-out that only looked done would leave it to the next person at a
 *   shared computer.
 *
 * Configuration arrives as explicit options ({@link AuthProviderOptions}),
 * the `fetch` function included, so tests drive the provider against a stub
 * and nothing here reads `window`.
 */
import type { AuthProvider, UserIdentity } from "ra-core"

import { basePathProblem } from "@/api/base-path"
import {
  envelopeMessage,
  keyMessage,
  parseErrorEnvelope,
  type UiMessage,
} from "@/api/error-envelope"
import type { LoginRequest } from "@/bindings/LoginRequest"
import type { LoginResponse } from "@/bindings/LoginResponse"
import type { LoginStep } from "@/bindings/LoginStep"
import type { MeResponse } from "@/bindings/MeResponse"
import type { MessageKey } from "@/i18n/keys"

import { readPermissions, readRoles } from "./authz"

/** The sign-in endpoint, relative to the API base path. */
export const LOGIN_PATH = "/auth/login"
/** The signed-in account's endpoint, relative to the API base path. */
export const ME_PATH = "/auth/me"
/** The sign-out endpoint, relative to the API base path. */
export const LOGOUT_PATH = "/auth/logout"

/** The message of a sign-in that failed for any reason but a refusal. */
export const SIGN_IN_FAILED_KEY = "auth.sign_in.failed" satisfies MessageKey
/** The message of every `401` answer to a sign-in. */
export const INVALID_CREDENTIALS_KEY =
  "errors.invalid_credentials" satisfies MessageKey
/**
 * What ra-core shows when `checkAuth` finds no session: it translates the
 * rejection's `message` (`useCheckAuth`).
 */
export const NO_SESSION_KEY = "ra.auth.auth_check_error" satisfies MessageKey
/** What ra-core shows when `checkAuth` could not ask the server. */
export const CHECK_FAILED_KEY =
  "ra.notification.http_error" satisfies MessageKey
/** What ra-core shows when `checkError` ends a session that expired. */
export const SESSION_ENDED_KEY = "errors.unauthorized" satisfies MessageKey
/**
 * The message of a sign-out that the server did not confirm and that
 * carries no readable error envelope.
 */
export const SIGN_OUT_FAILED_KEY = "auth.sign_out.failed" satisfies MessageKey

/**
 * The `params` that ask `logout` to end the session on the server: what the
 * user menu's „Kijelentkezés” passes through ra-core's `useLogout`, which
 * hands them to `authProvider.logout` unchanged. ra-core's own calls pass
 * `{}` (see the module documentation).
 */
export const SIGN_OUT_PARAMS = Object.freeze({ endSession: true } as const)

/**
 * Whether ra-core's `logout` call is the user's sign-out.
 *
 * @param params - what `useLogout` passed on.
 * @returns `true` only for an object whose `endSession` is exactly `true`,
 *   as {@link SIGN_OUT_PARAMS} has it.
 */
export function isSignOutRequest(params: unknown): boolean {
  return (
    typeof params === "object" &&
    params !== null &&
    (params as { endSession?: unknown }).endSession === true
  )
}

/** Options of {@link createAuthProvider}. */
export interface AuthProviderOptions {
  /**
   * Root-relative base path of the admin API, for example `/api/admin`; the
   * same rule as the data provider's (`src/api/base-path.ts`).
   */
  readonly apiBaseUrl: string
  /**
   * The `fetch` to call. `main.tsx` passes the browser's, bound to `window`;
   * tests pass a stub.
   */
  readonly fetch: typeof globalThis.fetch
}

/**
 * Thrown by {@link createAuthProvider} when its options are invalid. The
 * message says which option is wrong and what it must look like.
 */
export class AuthProviderConfigError extends Error {
  /**
   * @param message - the actionable description of the problem.
   */
  constructor(message: string) {
    super(message)
    this.name = "AuthProviderConfigError"
  }
}

/**
 * The rejection of a failed sign-in. The sign-in screen shows
 * {@link SignInError.display}; `message` is the same text as a `hu.ts` key or
 * the server's text, so ra-core's own notification would show it too.
 */
export class SignInError extends Error {
  /** The HTTP status, or `null` when no answer arrived. */
  readonly status: number | null
  /** What the sign-in screen shows. */
  readonly display: UiMessage

  /**
   * @param status - the HTTP status of the answer, or `null` when the request
   *   failed before an answer arrived.
   * @param display - the message to show.
   */
  constructor(status: number | null, display: UiMessage) {
    super(display.kind === "key" ? display.key : display.text)
    this.name = "SignInError"
    this.status = status
    this.display = display
  }
}

/**
 * The rejection of a sign-out that the server did not confirm. The user
 * menu shows {@link SignOutError.display}; `message` is the same text, as for
 * {@link SignInError}.
 */
export class SignOutError extends Error {
  /** The HTTP status, or `null` when no answer arrived. */
  readonly status: number | null
  /** What the user menu shows. */
  readonly display: UiMessage

  /**
   * @param status - the HTTP status of the answer, or `null` when the request
   *   failed before an answer arrived.
   * @param display - the message to show.
   */
  constructor(status: number | null, display: UiMessage) {
    super(display.kind === "key" ? display.key : display.text)
    this.name = "SignOutError"
    this.status = status
    this.display = display
  }
}

/**
 * The rejection of `checkAuth`, `checkError` and `getIdentity` when there is
 * no usable session or the server could not be asked. ra-core translates its
 * `message`, a `hu.ts` key.
 */
export class SessionCheckError extends Error {
  /** The HTTP status, or `null` when no answer arrived. */
  readonly status: number | null

  /**
   * @param status - the HTTP status of the answer, or `null`.
   * @param messageKey - the `hu.ts` key ra-core shows.
   */
  constructor(status: number | null, messageKey: MessageKey) {
    super(messageKey)
    this.name = "SessionCheckError"
    this.status = status
  }
}

/** The identity ra-core shows for the signed-in account. */
export interface AdminIdentity extends UserIdentity {
  /** The account's public id (`pid`), a UUID. */
  readonly id: string
  /** The display name. */
  readonly fullName: string
  /** The sign-in address. */
  readonly email: string
  /** What the session may do (ADR 0006 §4). */
  readonly scope: MeResponse["scope"]
}

/**
 * Reads the sign-in form's values as the body of `/auth/login`.
 *
 * @param params - what ra-core's `useLogin` passes on from the form.
 * @returns the request body.
 * @throws {TypeError} when either value is not a string: the sign-in screen
 *   always sends two strings, so anything else is a programming error.
 */
function loginRequest(params: unknown): LoginRequest {
  const email = (params as { email?: unknown } | null)?.email
  const password = (params as { password?: unknown } | null)?.password
  if (typeof email !== "string" || typeof password !== "string") {
    throw new TypeError(
      "authProvider.login expects { email: string, password: string }, the values of the sign-in form."
    )
  }
  return { email, password }
}

/**
 * Every step of the generated `LoginStep` union that this build handles.
 *
 * Why a `Record` over the union: the Rust `LoginStep` is the contract
 * (ADR 0005), and ADR 0006 §4 plans two more steps, `totp` and
 * `change_password`. When the Rust enum gains one, the regenerated binding
 * makes this object incomplete, so `npm run typecheck` fails here until
 * `login` handles the new step, instead of the provider quietly treating it
 * as a failed sign-in.
 */
const KNOWN_LOGIN_STEPS: Readonly<Record<LoginStep, true>> = { done: true }

/**
 * Reads a `200` answer of `/auth/login`.
 *
 * @param body - the parsed JSON body, or `undefined` when it was not JSON.
 * @returns the answer with only its `step`, or `undefined` when the body is
 *   not an object whose `step` is one of {@link KNOWN_LOGIN_STEPS}.
 */
export function parseLoginResponse(body: unknown): LoginResponse | undefined {
  if (typeof body !== "object" || body === null || Array.isArray(body)) {
    return undefined
  }
  const { step } = body as Record<string, unknown>
  // `Object.hasOwn`, not `in`: a step such as "toString" must not be found
  // on the prototype.
  if (typeof step !== "string" || !Object.hasOwn(KNOWN_LOGIN_STEPS, step)) {
    return undefined
  }
  return { step: step as LoginStep }
}

/**
 * Reads a `200` answer of `/auth/me`.
 *
 * @param body - the parsed JSON body.
 * @returns the account with its roles, effective permissions and admin flag
 *   (ADR 0007 §7), or `undefined` when the body is not a `MeResponse`: a
 *   member is missing or of the wrong type, or a role or permission is one
 *   this build does not know (`./authz.ts`). The server and this build ship
 *   together in one image, so an unknown value means a broken build, and the
 *   provider treats the answer as a server failure rather than guess.
 */
export function parseMeResponse(body: unknown): MeResponse | undefined {
  if (typeof body !== "object" || body === null || Array.isArray(body)) {
    return undefined
  }
  const member = body as Record<string, unknown>
  const { pid, name, email, scope, is_admin } = member
  const roles = readRoles(member.roles)
  const permissions = readPermissions(member.permissions)
  if (
    typeof pid !== "string" ||
    pid === "" ||
    typeof name !== "string" ||
    typeof email !== "string" ||
    (scope !== "admin" && scope !== "password_change") ||
    roles === undefined ||
    permissions === undefined ||
    typeof is_admin !== "boolean"
  ) {
    return undefined
  }
  return { pid, name, email, scope, roles, permissions, is_admin }
}

/**
 * The message the sign-in screen shows for an answer that was not `200`.
 *
 * @param status - the HTTP status.
 * @param body - the parsed JSON body, or `undefined` when it was not JSON.
 * @returns `errors.invalid_credentials` for every `401`; the envelope's
 *   message for any other refusal that carries one; otherwise
 *   `auth.sign_in.failed`.
 */
export function signInFailureMessage(status: number, body: unknown): UiMessage {
  if (status === 401) {
    return keyMessage(INVALID_CREDENTIALS_KEY)
  }
  const envelope = parseErrorEnvelope(body)
  return envelope === undefined
    ? keyMessage(SIGN_IN_FAILED_KEY)
    : envelopeMessage(envelope)
}

/**
 * The message the user menu shows for a sign-out that the server did not
 * confirm.
 *
 * @param body - the parsed JSON body of the answer, or `undefined` when it
 *   was not JSON or no answer arrived.
 * @returns the envelope's message when the body is an error envelope (a
 *   refusal such as a foreign `Origin`), otherwise `auth.sign_out.failed`.
 */
export function signOutFailureMessage(body: unknown): UiMessage {
  const envelope = parseErrorEnvelope(body)
  return envelope === undefined
    ? keyMessage(SIGN_OUT_FAILED_KEY)
    : envelopeMessage(envelope)
}

/**
 * The status of an error that ra-core hands to `checkError`: the data
 * provider's `HttpError` carries it as `status`.
 *
 * @param error - whatever the failed call rejected with.
 * @returns the status, or `undefined` when the error has none.
 */
function errorStatus(error: unknown): number | undefined {
  const status = (error as { status?: unknown } | null | undefined)?.status
  return typeof status === "number" ? status : undefined
}

/**
 * Parses a response body as JSON, tolerating a body that is not.
 *
 * @param response - the answer.
 * @returns the parsed body, or `undefined` when it is empty or not JSON.
 */
async function readJson(response: Response): Promise<unknown> {
  try {
    return await response.json()
  } catch {
    return undefined
  }
}

/**
 * Builds backend3's auth provider.
 *
 * @param options - the API base path and the `fetch` to use; see
 *   {@link AuthProviderOptions}.
 * @returns an ra-core `AuthProvider` with `login`, `logout`, `checkAuth`,
 *   `checkError` and `getIdentity`. It has no `getPermissions` or
 *   `canAccess` yet: the admin API decides every permission itself
 *   (ADR 0007), and the gated navigation comes with the authorization
 *   slices.
 * @throws {AuthProviderConfigError} when the options are invalid.
 */
export function createAuthProvider(options: AuthProviderOptions): AuthProvider {
  const problem = basePathProblem("apiBaseUrl", options.apiBaseUrl)
  if (problem !== undefined) {
    throw new AuthProviderConfigError(problem)
  }
  if (typeof options.fetch !== "function") {
    throw new AuthProviderConfigError(
      "fetch must be a function; main.tsx passes window.fetch bound to window."
    )
  }
  const { apiBaseUrl, fetch } = options

  /**
   * Asks `/auth/me` for the signed-in account.
   *
   * @returns the account.
   * @throws {SessionCheckError} with `ra.auth.auth_check_error` for a `401`
   *   and `ra.notification.http_error` for any other failure.
   */
  const readMe = async (): Promise<MeResponse> => {
    let response: Response
    try {
      response = await fetch(`${apiBaseUrl}${ME_PATH}`, {
        method: "GET",
        credentials: "same-origin",
        headers: { Accept: "application/json" },
      })
    } catch {
      throw new SessionCheckError(null, CHECK_FAILED_KEY)
    }
    if (response.status === 401) {
      throw new SessionCheckError(401, NO_SESSION_KEY)
    }
    const me = response.ok
      ? parseMeResponse(await readJson(response))
      : undefined
    if (me === undefined) {
      throw new SessionCheckError(response.status, CHECK_FAILED_KEY)
    }
    return me
  }

  return {
    async login(params: unknown) {
      const body = loginRequest(params)
      let response: Response
      try {
        response = await fetch(`${apiBaseUrl}${LOGIN_PATH}`, {
          method: "POST",
          credentials: "same-origin",
          headers: {
            Accept: "application/json",
            "Content-Type": "application/json",
          },
          body: JSON.stringify(body),
        })
      } catch {
        throw new SignInError(null, keyMessage(SIGN_IN_FAILED_KEY))
      }
      const answer = await readJson(response)
      if (!response.ok) {
        throw new SignInError(
          response.status,
          signInFailureMessage(response.status, answer)
        )
      }
      // `done` is the only step this build knows (`KNOWN_LOGIN_STEPS`); the
      // two-factor and forced password-change steps (ADR 0006 §4) arrive
      // with their own screens. An answer this build cannot read is a failed
      // sign-in, not a session.
      if (parseLoginResponse(answer)?.step !== "done") {
        throw new SignInError(response.status, keyMessage(SIGN_IN_FAILED_KEY))
      }
      // Resolving without `redirectTo` lets ra-core go back to the page that
      // sent the user here, or to the dashboard.
      return undefined
    },

    async logout(params: unknown) {
      // ra-core's own calls come after it found no session: nothing to end
      // on the server (see the module documentation).
      if (!isSignOutRequest(params)) {
        return undefined
      }
      let response: Response
      try {
        response = await fetch(`${apiBaseUrl}${LOGOUT_PATH}`, {
          method: "POST",
          credentials: "same-origin",
          headers: { Accept: "application/json" },
        })
      } catch {
        throw new SignOutError(null, keyMessage(SIGN_OUT_FAILED_KEY))
      }
      // 204: every session of the account ended, and the cookie is gone.
      // 401: there was no session left to end. Either way the user is
      // signed out, and ra-core shows the sign-in page.
      if (response.status === 204 || response.status === 401) {
        return undefined
      }
      // Anything else, a 2xx other than 204 included, is not the server's
      // confirmation: the session may still hold.
      throw new SignOutError(
        response.status,
        signOutFailureMessage(await readJson(response))
      )
    },

    async checkAuth() {
      await readMe()
    },

    async checkError(error: unknown) {
      if (errorStatus(error) === 401) {
        throw new SessionCheckError(401, SESSION_ENDED_KEY)
      }
      // Every other error, 403 included, leaves the session alone: the
      // screen shows it, and the user stays signed in.
    },

    async getIdentity(): Promise<AdminIdentity> {
      const me = await readMe()
      return { id: me.pid, fullName: me.name, email: me.email, scope: me.scope }
    },
  }
}
