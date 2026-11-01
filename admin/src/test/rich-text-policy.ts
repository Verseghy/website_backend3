/**
 * Test support for code built on the content-v1 policy
 * (`src/rich-text/policy.ts`).
 *
 * - {@link policyWith} gives a changed copy of the committed policy, to
 *   check that code follows the policy rather than restating it.
 * - {@link writeDeclarations} writes declarations as a canonical style
 *   string (editor.md §3.1 rule 5), to compare them with
 *   `css-vectors.jsonl`; the stored-HTML serialiser's `canonicalStyle` is
 *   the production writer.
 * - {@link readVectors} reads `css-vectors.jsonl`.
 */
import vectorsText from "@/rich-text/css-vectors.jsonl?raw"
import policyJson from "@/rich-text/policy.json"
import type { RichTextPolicy } from "@/rich-text/policy"
import { readPolicy } from "@/rich-text/policy"

/** A JSON value, for editing the manifest. */
export type Json =
  null | boolean | number | string | Json[] | { [key: string]: Json }

/** A fresh, mutable copy of the committed `policy.json`. */
export function policyJsonCopy(): { [key: string]: Json } {
  return structuredClone(policyJson) as unknown as { [key: string]: Json }
}

/**
 * The committed policy with a change applied to its JSON first.
 *
 * @param change - edits the mutable JSON copy in place.
 * @returns the validated policy.
 */
export function policyWith(
  change: (json: { [key: string]: Json }) => void
): RichTextPolicy {
  const json = policyJsonCopy()
  change(json)
  return readPolicy(json)
}

/**
 * Writes declarations as a canonical style string: sorted by property,
 * `prop: value`, joined with `; `.
 *
 * @param declarations - the declarations by property.
 * @returns the style string, or `null` when there is none.
 */
export function writeDeclarations(
  declarations: ReadonlyMap<string, string>
): string | null {
  if (declarations.size === 0) {
    return null
  }
  return [...declarations]
    .sort(([a], [b]) => (a < b ? -1 : a > b ? 1 : 0))
    .map(([property, value]) => `${property}: ${value}`)
    .join("; ")
}

/** One line of `css-vectors.jsonl`. */
export interface CssVector {
  /** The element the style is on. */
  readonly element: string
  /** The style attribute as written. */
  readonly input: string
  /** The canonical string the server writes, or `null` when it drops it. */
  readonly output: string | null
}

/** Every line of the committed `css-vectors.jsonl`. */
export function readVectors(): CssVector[] {
  return vectorsText
    .split("\n")
    .filter((line) => line !== "")
    .map((line) => JSON.parse(line) as CssVector)
}
