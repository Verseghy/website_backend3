/**
 * The console guard's one exception, `isExpectedHttpError` of `fixtures.ts`:
 * it may ignore only Chromium's failed-resource line of an answer that the
 * test listed, by status and path. Vitest does not run the E2E directory, so
 * these table tests run in the Playwright suite; they need no server.
 */
import { expect, isExpectedHttpError, test } from "./fixtures.ts"

const ME_401 = [{ status: 401, path: "/api/admin/auth/me" }] as const
const FAILED_401 =
  "Failed to load resource: the server responded with a status of 401 (Unauthorized)"
const ME_URL = "http://localhost:5150/api/admin/auth/me"

test.describe("the console guard's expected HTTP errors", () => {
  test("ignores a listed status of a listed path", () => {
    expect(isExpectedHttpError(FAILED_401, ME_URL, ME_401)).toBe(true)
    // The query string is not part of the path.
    expect(isExpectedHttpError(FAILED_401, `${ME_URL}?x=1`, ME_401)).toBe(true)
  })

  test("keeps every other console error", () => {
    for (const [text, url, expected] of [
      // Nothing listed: the default of every signed-in test.
      [FAILED_401, ME_URL, []],
      // Another status of the listed path.
      [
        "Failed to load resource: the server responded with a status of 403 (Forbidden)",
        ME_URL,
        ME_401,
      ],
      // The listed status of another path.
      [FAILED_401, "http://localhost:5150/api/admin/auth/login", ME_401],
      [FAILED_401, "http://localhost:5150/api/admin/auth/me/x", ME_401],
      // Not Chromium's failed-resource line.
      ["Uncaught Error: 401", ME_URL, ME_401],
      [
        "Refused to load the image because it violates the Content Security Policy",
        ME_URL,
        ME_401,
      ],
      // A network failure has no status.
      ["Failed to load resource: net::ERR_CONNECTION_REFUSED", ME_URL, ME_401],
      // No location to compare.
      [FAILED_401, "", ME_401],
    ] as const) {
      expect(
        isExpectedHttpError(text, url, expected),
        `${text} at ${url}`
      ).toBe(false)
    }
  })
})
