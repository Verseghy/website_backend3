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
 * - `refreshSession`, which ra-core never calls, posts `/auth/refresh`, the
 *   explicit sliding refresh (ADR 0006 §1): the server renews the session
 *   cookie for two hours, up to the session's absolute limit. The activity
 *   tracker (`activity-refresh.ts`) calls it on typing or clicking, at most
 *   every 10 minutes, so a long edit without saves stays signed in. It never
 *   rejects and never signs out: it reports a {@link RefreshOutcome}, and a
 *   session that has ended meets its `401` on the next real request, where
 *   `checkError` handles it. It gives up on an answer that has not come
 *   after {@link REFRESH_TIMEOUT_MS}, so the tracker can call again.
 * - `canAccess` answers from the last `/auth/me` answer, without a request
 *   of its own (ADR 0007 §7): `checkAuth` reads `/auth/me` before ra-core
 *   renders a page, and the provider keeps that answer. Only when it holds
 *   none yet does `canAccess` read it, once for all the questions that wait.
 *   The answer is `decideAccess` (`./access.ts`): the access map the server
 *   computed, and the record-level rules that the admin mirrors.
 * - A `403` refetches `/auth/me` (ADR 0007 §7): the server refused although
 *   the access map allowed the call, which happens when the account's roles
 *   or grants changed since the last answer, for example from another
 *   session. The refetch is a read sent after the refusal, never one that
 *   was already in flight and may carry the old access (`refetchMe`).
 *   `checkError` then keeps the user signed in, and when the new
 *   answer differs, {@link AuthProviderOptions.onAccessChange} lets ra-core
 *   ask its cached access questions again, so the sidebar and the screens
 *   follow at once. The refused call's own notification shows the Hungarian
 *   message (`refusalError`, `src/api/error-envelope.ts`). Only a `401`,
 *   from the call or from that refetch, signs the user out, and the
 *   refetch's `401 unauthorized` first waits for the in-place sign-in, as
 *   `checkAuth`'s does (below).
 * - A session that ends under an open page is renewed in place, not by the
 *   sign-in page (ADR 0006, "Follow-up work"). When the `/auth/me` read of
 *   `checkAuth`, or the refetch of a `403`, answers `401 unauthorized`, the
 *   check waits on the retry queue of `session-expiry.ts` while the layout's
 *   dialog is mounted, instead of rejecting, so ra-core's session check,
 *   which react-query repeats whenever the tab comes back into view, cannot
 *   swap the layout and its open form for the sign-in page; after the
 *   sign-in it reads `/auth/me` again. Without a mounted dialog (the first
 *   check of a page load) the `401` rejects at once, as before. The read
 *   itself never waits, so `getIdentity` and `canAccess`, which share it,
 *   never wait: `getIdentity`'s `401` leaves ra-core's identity as it was,
 *   and nothing signs out on it. The dialog signs in again with
 *   `resumeSession`, for the account whose session ended, which
 *   `sessionAccount` names: the account of the last `/auth/me` answer that
 *   the provider kept. The waiting requests are sent again only when that
 *   same account holds a full `admin` session again (`resumed`); otherwise
 *   (`restart`) the dialog ends the new session with the confirmed
 *   sign-out. Both rules, with „Kilépés” as the same confirmed sign-out, are
 *   the maintainer's answer of 2026-11-15, "Confirm all three as built"
 *   (research: DECISIONS R37 (D158)). `resumeSession` gives up after
 *   {@link RESUME_TIMEOUT_MS}, so a stalled request never leaves the dialog
 *   without a way on. Every `logout` gives the in-place sign-in up, so no
 *   `401` opens the dialog while ra-core leaves the signed-in pages.
 * - The kept answer belongs to one session: a sign-in, a sign-out and a
 *   `401` that `checkAuth` or `checkError` rejects with drop it, so the next
 *   account never sees the previous one's access. A `401` that waits for the
 *   in-place sign-in keeps it, so `canAccess` goes on answering the open
 *   page's questions while the dialog is open (a rejection there would send
 *   the page to the authentication-error page), and a resumed session
 *   replaces it with the answer that `resumeSession` read.
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

