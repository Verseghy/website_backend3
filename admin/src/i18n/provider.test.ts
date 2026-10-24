import { describe, expect, it, vi } from "vitest"

import { hu } from "./hu"
import {
  HU_LOCALE,
  createI18nProvider,
  isReportableMissingKey,
} from "./provider"

describe("createI18nProvider", () => {
  it("has exactly one locale, Hungarian, so the kit shows no language switcher", () => {
    const provider = createI18nProvider({ reportMissingKeys: false })
    expect(provider.getLocale()).toBe(HU_LOCALE)
    expect(provider.getLocales?.()).toEqual([{ locale: "hu", name: "Magyar" }])
  })

  it("translates ra-core keys into Hungarian", () => {
    const provider = createI18nProvider({ reportMissingKeys: false })
    expect(provider.translate("ra.action.save")).toBe("Mentés")
    expect(provider.translate("app.title")).toBe(hu.app.title)
  })

  it("interpolates and picks the Hungarian plural form", () => {
    const provider = createI18nProvider({ reportMissingKeys: false })
    expect(
      provider.translate("ra.notification.deleted", { smart_count: 1 })
    ).toBe("Törölve")
    expect(
      provider.translate("ra.notification.deleted", { smart_count: 3 })
    ).toBe("3 elem törölve")
    expect(
      provider.translate("ra.navigation.page_range_info", {
        offsetBegin: 1,
        offsetEnd: 10,
        total: 42,
      })
    ).toBe("1–10 / 42")
  })

  it("falls back to English for a ra-core key the Hungarian tree lacks, never to a raw key", () => {
    const provider = createI18nProvider({
      reportMissingKeys: false,
      messages: { ra: { action: { save: "Mentés" } } },
    })
    expect(provider.translate("ra.action.delete")).toBe("Delete")
  })

  it("reports a missing key once, outside production", () => {
    const onMissingKey = vi.fn()
    const provider = createI18nProvider({
      reportMissingKeys: true,
      onMissingKey,
      messages: { ra: { action: { save: "Mentés" } } },
    })
    provider.translate("ra.action.delete")
    provider.translate("ra.action.delete")
    provider.translate("ra.action.save")
    expect(onMissingKey).toHaveBeenCalledTimes(1)
    expect(onMissingKey).toHaveBeenCalledWith("ra.action.delete")
  })

  it("reports through console.error by default", () => {
    const error = vi.spyOn(console, "error").mockImplementation(() => {})
    const provider = createI18nProvider({
      reportMissingKeys: true,
      messages: {},
    })
    provider.translate("ra.action.save")
    expect(error).toHaveBeenCalledWith(
      expect.stringContaining('"ra.action.save" is not in admin/src/i18n/hu.ts')
    )
  })

  it("stays silent in production even for a missing key", () => {
    const onMissingKey = vi.fn()
    const provider = createI18nProvider({
      reportMissingKeys: false,
      onMissingKey,
      messages: {},
    })
    expect(provider.translate("ra.action.save")).toBe("Save")
    expect(onMissingKey).not.toHaveBeenCalled()
  })

  it("does not report anything for the real hu.ts on every English key", async () => {
    const onMissingKey = vi.fn()
    const provider = createI18nProvider({
      reportMissingKeys: true,
      onMissingKey,
    })
    const { default: english } = await import("ra-language-english")
    const { messageKeys } = await import("./messages")
    for (const key of messageKeys(english as never)) {
      provider.translate(key)
    }
    expect(onMissingKey).not.toHaveBeenCalled()
  })
})

describe("isReportableMissingKey", () => {
  const messages = { ra: { action: { save: "Mentés" } } }

  it("reports a key that is missing", () => {
    expect(isReportableMissingKey("ra.action.delete", false, messages)).toBe(
      true
    )
  })

  it("does not report a key that exists", () => {
    expect(isReportableMissingKey("ra.action.save", false, messages)).toBe(
      false
    )
  })

  it("does not report free text that ra-core passes through translate", () => {
    expect(
      isReportableMissingKey("Már lefordított szöveg", true, messages)
    ).toBe(false)
    expect(isReportableMissingKey("posts", true, messages)).toBe(false)
  })

  it("does not report an optional per-resource override that comes with a fallback", () => {
    for (const key of [
      "resources.posts.forcedCaseName",
      "resources.posts.empty",
      "resources.posts.invite",
      "resources.posts.page.list",
      "resources.posts.action.create",
      "resources.posts.notifications.deleted",
    ]) {
      expect(isReportableMissingKey(key, true, messages), key).toBe(false)
    }
  })

  it("reports an optional override when no fallback is given", () => {
    expect(
      isReportableMissingKey("resources.posts.empty", false, messages)
    ).toBe(true)
  })

  it("always reports a missing resource name or field label, which ADR 0021 §7 requires", () => {
    expect(isReportableMissingKey("resources.posts.name", true, messages)).toBe(
      true
    )
    expect(
      isReportableMissingKey("resources.posts.fields.title", true, messages)
    ).toBe(true)
  })
})
