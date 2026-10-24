/**
 * The admin's i18n provider (ADR 0021 §3).
 *
 * One locale, `hu` („Magyar”), so the kit renders no language switcher. The
 * messages are `ra-language-english` with `hu.ts` laid over it: a key that
 * `hu.ts` lacks then shows English instead of a raw key. That is the only job
 * English has.
 *
 * Because of that merge, polyglot never sees a missing `ra.*` key and cannot
 * raise the alarm itself. So, outside production, the provider checks every
 * requested key against `hu.ts` alone and reports the ones it lacks. CI turns
 * the same gaps into failed tests (`hu.test.ts`, `key-usage.test.ts`), and the
 * E2E build reports them too (`report-missing-keys.ts`); production falls back
 * to English silently.
 */
import polyglotI18nProvider from "ra-i18n-polyglot"
import englishMessages from "ra-language-english"
import type { I18nProvider, TranslationMessages } from "ra-core"

import { hu, type MessageTree } from "./hu"
import { lookupMessage, mergeMessages } from "./messages"

/** The only locale of the admin UI. */
export const HU_LOCALE = "hu"

/**
 * Per-resource keys that ra-core and the kit look up speculatively, always
 * with a computed `_` fallback built from other, required keys (for example
 * `resources.posts.empty` falls back to `ra.page.empty`). A resource defines
 * them only to override that default, so their absence is not a defect.
 *
 * Collected from every `resources.${resource}.…` lookup in ra-core 5.15.5 and
 * shadcn-admin-kit 2.0.0. `resources.<r>.name` and `resources.<r>.fields.<f>`
 * are not optional: ADR 0021 §7 requires them for every registered resource.
 */
const OPTIONAL_RESOURCE_OVERRIDE =
  /^resources\.[^.]+\.(forcedCaseName|empty|invite|page\.[^.]+|action\.[^.]+|notifications\.[^.]+)$/

/**
 * A translation key is a dot-joined path of identifiers. ra-core also passes
 * already-readable text through `translate` (a label may be a key or free
 * text); such text is never a key, so it is not reported. The
 * `local/no-ui-literals` lint rule catches free text in the source instead.
 */
const KEY_SHAPE = /^[A-Za-z_][\w-]*(\.[\w-]+)+$/

/**
 * Decides whether a requested key is a defect worth reporting.
 *
 * @param key - the key ra-core or a component asked for.
 * @param hasFallback - whether the caller passed an inline `_` fallback.
 * @param messages - the Hungarian tree to check against (`hu.ts`).
 * @returns `true` when `key` looks like a key, is missing from `messages`, and
 *   is not an optional per-resource override that comes with a fallback.
 */
export function isReportableMissingKey(
  key: string,
  hasFallback: boolean,
  messages: MessageTree
): boolean {
  if (!KEY_SHAPE.test(key)) {
    return false
  }
  if (lookupMessage(messages, key) !== undefined) {
    return false
  }
  return !(hasFallback && OPTIONAL_RESOURCE_OVERRIDE.test(key))
}

/** Options of {@link createI18nProvider}. */
export interface I18nProviderOptions {
  /**
   * Report keys that `hu.ts` lacks. The composition root passes `true` in
   * development and tests, and in the CI E2E build (`VITE_REPORT_MISSING_KEYS=1`,
   * see `report-missing-keys.ts`), and `false` in other production builds.
   */
  readonly reportMissingKeys: boolean
  /**
   * Receives each missing key once per provider. Defaults to `console.error`,
   * which the unit-test setup turns into a failing test.
   */
  readonly onMissingKey?: (key: string) => void
  /**
   * The Hungarian tree. Only tests pass a different one; the app always uses
   * `hu.ts`.
   */
  readonly messages?: MessageTree
}

/**
 * Builds the admin's i18n provider.
 *
 * @param options - see {@link I18nProviderOptions}.
 * @returns an ra-core `I18nProvider` with the single locale `hu`.
 */
export function createI18nProvider(options: I18nProviderOptions): I18nProvider {
  const {
    reportMissingKeys,
    onMissingKey = (key: string) =>
      console.error(
        `Missing Hungarian translation: "${key}" is not in admin/src/i18n/hu.ts`
      ),
    messages = hu,
  } = options

  // ra-language-english is typed as ra-core's TranslationMessages, whose leaves
  // are strings too; the cast only widens it to the structural tree type.
  const merged = mergeMessages(
    englishMessages as unknown as MessageTree,
    messages
  )

  const base = polyglotI18nProvider(
    () => merged as unknown as TranslationMessages,
    HU_LOCALE,
    [{ locale: HU_LOCALE, name: "Magyar" }],
    // allowMissing: polyglot returns the key (or the caller's `_`) for a key
    // in neither tree instead of warning on every lookup; the reporter below
    // is the alarm.
    { allowMissing: true }
  )

  if (!reportMissingKeys) {
    return base
  }

  const reported = new Set<string>()
  return {
    ...base,
    translate: (key: string, translateOptions?: Record<string, unknown>) => {
      const hasFallback =
        translateOptions !== undefined && translateOptions._ !== undefined
      if (
        !reported.has(key) &&
        isReportableMissingKey(key, hasFallback, messages)
      ) {
        reported.add(key)
        onMissingKey(key)
      }
      return base.translate(key, translateOptions)
    },
  }
}