import { decideAccess } from "./access"
import { readAccess, readPermissions, readRoles } from "./authz"
import {
  isSessionEnded,
  type SessionExpiry,
  type SignInWait,
} from "./session-expiry"

/** The sign-in endpoint, relative to the API base path. */
export const LOGIN_PATH = "/auth/login"
/** The signed-in account's endpoint, relative to the API base path. */
export const ME_PATH = "/auth/me"
/** The sign-out endpoint, relative to the API base path. */
export const LOGOUT_PATH = "/auth/logout"
/** The sliding session refresh, relative to the API base path. */
export const REFRESH_PATH = "/auth/refresh"

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
  /**
   * How long `refreshSession` waits for an answer before it gives up, in
   * milliseconds; at least 1. {@link REFRESH_TIMEOUT_MS} unless a test says
   * otherwise.
   */
  readonly refreshTimeoutMs?: number
  /**
   * How long `resumeSession` waits for its two answers (the sign-in and
   * `/auth/me`) together before it gives up, in milliseconds; at least 1.
   * {@link RESUME_TIMEOUT_MS} unless a test says otherwise.
   */
  readonly resumeTimeoutMs?: number
  /**
   * The retry queue of the in-place sign-in (`createSessionExpiry`), where
   * the `401 unauthorized` of `checkAuth`, and of the `/auth/me` refetch of
   * a `403`, waits while the layout's dialog is mounted. `main.tsx` passes
   * the one queue that the data provider and the app get too; a queue with
   * no dialog attached hands every `401` back at once.
   */
  readonly sessionExpiry: SessionExpiry
  /**
   * Called after a `/auth/me` answer changed what the signed-in account
   * may do (its access map, permissions, admin flag or scope), so that the
   * answers ra-core cached for `canAccess` are asked again. `main.tsx`
   * passes `invalidateAccessChecks` (`./access-checks.ts`) over the query
   * client it gives `<Admin>`; without it, ra-core re-asks only when a
   * component mounts again. Never called for the first answer of a session.
   */
  readonly onAccessChange?: () => void
}

/**
 * How long `refreshSession` waits for an answer: 30 seconds.
 *
 * Why it is bounded: the activity tracker starts no new refresh while one
 * is open, and neither the browser nor backend3 (Loco's request timeout is
 * off) ends a request that stalls, so one hung call would stop the refresh
 * for good and end a long edit at the token's two hours. The refresh is a
 * small request that answers at once; 30 seconds is far beyond a slow
 * answer and far below the tracker's 10-minute interval.
 */
export const REFRESH_TIMEOUT_MS = 30 * 1000

/**
 * How long `resumeSession`, the in-place sign-in, waits for its answers:
 * 30 seconds.
 *
 * Why it is bounded: the dialog disables both of its buttons while the
 * sign-in is on its way, because leaving then could race a session cookie
 * that the sign-in sets after the sign-out. Neither the browser nor
 * backend3 ends a request that stalls, so without a bound one stalled
 * request would leave a reload as the only way on, and a reload loses the
 * open form that the dialog exists to keep. The bound is the refresh's
 * ({@link REFRESH_TIMEOUT_MS}): far beyond a slow sign-in, which checks one
 * password hash, and short enough that the user can try again or leave.
 */
export const RESUME_TIMEOUT_MS = 30 * 1000

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

/**
 * The rejection of an `/auth/me` read whose answer was `401 unauthorized`:
 * the session has ended (`isSessionEnded` of `./session-expiry.ts`).
 *
 * To ra-core it is the {@link SessionCheckError} of any other `401`, with
 * the sign-in prompt. It also carries the retry queue's round in which the
 * read was sent, which a check that may wait for the in-place sign-in hands
 * to `waitForSignIn`: a read that left before the last round ended carries
 * the old cookie and must not open a second dialog.
 */
