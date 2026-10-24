/**
 * Typed translation keys for backend3's own code (ADR 0021 §7, "Key usage").
 *
 * `MessageKey` is every dotted leaf path of `hu.ts`, computed by the compiler.
 * backend3's components translate through {@link useT}, so a typo or a key
 * that `hu.ts` lacks fails `npm run typecheck` instead of showing English (or a
 * raw key) to an editor. The kit's copied components keep ra-core's untyped
 * `useTranslate`; `key-usage.test.ts` covers their literal keys.
 */
import { useCallback } from "react"
import { useTranslate } from "ra-core"

import type { hu } from "./hu"

/** Dotted leaf paths of a nested object type whose leaves are strings. */
type LeafPaths<Tree, Prefix extends string = ""> = {
  [Name in keyof Tree & string]: Tree[Name] extends string
    ? `${Prefix}${Name}`
    : LeafPaths<Tree[Name], `${Prefix}${Name}.`>
}[keyof Tree & string]

/** Every key that `hu.ts` defines, for example `"app.footer.source_code"`. */
export type MessageKey = LeafPaths<typeof hu>

/**
 * Interpolation values for a phrase. `smart_count` selects the plural form;
 * `_` is deliberately absent: backend3's own code never supplies an inline
 * fallback text, because every key exists in `hu.ts`.
 */
export type MessageValues = Readonly<Record<string, string | number>>

/** A translate function restricted to {@link MessageKey}. */
export type TypedTranslate = (key: MessageKey, values?: MessageValues) => string

/**
 * Returns a translate function that accepts only keys defined in `hu.ts`.
 *
 * It wraps ra-core's `useTranslate`, so it must be called inside the admin
 * (or a test wrapper that provides an i18n context). The returned function is
 * stable across renders as long as the i18n provider does not change.
 *
 * @returns a {@link TypedTranslate} bound to the current i18n provider.
 */
export function useT(): TypedTranslate {
  const translate = useTranslate()
  return useCallback<TypedTranslate>(
    (key, values) => translate(key, values),
    [translate]
  )
}
