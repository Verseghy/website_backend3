/**
 * The E2E `test` with a guard that fails any test whose page reports a
 * Content-Security-Policy violation, a console error or an uncaught exception
 * (ADR 0004 "What must be true", ADR 0023 §6).
 *
 * CSP violations are collected twice on purpose: Chromium logs each one as a
 * console error, and the page's `securitypolicyviolation` events carry the
 * violated directive and the blocked URL, which make the failure readable.
 */
import { test as base, expect } from "@playwright/test"

/** One CSP violation as the browser reports it. */
interface CspViolation {
  readonly directive: string
  readonly blockedURI: string
  readonly sourceFile: string
}

declare global {
  interface Window {
    /** Filled by the init script below; read once the test body is done. */
    __cspViolations?: CspViolation[]
  }
}

export const test = base.extend<{ pageGuard: void }>({
  pageGuard: [
    async ({ page }, use) => {
      const problems: string[] = []
      page.on("console", (message) => {
        if (message.type() === "error") {
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

export { expect }