class SessionEndedError extends SessionCheckError {
  /** The retry queue's round when the read was sent. */
  readonly sentIn: number

  /**
   * @param sentIn - `SessionExpiry.round()`, read right before the request
   *   was sent.
   */
  constructor(sentIn: number) {
    super(401, NO_SESSION_KEY)
    this.sentIn = sentIn
  }
}

/**
 * What `refreshSession` found:
 *
 * - `refreshed`: `204`; the server renewed the session cookie, or kept it
 *   unchanged at the session's absolute limit, where nothing is left to
 *   renew;
 * - `ended`: `401`; there is no session to renew;
 * - `failed`: any other answer, or no answer (a network failure, or none
 *   within {@link REFRESH_TIMEOUT_MS}); the session is as it was.
 */
export type RefreshOutcome = "refreshed" | "ended" | "failed"

/**
 * The account whose session the open page holds: the one that `/auth/me`
 * last answered. The in-place sign-in shows it and signs it in again.
 */
export interface SessionAccount {
  /** The account's public id (`pid`), a UUID. */
  readonly pid: string
  /** The display name. */
  readonly name: string
  /** The sign-in address. */
  readonly email: string
}

/**
 * What `resumeSession` found after a successful sign-in:
 *
 * - `resumed`: the same account holds a full `admin` session again, so the
 *   waiting requests may be sent again;
 * - `restart`: `/auth/me` answered for another account (a sign-in in
 *   another tab overwrote the cookie in the meantime, or the address moved
 *   to another account) or for a session whose scope is not `admin`. The
 *   waiting requests, built under the old session, must not be sent; the
 *   dialog ends the session that the cookie now holds with the confirmed
 *   sign-out, and ra-core then shows the sign-in page.
 *
 * Only the account whose session ended may continue: the maintainer's
 * answer of 2026-11-15 (research: DECISIONS R37 (D158)).
 *
 * Today `/auth/login` answers only the `done` step; a `change_password` or
 * `totp` step is a failed sign-in until the slices that add those steps
 * teach `login` and `resumeSession` about them (see `KNOWN_LOGIN_STEPS`).
 */
export type ResumeOutcome = "resumed" | "restart"

/**
 * backend3's auth provider: ra-core's methods, the explicit session refresh
 * that the activity tracker calls (ADR 0006 §1), and the in-place sign-in
 * of the session-expiry dialog.
 */
export interface BackendAuthProvider extends AuthProvider {
  /**
   * Posts `/auth/refresh`.
   *
   * @returns what the server answered; it never rejects.
   */
  refreshSession(): Promise<RefreshOutcome>
  /**
   * The account whose session the open page holds.
   *
   * @returns the account of the last `/auth/me` answer, or `undefined`
   *   before the first one and after a sign-out.
   */
  sessionAccount(): SessionAccount | undefined
  /**
   * Signs {@link BackendAuthProvider.sessionAccount} in again with
   * `password` (`POST /auth/login`), then asks `/auth/me` who holds the new
   * session. It never waits for the dialog: the dialog is what calls it.
   * Both requests together are bounded by the `resumeTimeoutMs` option
   * ({@link RESUME_TIMEOUT_MS}). The remembered account and the kept access
   * answer change only on `resumed`, so after a `restart` they still belong
   * to the account whose session ended.
   *
   * @param password - the password as typed.
   * @returns `resumed` or `restart`; see {@link ResumeOutcome}.
   * @throws {SignInError} when the sign-in failed, with the message the
   *   sign-in screen would show, or with `auth.sign_in.failed` when no
   *   account is known, no answer came within the timeout, or `/auth/me`
   *   could not be read after it.
   * @throws {TypeError} when `password` is not a string.
   */
  resumeSession(password: string): Promise<ResumeOutcome>
}

