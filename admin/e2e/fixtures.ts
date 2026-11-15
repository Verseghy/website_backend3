/**
 * The E2E `test` with a guard that fails any test whose page reports a
 * Content-Security-Policy violation, a console error or an uncaught exception
 * (ADR 0004 "What must be true", ADR 0023 §6), and the sign-in helper of the
 * signed-in tests.
 *
 * CSP violations are collected twice on purpose: Chromium logs each one as a
 * console error, and the page's `securitypolicyviolation` events carry the
 * violated directive and the blocked URL, which make the failure readable.
 *
 * Chromium also logs every HTTP error answer of a request as a console error
 * ("Failed to load resource: the server responded with a status of 401"),
 * and Playwright reports those log entries like `console.error` calls. A test
 * of the signed-out flows expects such answers: `/api/admin/auth/me` answers
 * `401` while nobody is signed in, and `/api/admin/auth/login` answers `401`
 * to a wrong password. Such a test lists them in `expectedHttpErrors`, by
 * status and path; the guard then ignores exactly those log entries and
 * still fails on every other console error, a `401` of any other path
 * included. The default is none, so a signed-in test fails on any of them.
 */
import { test as base, expect, type Page } from "@playwright/test"

import { SEED_ACCOUNT } from "./seed-account.ts"

/** One CSP violation as the browser reports it. */
interface CspViolation {
  readonly directive: string
  readonly blockedURI: string
  readonly sourceFile: string
}

/** An HTTP error answer that a test expects the browser to log. */
export interface ExpectedHttpError {
  /** The status, for example 401. */
  readonly status: number
  /** The request's path, for example `/api/admin/auth/me`. */
  readonly path: string
}

declare global {
  interface Window {
    /** Filled by the init script below; read once the test body is done. */
    __cspViolations?: CspViolation[]
  }
}

/** Chromium's log line for a request that got an HTTP error answer. */
const FAILED_RESOURCE =
  /^Failed to load resource: the server responded with a status of (\d{3})\b/

/**
 * Whether a console error is one of the HTTP error answers a test expects.
 *
 * @param text - the console message's text.
 * @param url - the URL the message names as its location.
 * @param expected - the answers the test expects.
 * @returns `true` only for Chromium's failed-resource line of a listed
 *   status and path.
 */
export function isExpectedHttpError(
  text: string,
  url: string,
  expected: readonly ExpectedHttpError[]
): boolean {
  const match = FAILED_RESOURCE.exec(text)
  if (match === null || url === "") {
    return false
  }
  const status = Number(match[1])
  const path = new URL(url).pathname
  return expected.some(
    (answer) => answer.status === status && answer.path === path
  )
}

export const test = base.extend<{
  expectedHttpErrors: readonly ExpectedHttpError[]
  pageGuard: void
}>({
  expectedHttpErrors: [[], { option: true }],
  pageGuard: [
    async ({ page, expectedHttpErrors }, use) => {
      const problems: string[] = []
      page.on("console", (message) => {
        if (
          message.type() === "error" &&
          !isExpectedHttpError(
            message.text(),
            message.location().url,
            expectedHttpErrors
          )
        ) {
          problems.push(`console.error: ${message.text()}`)
        }
      })
      page.on("pageerror", (error) =>
        problems.push(`uncaught exception: ${error.message}`)
      )
      await page.addInitScript(() => {
        window.__cspViolations = []
        document.addEventListener("securitypolicyviolation", (event) => {
          window.__cspViolations?.push({
            directive: event.effectiveDirective,
            blockedURI: event.blockedURI,
            sourceFile: event.sourceFile,
          })
        })
      })

      await use()

      // The page may already be closed or mid-navigation; that is not a CSP signal.
      const violations = await page
        .evaluate(() => window.__cspViolations ?? [])
        .catch(() => [] as CspViolation[])
      for (const { directive, blockedURI, sourceFile } of violations) {
        problems.push(
          `CSP violation: ${directive} blocked ${blockedURI || "(inline)"} in ${sourceFile || "(document)"}`
        )
      }
      expect(problems, "the page reported CSP violations or errors").toEqual([])
    },
    { auto: true },
  ],
})

/**
 * Signs in with the seed account through the sign-in screen and waits for
 * the dashboard. It opens `/admin/login` directly, which asks no session
 * check, so a signed-in test sees no `401` at all.
 *
 * @param page - the test's page; its context then holds the session cookie.
 */
export async function signIn(page: Page): Promise<void> {
  await page.goto("/admin/login")
  await page.getByLabel("E-mail-cím").fill(SEED_ACCOUNT.email)
  await page.getByLabel("Jelszó").fill(SEED_ACCOUNT.password)
  await page.getByRole("button", { name: "Bejelentkezés" }).click()
  await expect(
    page.getByTestId("dashboard"),
    "the seed account did not sign in: run `LOCO_ENV=e2e cargo loco db seed` first"
  ).toBeVisible()
}

export { expect }
