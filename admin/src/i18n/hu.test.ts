// @vitest-environment node
/**
 * Completeness checks of the Hungarian message file (ADR 0021 §7).
 *
 * "Key parity" is the first test: every key of the installed
 * `ra-language-english` must exist in `hu.ts`. It prints the missing keys, so
 * a ra-core upgrade that adds strings fails with the exact list to translate.
 */
import { readFileSync } from "node:fs"
import { resolve } from "node:path"

import englishMessages from "ra-language-english"
import { describe, expect, it } from "vitest"

import { hu, type MessageTree } from "./hu"
import {
  lookupMessage,
  messageKeys,
  placeholders,
  pluralFormCount,
} from "./messages"

const english = englishMessages as unknown as MessageTree
const englishKeys = messageKeys(english)
const hungarianKeys = messageKeys(hu)

/**
 * Keys that shadcn-admin-kit v2.0.0 translates but `ra-language-english`
 * lacks (ADR 0021 Context). The kit supplies inline English defaults for
 * them, so `hu.ts` must define them too.
 */
const KIT_ONLY_RA_KEYS = [
  "ra.action.clear_search",
  "ra.navigation.breadcrumb_drawer_instructions",
  "ra.navigation.breadcrumb_drawer_title",
]

describe("hu.ts", () => {
  it("defines every key of the installed ra-language-english (key parity)", () => {
    const missing = englishKeys.filter(
      (key) => lookupMessage(hu, key) === undefined
    )
    expect(
      missing,
      `Add these keys to admin/src/i18n/hu.ts:\n${missing.join("\n")}`
    ).toEqual([])
  })

  it("covers the English pack size recorded in ADR 0021 for ra-core 5.15.5", () => {
    // A changed count means ra-core was upgraded: re-run the parity review.
    expect(englishKeys).toHaveLength(164)
  })

  it("defines the kit-only keys", () => {
    for (const key of KIT_ONLY_RA_KEYS) {
      expect(
        englishKeys,
        `${key} is now in ra-language-english; drop it from KIT_ONLY_RA_KEYS`
      ).not.toContain(key)
      expect(lookupMessage(hu, key), key).toBeDefined()
    }
  })

  it("has no ra.* key that neither ra-core nor the kit uses (catches typos and stale keys)", () => {
    const known = new Set([...englishKeys, ...KIT_ONLY_RA_KEYS])
    const stale = hungarianKeys.filter(
      (key) => key.startsWith("ra.") && !known.has(key)
    )
    expect(stale).toEqual([])
  })

  it("uses only placeholders that the English phrase also has", () => {
    const unknown = englishKeys.flatMap((key) => {
      const huPhrase = lookupMessage(hu, key) ?? ""
      const allowed = new Set(placeholders(lookupMessage(english, key) ?? ""))
      return placeholders(huPhrase)
        .filter((name) => !allowed.has(name))
        .map((name) => `${key}: %{${name}}`)
    })
    expect(unknown).toEqual([])
  })

  it("never has more plural forms than polyglot's Hungarian rule (one, other)", () => {
    const tooMany = hungarianKeys.filter(
      (key) => pluralFormCount(lookupMessage(hu, key) ?? "") > 2
    )
    expect(tooMany).toEqual([])
  })

  it("has no empty phrase (only ra.boolean.null is deliberately a blank space)", () => {
    const empty = hungarianKeys.filter(
      (key) =>
        key !== "ra.boolean.null" &&
        (lookupMessage(hu, key) ?? "").trim() === ""
    )
    expect(empty).toEqual([])
  })

  it("uses Hungarian quotation marks, never straight double quotes", () => {
    const straight = hungarianKeys.filter((key) =>
      (lookupMessage(hu, key) ?? "").includes('"')
    )
    expect(straight).toEqual([])
  })

  it("never uses „menü” alone, which clashes with the canteen's dishes (ADR 0021 §6)", () => {
    const clashes = hungarianKeys.filter((key) =>
      /(^|[^\p{L}])menü([^\p{L}]|$)/iu.test(lookupMessage(hu, key) ?? "")
    )
    expect(clashes).toEqual([])
  })

  it("has a message for the admin API's error codes (ADR 0021 §4)", () => {
    // The compiler enforces completeness (`satisfies Record<ErrorCode, string>`);
    // this pins the wording of each code.
    expect(hu.errors.not_found).toBe("A keresett elem nem található.")
    // One generic message for every failed sign-in (ADR 0006 §4).
    expect(hu.errors.invalid_credentials).toBe("Hibás e-mail-cím vagy jelszó.")
    // A body that is not the endpoint's JSON, and one above the size limit.
    expect(hu.errors.invalid_body).toBe(
      "A kérés formátuma hibás. Frissítsd az oldalt, és próbáld újra!",
    )
    expect(hu.errors.body_too_large).toBe("Az elküldött adat túl nagy.")
  })

  it("keeps the ADR 0004 §5 update prompt verbatim", () => {
    expect(hu.app.update.available).toBe(
      "Frissítés elérhető. Kérlek töltsd újra az oldalt!"
    )
  })
})

describe("index.html", () => {
  const html = readFileSync(
    resolve(import.meta.dirname, "../../index.html"),
    "utf8"
  )

  it("declares the page language as Hungarian (ADR 0021 §1)", () => {
    expect(html).toMatch(/<html lang="hu">/)
  })

  it("carries the same title as hu.ts, shown before the script sets it", () => {
    expect(html).toContain(`<title>${hu.app.title}</title>`)
  })
})