/**
 * The outcome of an answer of `/auth/refresh`.
 *
 * @param status - the HTTP status.
 * @returns `refreshed` for `204`, `ended` for `401`, `failed` otherwise
 *   (a `200` too: the endpoint answers no body).
 */
export function refreshOutcome(status: number): RefreshOutcome {
  if (status === 204) {
    return "refreshed"
  }
  return status === 401 ? "ended" : "failed"
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
 * @returns the account with its roles, effective permissions, admin flag and
 *   access map (ADR 0007 §7), or `undefined` when the body is not a
 *   `MeResponse`: a member is missing or of the wrong type, or a role,
 *   permission or action is one this build does not know (`./authz.ts`).
 *   The server and this build ship together in one image, so an unknown
 *   value means a broken build, and the provider treats the answer as a
 *   server failure rather than guess.
 */
export function parseMeResponse(body: unknown): MeResponse | undefined {
  if (typeof body !== "object" || body === null || Array.isArray(body)) {
    return undefined
  }
  const member = body as Record<string, unknown>
  const { pid, name, email, scope, is_admin } = member
  const roles = readRoles(member.roles)
  const permissions = readPermissions(member.permissions)
  const access = readAccess(member.access)
  if (
    typeof pid !== "string" ||
    pid === "" ||
    typeof name !== "string" ||
    typeof email !== "string" ||
    (scope !== "admin" && scope !== "password_change") ||
    roles === undefined ||
    permissions === undefined ||
    typeof is_admin !== "boolean" ||
    access === undefined
  ) {
    return undefined
  }
  return { pid, name, email, scope, roles, permissions, is_admin, access }
}

/**
 * Whether two `/auth/me` answers let the account do the same things.
 *
 * @param before - the answer the provider held.
 * @param after - the answer that replaces it.
 * @returns `true` when the account, the scope, the permissions, the admin
 *   flag and the access map are the same. The server sends each list and
 *   each map in one fixed order, so comparing their JSON is exact.
 */
export function sameAccess(before: MeResponse, after: MeResponse): boolean {
  const facts = (me: MeResponse) =>
    JSON.stringify([me.pid, me.scope, me.is_admin, me.permissions, me.access])
  return facts(before) === facts(after)
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
 * Reads a timeout option.
 *
 * @param name - the option's name, for the error message.
 * @param value - the option as passed, or `undefined`.
 * @param fallback - the value when the option is left out, and the one the
 *   error message names.
 * @param fallbackName - the constant that holds `fallback`.
 * @returns the timeout in milliseconds.
 * @throws {AuthProviderConfigError} when the value is not a finite number of
 *   at least 1.
 */
function timeoutOption(
  name: string,
  value: number | undefined,
  fallback: number,
  fallbackName: string
): number {
  const timeout = value ?? fallback
  if (!Number.isFinite(timeout) || timeout < 1) {
    throw new AuthProviderConfigError(
      `${name} must be a finite number of milliseconds, at least 1; got ${String(timeout)}. The admin uses ${fallbackName} (${String(fallback / 1000)} seconds).`
    )
  }
  return timeout
}

/**
 * Builds backend3's auth provider.
 *
 * @param options - the API base path, the `fetch` to use, the timeouts of
 *   the refresh and of the in-place sign-in, its retry queue and the
 *   access-change callback; see {@link AuthProviderOptions}.
 * @returns an ra-core `AuthProvider` with `login`, `logout`, `checkAuth`,
 *   `checkError`, `getIdentity` and `canAccess`, and backend3's
 *   `refreshSession`, `sessionAccount` and `resumeSession`. It has no
 *   `getPermissions`: the access map of `/auth/me` answers every access
 *   question, and the admin API decides every request itself (ADR 0007).
 * @throws {AuthProviderConfigError} when the options are invalid.
 */
export function createAuthProvider(
  options: AuthProviderOptions
): BackendAuthProvider {
  const problem = basePathProblem("apiBaseUrl", options.apiBaseUrl)
  if (problem !== undefined) {
    throw new AuthProviderConfigError(problem)
  }
  if (typeof options.fetch !== "function") {
    throw new AuthProviderConfigError(
      "fetch must be a function; main.tsx passes window.fetch bound to window."
    )
  }
  const refreshTimeoutMs = timeoutOption(
    "refreshTimeoutMs",
    options.refreshTimeoutMs,
    REFRESH_TIMEOUT_MS,
    "REFRESH_TIMEOUT_MS"
  )
  const resumeTimeoutMs = timeoutOption(
    "resumeTimeoutMs",
    options.resumeTimeoutMs,
    RESUME_TIMEOUT_MS,
    "RESUME_TIMEOUT_MS"
  )
  if (typeof options.sessionExpiry?.waitForSignIn !== "function") {
    throw new AuthProviderConfigError(
      "sessionExpiry must be the retry queue of createSessionExpiry(); main.tsx creates one and passes the same queue to both providers and to the app."
    )
  }
  if (
    options.onAccessChange !== undefined &&
    typeof options.onAccessChange !== "function"
  ) {
    throw new AuthProviderConfigError(
      "onAccessChange must be a function or left out; main.tsx passes one that invalidates ra-core's cached canAccess answers."
    )
  }
  const { apiBaseUrl, fetch, sessionExpiry, onAccessChange } = options

  /** The last `/auth/me` answer of this session, or `undefined`. */
  let current: MeResponse | undefined
  /**
   * The account of the last `/auth/me` answer that the provider kept: what
   * the in-place sign-in shows and signs in again. Unlike {@link current},
   * a `401` never clears it, since it names the account whose session
   * ended; every sign-out does.
   */
  let account: SessionAccount | undefined
  /** The `/auth/me` read in flight, which every caller shares. */
  let reading: Promise<MeResponse> | undefined
  /**
   * The refetch that waits for the read in flight to settle before it
   * sends its own: what a `403` asks for while a read is already on its
   * way ({@link refetchMe}). Every refusal that arrives meanwhile shares it.
   */
  let queued: Promise<void> | undefined
  /**
   * Counts the sign-ins and sign-outs, so a read that was sent before one
   * of them never stores its answer after it.
   */
  let generation = 0

  /** Drops the kept answer: the session it belonged to is over. */
  const forget = () => {
    generation += 1
    current = undefined
    reading = undefined
    queued = undefined
  }

  /**
   * Forgets the session and the account and gives the in-place sign-in up:
   * ra-core leaves the signed-in pages now, so no `401` may open the dialog
   * on the way out (ra-core asks `checkAuth` once more before it signs out
   * on a refused request). The next layout's dialog attaches again.
   */
  const signedOut = (): void => {
    forget()
    account = undefined
    sessionExpiry.abandon()
  }

  /**
   * Keeps an `/auth/me` answer of this session: the access answer that
   * `canAccess` reads, and the account that the in-place sign-in names.
   * When it lets the account do something else than the kept answer,
   * `onAccessChange` runs.
   *
   * @param me - the answer.
   */
  const keep = (me: MeResponse): void => {
    const before = current
    current = me
    account = { pid: me.pid, name: me.name, email: me.email }
    if (before !== undefined && !sameAccess(before, me)) {
      onAccessChange?.()
    }
  }

  /**
   * Asks `/auth/me` for the signed-in account, once: it never waits for the
   * in-place sign-in.
   *
   * @param signal - ends the request; the in-place sign-in's timeout. The
   *   other reads pass none, as before the in-place sign-in existed.
   * @returns the account.
   * @throws {SessionCheckError} with `ra.auth.auth_check_error` for a `401`
   *   (a {@link SessionEndedError} when it says the session ended) and
   *   `ra.notification.http_error` for any other failure, an aborted
   *   request included.
   */
  const askMe = async (signal?: AbortSignal): Promise<MeResponse> => {
    // Read before the request leaves: a sign-in that ends the round while
    // the request is on its way makes its 401 an old cookie's answer
    // (`session-expiry.ts`, "Rounds").
    const sentIn = sessionExpiry.round()
    let response: Response
    try {
      response = await fetch(`${apiBaseUrl}${ME_PATH}`, {
        method: "GET",
        credentials: "same-origin",
        headers: { Accept: "application/json" },
        ...(signal === undefined ? {} : { signal }),
      })
    } catch {
      throw new SessionCheckError(null, CHECK_FAILED_KEY)
    }
    if (response.status === 401) {
      const ended = await isSessionEnded(response)
      // Nobody reads this 401 (`isSessionEnded` read a copy, and a check
      // that waits sends a new read after the sign-in): release its
      // connection, as `sendWithSignIn` does before its retry.
      await response.body?.cancel().catch(() => undefined)
      throw ended
        ? new SessionEndedError(sentIn)
        : new SessionCheckError(401, NO_SESSION_KEY)
    }
    const me = response.ok
      ? parseMeResponse(await readJson(response))
      : undefined
    if (me === undefined) {
      throw new SessionCheckError(response.status, CHECK_FAILED_KEY)
    }
    return me
  }

  /**
   * Reads `/auth/me` and keeps the answer; callers that ask while a read is
   * in flight share it. A failure leaves the kept answer: the check that
   * ra-core signs out on drops it ({@link checkUntilSignedIn}), and any
   * other caller's failure, `getIdentity`'s `401` included, says nothing
   * final about the session while the in-place sign-in may still renew it.
   *
   * @returns the account.
   * @throws {SessionCheckError} as {@link askMe} does.
   */
  const readMe = (): Promise<MeResponse> => {
    if (reading !== undefined) {
      return reading
    }
    const sentIn = generation
    const read = askMe().then((me) => {
      if (sentIn === generation) {
        keep(me)
      }
      return me
    })
    const shared = read.finally(() => {
      if (reading === shared) {
        reading = undefined
      }
    })
    reading = shared
    return shared
  }

  /**
   * Reads `/auth/me` with a request sent after this call, for a `403`: the
   * server refused although the kept answer allowed the call, so only an
   * answer that the server wrote after the refusal can say what changed.
   *
   * A read in flight was sent before the refusal (as far as the provider
   * can tell) and may carry the old access, so the refetch is not that
   * read: it waits for it to settle and then reads again ({@link readMe},
   * which a read started meanwhile serves, since that one was sent after
   * the refusal too). Refusals that arrive while such a refetch waits share
   * it, so a burst of `403`s sends one more read, not one each. A sign-in
   * or sign-out while it waits cancels it: that session's access is no
   * longer the question.
   *
   * @returns a promise that settles when the new answer is kept.
   * @throws {SessionCheckError} as {@link askMe} does.
   */
  const refetchMe = (): Promise<void> => {
    if (reading === undefined) {
      return readMe().then(() => undefined)
    }
    if (queued !== undefined) {
      return queued
    }
    const queuedIn = generation
    const ignore = () => undefined
    const next: Promise<void> = reading.then(ignore, ignore).then(async () => {
      if (queued === next) {
        queued = undefined
      }
      if (queuedIn === generation) {
        await readMe()
      }
    })
    queued = next
    return next
  }

  /**
   * Runs `read` for a check that ra-core signs out on (`checkAuth`, and
   * `checkError` for a `403`), and waits for the in-place sign-in whenever
   * its `401 unauthorized` says the session ended: after the sign-in it runs
   * `read` again, as `sendWithSignIn` does for the data provider's
   * requests. Without a mounted dialog, or after the user gave up, the
   * `401` rejects at once and drops the kept answer.
   *
   * @param read - reads `/auth/me` once ({@link readMe} or
   *   {@link refetchMe}).
   * @param signal - ra-core's abort signal of the check, if it has one: it
   *   ends only the wait, never the read, which other callers may share.
   * @throws {SessionCheckError} as {@link askMe} does, after any wait; with
   *   `ra.notification.http_error` and no status when `signal` ended the
   *   wait.
   */
  const checkUntilSignedIn = async (
    read: () => Promise<unknown>,
    signal?: AbortSignal
  ): Promise<void> => {
    for (;;) {
      const sentIn = generation
      try {
        await read()
        return
      } catch (error) {
        if (error instanceof SessionEndedError) {
          let wait: SignInWait
          try {
            wait = await sessionExpiry.waitForSignIn(error.sentIn, signal)
          } catch (failure) {
            if (signal?.aborted === true) {
              // ra-core cancelled the check: it waits no more, and nothing
              // about the session is decided.
              throw new SessionCheckError(null, CHECK_FAILED_KEY)
            }
            throw failure
          }
          if (wait === "resumed") {
            continue
          }
        }
        if (
          sentIn === generation &&
          error instanceof SessionCheckError &&
          error.status === 401
        ) {
          // ra-core signs out on this rejection: the session is over.
          current = undefined
        }
        throw error
      }
    }
  }

  /**
   * Posts the e-mail address and password to `/auth/login`, which sets the
   * session cookie.
   *
   * @param body - the request body.
   * @param signal - ends the request; the in-place sign-in's timeout. The
   *   sign-in screen passes none.
   * @throws {SignInError} with the message the sign-in screen shows (see
   *   the module documentation).
   */
  const signIn = async (
    body: LoginRequest,
    signal?: AbortSignal
  ): Promise<void> => {
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
        ...(signal === undefined ? {} : { signal }),
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
    // with their own screens. Until then a `totp` or `change_password`
    // answer is a failed sign-in here, on the sign-in screen and in the
    // in-place sign-in alike; the slices that add the steps teach both
    // about them. An answer this build cannot read is a failed sign-in,
    // not a session.
    if (parseLoginResponse(answer)?.step !== "done") {
      throw new SignInError(response.status, keyMessage(SIGN_IN_FAILED_KEY))
    }
  }

  return {
    async login(params: unknown) {
      await signIn(loginRequest(params))
      // A new session, perhaps of another account: nothing kept from before
      // may answer for it. The next checkAuth reads its /auth/me.
      forget()
      // Resolving without `redirectTo` lets ra-core go back to the page that
      // sent the user here, or to the dashboard.
      return undefined
    },

    async logout(params: unknown) {
      // ra-core's own calls come after it found no session: nothing to end
      // on the server (see the module documentation), and nothing to keep.
      if (!isSignOutRequest(params)) {
        signedOut()
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
        signedOut()
        return undefined
      }
      // Anything else, a 2xx other than 204 included, is not the server's
      // confirmation: the session may still hold.
      throw new SignOutError(
        response.status,
        signOutFailureMessage(await readJson(response))
      )
    },

    async checkAuth(params: unknown) {
      // ra-core passes react-query's abort signal with the check's params.
      const signal = (params as { signal?: unknown } | null | undefined)?.signal
      await checkUntilSignedIn(
        readMe,
        signal instanceof AbortSignal ? signal : undefined
      )
    },

    async checkError(error: unknown) {
      const status = errorStatus(error)
      if (status === 401) {
        throw new SessionCheckError(401, SESSION_ENDED_KEY)
      }
      if (status === 403) {
        // The access map allowed a call that the server refused: the
        // account's roles or grants may have changed (ADR 0007 §7). Ask
        // again, with a read sent after the refusal; readMe tells ra-core
        // to re-ask its access questions when the answer changed. A
        // session that ended in between waits for the in-place sign-in
        // first, so the open page stays.
        try {
          await checkUntilSignedIn(refetchMe)
        } catch (failure) {
          // The session ended in between and the user gave up, or no
          // dialog can ask: that, not the 403, signs out.
          if (failure instanceof SessionCheckError && failure.status === 401) {
            throw new SessionCheckError(401, SESSION_ENDED_KEY)
          }
          // Any other failure keeps the session and the kept answer; the
          // next checkAuth asks again.
        }
      }
      // Every other error, 403 included, leaves the session alone: the
      // screen shows it, and the user stays signed in.
    },

    async getIdentity(): Promise<AdminIdentity> {
      // Never waits for the in-place sign-in (see the module
      // documentation): its 401 rejects at once and ends nothing.
      const me = await readMe()
      return { id: me.pid, fullName: me.name, email: me.email, scope: me.scope }
    },

    sessionAccount: () => account,

    async resumeSession(password: string): Promise<ResumeOutcome> {
      if (typeof password !== "string") {
        throw new TypeError(
          "authProvider.resumeSession expects the password as a string, the value of the dialog's password input."
        )
      }
      const expected = account
      if (expected === undefined) {
        // The layout, and so the dialog, renders only after `checkAuth`
        // read an account; without one there is nobody to sign in again.
        throw new SignInError(null, keyMessage(SIGN_IN_FAILED_KEY))
      }
      // One bound for both requests (RESUME_TIMEOUT_MS says why). A request
      // that it ends rejects with a `TimeoutError`, which both steps below
      // report as `auth.sign_in.failed`.
      const signal = AbortSignal.timeout(resumeTimeoutMs)
      // A sign-out while the sign-in is on its way ends this session's
      // claim on the kept answer.
      const sentIn = generation
      await signIn({ email: expected.email, password }, signal)
      let me: MeResponse
      try {
        // A read of its own, never the shared one: a read in flight was
        // sent with the ended session's cookie. Not through the retry
        // queue either: the dialog that would answer the wait is the
        // caller.
        me = await askMe(signal)
      } catch {
        throw new SignInError(null, keyMessage(SIGN_IN_FAILED_KEY))
      }
      if (me.pid !== expected.pid || me.scope !== "admin") {
        // Not kept: until the dialog's sign-out has ended this session,
        // the dialog keeps naming, and signing in, the account whose
        // session ended, and canAccess keeps answering for it.
        return "restart"
      }
      // The same account, whose access may have changed since the session
      // ended: keeping the answer re-asks ra-core's access questions then.
      if (sentIn === generation) {
        keep(me)
      }
      return "resumed"
    },

    async refreshSession(): Promise<RefreshOutcome> {
      let response: Response
      try {
        response = await fetch(`${apiBaseUrl}${REFRESH_PATH}`, {
          method: "POST",
          credentials: "same-origin",
          headers: { Accept: "application/json" },
          // Bounded, so the activity tracker can call again after its
          // interval (REFRESH_TIMEOUT_MS says why). A timed-out call
          // rejects with a `TimeoutError` and counts as failed below.
          signal: AbortSignal.timeout(refreshTimeoutMs),
        })
      } catch {
        return "failed"
      }
      return refreshOutcome(response.status)
    },

    async canAccess(params: {
      action: string
      resource: string
      record?: unknown
    }) {
      // The kept answer when there is one: no request per question
      // (ADR 0007 §7). Before the first one, every waiting question shares
      // one read. Its failure rejects, and ra-core then hides what it asked
      // about (useCanAccess) or, for a page (useRequireAccess), sends the
      // user to /authentication-error, never to a screen it could not
      // check; a plain "no" sends a page to /access-denied. App.tsx gives
      // both routes backend3's Hungarian pages (ra-core renders nothing
      // there by default). A session that ended under the open page keeps
      // its answer until the in-place sign-in settles (see the module
      // documentation), so the open page's questions never reject then.
      const me = current ?? (await readMe())
      return decideAccess(me, {
        action: params.action,
        resource: params.resource,
        record: params.record,
      })
    },
  }
}
