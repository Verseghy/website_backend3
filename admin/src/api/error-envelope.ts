/**
 * Reads the admin API's error envelope and turns it into the message the
 * admin shows (ADR 0021 §4).
 *
 * Why it exists: every refused admin request answers
 * `{ error, description, field, params }`. The admin shows the Hungarian
 * `errors.<code>` message of `hu.ts`, filled from `params`, and shows the
 * server's English `description` only for a code this build does not know,
 * for example when the server was updated while an editor's tab still runs
 * the previous admin. The body is untrusted JSON from the network, so
 * {@link parseErrorEnvelope} checks every member before anything reads it.
 *
 * The sign-in screen is the first reader. The data provider's handlers
 * reject a refused request with {@link refusalError}, so ra-core's own
 * notification shows the same Hungarian message: a list that the account may
 * not see shows `errors.forbidden` (ADR 0007 §5), never an empty list.
 */
import { HttpError } from "ra-core"

import type { ErrorCode } from "@/bindings/ErrorCode"
import { hu } from "@/i18n/hu"
import type { MessageKey, MessageValues } from "@/i18n/keys"

/**
 * An error envelope as it arrived: the shape of the `ErrorEnvelope` binding,
 * except that `error` may be any string, because a newer server may send a
 * code that this build's `ErrorCode` lacks.
 */
export interface ReceivedErrorEnvelope {
  /** The stable error code. */
  readonly error: string
  /** English free text for script clients and logs. */
  readonly description: string
  /** The request field the error belongs to, or `null`. */
  readonly field: string | null
  /** Named values for the placeholders of the message. */
  readonly params: Readonly<Record<string, string | number>>
}

/**
 * A message ready for the screen: a key of `hu.ts` with its values, or a text
 * that came from the server and is shown as it is.
 */
export type UiMessage =
  | {
      readonly kind: "key"
      /** The `hu.ts` key, for `useT()`. */
      readonly key: MessageKey
      /** The placeholder values, `{}` when there are none. */
      readonly values: MessageValues
    }
  | {
      readonly kind: "text"
      /** The server's own text (English; ADR 0021 §4). */
      readonly text: string
    }

/**
 * Builds a {@link UiMessage} for a `hu.ts` key.
 *
 * @param key - the key; `useT()`'s type guarantees it exists.
 * @param values - the placeholder values, if the phrase has any.
 * @returns the message.
 */
export function keyMessage(
  key: MessageKey,
  values: MessageValues = {}
): UiMessage {
  return { kind: "key", key, values }
}

/**
 * Whether `value` is a plain object (not `null`, not an array).
 *
 * @param value - any JSON value.
 * @returns `true` for an object that can hold named members.
 */
function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value)
}

/**
 * Reads a response body as an error envelope.
 *
 * @param body - the parsed JSON body of a refused request, of any shape.
 * @returns the envelope when every member has the type ADR 0021 §4 gives it
 *   (`params` values are strings or finite numbers), otherwise `undefined`,
 *   for example for a proxy's HTML error page that the caller could not
 *   parse as JSON at all, or a body of another shape.
 */
export function parseErrorEnvelope(
  body: unknown
): ReceivedErrorEnvelope | undefined {
  if (!isRecord(body)) {
    return undefined
  }
  const { error, description, field, params } = body
  if (
    typeof error !== "string" ||
    error === "" ||
    typeof description !== "string" ||
    (field !== null && typeof field !== "string") ||
    !isRecord(params)
  ) {
    return undefined
  }
  const checkedParams: Record<string, string | number> = {}
  for (const [name, value] of Object.entries(params)) {
    if (
      typeof value !== "string" &&
      !(typeof value === "number" && Number.isFinite(value))
    ) {
      return undefined
    }
    checkedParams[name] = value
  }
  return { error, description, field, params: checkedParams }
}

/**
 * Whether this build has a Hungarian message for `code`.
 *
 * @param code - an error code as the server sent it.
 * @returns `true` when `hu.ts` has `errors.<code>`, which the type check ties
 *   to the `ErrorCode` binding (`satisfies Record<ErrorCode, string>`).
 */
export function isKnownErrorCode(code: string): code is ErrorCode {
  return Object.hasOwn(hu.errors, code)
}

/**
 * The `hu.ts` key of an error code.
 *
 * @param code - a code that {@link isKnownErrorCode} accepted.
 * @returns `errors.<code>`.
 */
export function errorCodeKey(code: ErrorCode): MessageKey {
  return `errors.${code}`
}

/**
 * The message to show for an envelope: `errors.<code>` with the envelope's
 * `params`, or, for a code this build does not know, the server's
 * `description` (ADR 0021 §4).
 *
 * @param envelope - a parsed envelope.
 * @returns the message.
 */
export function envelopeMessage(envelope: ReceivedErrorEnvelope): UiMessage {
  if (isKnownErrorCode(envelope.error)) {
    return keyMessage(errorCodeKey(envelope.error), envelope.params)
  }
  return { kind: "text", text: envelope.description }
}

/**
 * What ra-core shows for a refused request whose answer carries no readable
 * envelope (a proxy's page, Loco's own `500` body, an empty answer).
 */
export const REFUSED_WITHOUT_ENVELOPE_KEY =
  "ra.notification.http_error" satisfies MessageKey

/**
 * The rejection of a data-provider call that the admin API refused: an
 * ra-core `HttpError`, which ra-core passes to `authProvider.checkError` and
 * whose `message` its notification translates and shows (ADR 0021 §4).
 *
 * - `status` is the answer's status, which `checkError` reads: only `401`
 *   signs the user out, so a `403 forbidden` keeps the session (ADR 0007 §7).
 * - `message` is the `errors.<code>` key of a known code, the server's
 *   `description` for a code this build does not know (shown as it is: it is
 *   not shaped like a key), and {@link REFUSED_WITHOUT_ENVELOPE_KEY} without
 *   an envelope.
 * - `body` is the parsed envelope, or `null` without one, so a form can show
 *   a field's message at its input with {@link envelopeMessage}. ra-core's
 *   notification fills no placeholder, so a code whose message needs
 *   `params` is shown by the screen from `body`, not by the notification;
 *   `forbidden` and the other whole-request refusals need none.
 *
 * @param status - the HTTP status of the answer, `4xx` or `5xx`.
 * @param body - the parsed JSON body, or `undefined` when it was not JSON.
 * @returns the rejection.
 */
export function refusalError(status: number, body: unknown): HttpError {
  const envelope = parseErrorEnvelope(body)
  if (envelope === undefined) {
    return new HttpError(REFUSED_WITHOUT_ENVELOPE_KEY, status, null)
  }
  const message = envelopeMessage(envelope)
  return new HttpError(
    message.kind === "key" ? message.key : message.text,
    status,
    envelope
  )
}
