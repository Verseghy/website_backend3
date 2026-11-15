import fc from "fast-check"
import { HttpError } from "ra-core"
import { describe, expect, it } from "vitest"

import type { ErrorCode } from "@/bindings/ErrorCode"
import type { ErrorEnvelope } from "@/bindings/ErrorEnvelope"
import { hu } from "@/i18n/hu"

import {
  REFUSED_WITHOUT_ENVELOPE_KEY,
  envelopeMessage,
  errorCodeKey,
  isKnownErrorCode,
  keyMessage,
  parseErrorEnvelope,
  refusalError,
} from "./error-envelope"

/** Every code of the binding, from the message file that must cover it. */
const CODES = Object.keys(hu.errors) as ErrorCode[]

const ENVELOPE: ErrorEnvelope = {
  error: "invalid_body",
  description: "The request body is not valid JSON.",
  field: null,
  params: {},
}

describe("parseErrorEnvelope", () => {
  it("reads the envelope the server sends", () => {
    expect(parseErrorEnvelope(ENVELOPE)).toEqual(ENVELOPE)
    expect(
      parseErrorEnvelope({
        error: "post.date_required",
        description: "d",
        field: "publishedAt",
        params: { days: 12, name: "x" },
      })
    ).toEqual({
      error: "post.date_required",
      description: "d",
      field: "publishedAt",
      params: { days: 12, name: "x" },
    })
  })

  it("drops members that the envelope does not have", () => {
    expect(parseErrorEnvelope({ ...ENVELOPE, extra: true })).toEqual(ENVELOPE)
  })

  it.each([
    ["null", null],
    ["a string", "invalid_body"],
    ["an array", [ENVELOPE]],
    ["no code", { ...ENVELOPE, error: undefined }],
    ["an empty code", { ...ENVELOPE, error: "" }],
    ["a numeric code", { ...ENVELOPE, error: 401 }],
    ["no description", { ...ENVELOPE, description: undefined }],
    ["a missing field", { ...ENVELOPE, field: undefined }],
    ["a numeric field", { ...ENVELOPE, field: 3 }],
    ["no params", { ...ENVELOPE, params: undefined }],
    ["array params", { ...ENVELOPE, params: [] }],
    ["a nested param", { ...ENVELOPE, params: { a: { b: 1 } } }],
    ["a boolean param", { ...ENVELOPE, params: { a: true } }],
    ["a null param", { ...ENVELOPE, params: { a: null } }],
    ["an infinite param", { ...ENVELOPE, params: { a: Infinity } }],
  ])("refuses %s", (_case, body) => {
    expect(parseErrorEnvelope(body)).toBeUndefined()
  })

  it("never throws, and keeps what it accepts", () => {
    fc.assert(
      fc.property(fc.jsonValue(), (body) => {
        const parsed = parseErrorEnvelope(body)
        if (parsed !== undefined) {
          expect(typeof parsed.error).toBe("string")
          expect(parsed.error).not.toBe("")
          expect(typeof parsed.description).toBe("string")
          for (const value of Object.values(parsed.params)) {
            expect(["string", "number"]).toContain(typeof value)
          }
        }
      })
    )
  })
})

describe("isKnownErrorCode", () => {
  it.each(CODES)("knows %s, which hu.ts translates", (code) => {
    expect(isKnownErrorCode(code)).toBe(true)
  })

  it.each(["too_many_attempts", "post.published_locked", "toString", ""])(
    "does not know %j",
    (code) => {
      expect(isKnownErrorCode(code)).toBe(false)
    }
  )
})

describe("envelopeMessage", () => {
  it.each(CODES)("shows errors.%s for a known code", (code) => {
    expect(envelopeMessage({ ...ENVELOPE, error: code })).toEqual(
      keyMessage(errorCodeKey(code))
    )
    expect(errorCodeKey(code)).toBe(`errors.${code}`)
  })

  it("passes the params on to the message", () => {
    expect(
      envelopeMessage({ ...ENVELOPE, error: "not_found", params: { id: 7 } })
    ).toEqual({ kind: "key", key: "errors.not_found", values: { id: 7 } })
  })

  it("shows the server's description for a code this build does not know", () => {
    expect(
      envelopeMessage({
        ...ENVELOPE,
        error: "too_many_attempts",
        description: "Too many attempts; try again later.",
      })
    ).toEqual({ kind: "text", text: "Too many attempts; try again later." })
  })

  it("uses a hu.ts key for every known code and the description otherwise", () => {
    fc.assert(
      fc.property(
        fc.oneof(fc.constantFrom(...CODES), fc.string({ minLength: 1 })),
        fc.string(),
        (code, description) => {
          const message = envelopeMessage({
            error: code,
            description,
            field: null,
            params: {},
          })
          if (isKnownErrorCode(code)) {
            expect(message).toEqual(keyMessage(`errors.${code}`))
          } else {
            expect(message).toEqual({ kind: "text", text: description })
          }
        }
      )
    )
  })
})

describe("refusalError", () => {
  const FORBIDDEN: ErrorEnvelope = {
    error: "forbidden",
    description: "The signed-in account may not perform this operation",
    field: null,
    params: {},
  }

  it("turns a 403 forbidden into the Hungarian key, keeping the status and the envelope", () => {
    const error = refusalError(403, FORBIDDEN)
    expect(error).toBeInstanceOf(HttpError)
    expect(error.message).toBe("errors.forbidden")
    expect(error.status).toBe(403)
    expect(error.body).toEqual(FORBIDDEN)
    // The key ra-core's notification translates is the ADR 0007 §5 message.
    expect(hu.errors.forbidden).toBe("Nincs jogosultságod ehhez a művelethez.")
  })

  it("shows the server's description for a code this build does not know", () => {
    const envelope = {
      ...FORBIDDEN,
      error: "post.published_locked",
      description: "Publishing needs the publish posts permission",
    }
    const error = refusalError(403, envelope)
    expect(error.message).toBe("Publishing needs the publish posts permission")
    expect(error.body).toEqual(envelope)
  })

  it("falls back to ra-core's HTTP error message without an envelope", () => {
    for (const body of [undefined, null, "<html>", { error: "forbidden" }]) {
      const error = refusalError(502, body)
      expect(error.message).toBe(REFUSED_WITHOUT_ENVELOPE_KEY)
      expect(error.status).toBe(502)
      expect(error.body).toBeNull()
    }
    expect(REFUSED_WITHOUT_ENVELOPE_KEY).toBe("ra.notification.http_error")
  })

  it("keeps the status and reads the message as envelopeMessage does", () => {
    fc.assert(
      fc.property(
        fc.integer({ min: 400, max: 599 }),
        fc.oneof(fc.constantFrom(...CODES), fc.string({ minLength: 1 })),
        fc.string(),
        (status, code, description) => {
          const envelope = { error: code, description, field: null, params: {} }
          const error = refusalError(status, envelope)
          expect(error.status).toBe(status)
          expect(error.body).toEqual(envelope)
          const message = envelopeMessage(envelope)
          expect(error.message).toBe(
            message.kind === "key" ? message.key : message.text
          )
        }
      )
    )
  })
})
