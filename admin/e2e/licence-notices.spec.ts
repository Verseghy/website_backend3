/**
 * The licence notices of the bundled code (ADR 0038), as a visitor of
 * `/admin` gets them: the file the Vite build wrote into `dist/`, served by
 * backend3's root-file row (ADR 0004 §3), with its UTF-8 charset stated, and
 * shown inline by the browser. Every test also passes the CSP and console
 * guard of `fixtures.ts`. The request tests decode non-ASCII text against the
 * fixture (`tests/requests/admin_ui.rs`); here the real file's non-ASCII
 * text is the licence file of the Hungarian strings, compared with the
 * committed copy.
 */
import { readFile } from "node:fs/promises"

import { expect, test } from "./fixtures.ts"

/** The committed licence file of the copied Hungarian strings. */
const HUNGARIAN_LICENCE = new URL(
  "../src/i18n/LICENSE.ra-language-hungarian",
  import.meta.url
)

test.describe("the licence notices served by backend3", () => {
  test("open as UTF-8 text and list the bundled packages and copied code", async ({
    page,
  }) => {
    // A download instead of a page would make goto throw.
    const response = await page.goto("/admin/licenses.txt")
    expect(response?.status()).toBe(200)
    expect(response?.headers()["content-type"]).toBe(
      "text/plain; charset=utf-8"
    )
    expect(response?.headers()["cache-control"]).toBe("no-cache")

    const text = await page.locator("body").innerText()
    expect(text.startsWith("# Licenses")).toBe(true)
    // A package of a JavaScript chunk, which Vite lists, and one of the
    // stylesheet, which the licenceNotices plugin adds.
    expect(text).toMatch(/^## ra-core - \S+ \(MIT\)$/m)
    expect(text).toMatch(/^## tailwindcss - \S+ \(MIT\)$/m)
    // The copied code's section, after the packages.
    expect(text).toMatch(/^# Code copied into the admin$/m)
    expect(text).toMatch(/^## src\/components\/admin\/LICENSE$/m)
    // Its accented copyright line arrives intact only if the browser decoded
    // the file as UTF-8.
    const hungarian = (await readFile(HUNGARIAN_LICENCE, "utf8")).trim()
    expect(
      [...hungarian].some((char) => (char.codePointAt(0) ?? 0) > 0x7f)
    ).toBe(true)
    expect(text).toContain(hungarian)
  })

  test("are named by the comment at the start of the admin's script", async ({
    page,
  }) => {
    const response = await page.goto("/admin")
    expect(response?.status()).toBe(200)
    const entry = await page
      .locator('script[type="module"][src^="/admin/assets/"]')
      .getAttribute("src")
    expect(entry).not.toBeNull()

    const script = await page.request.get(entry ?? "")
    expect(script.status()).toBe(200)
    expect(
      (await script.text()).startsWith(
        "/*! Licence notices of the bundled code: /admin/licenses.txt */"
      )
    ).toBe(true)
  })
})
