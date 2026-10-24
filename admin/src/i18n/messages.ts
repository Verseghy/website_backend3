/**
 * Pure helpers over polyglot message trees.
 *
 * They exist so that the provider (`provider.ts`) and the completeness checks
 * (`hu.test.ts`, `key-usage.test.ts`) agree on what a "key" is: a dot-joined
 * path from the root of a tree to a string leaf, as ra-core and polyglot use
 * it (`ra.action.save`).
 */
import type { MessageTree } from "./hu"

/**
 * Lists every leaf key of a message tree.
 *
 * @param tree - the message tree to walk.
 * @param prefix - the dotted path of `tree` itself; callers leave it empty.
 * @returns the dotted keys of all string leaves, in insertion order.
 */
export function messageKeys(tree: MessageTree, prefix = ""): string[] {
  return Object.entries(tree).flatMap(([name, value]) =>
    typeof value === "string"
      ? [`${prefix}${name}`]
      : messageKeys(value, `${prefix}${name}.`)
  )
}

/**
 * Looks up the phrase stored under a dotted key.
 *
 * @param tree - the message tree to search.
 * @param key - a dotted key such as `ra.action.save`.
 * @returns the phrase, or `undefined` when the key is absent or names an
 *   inner node rather than a leaf.
 */
export function lookupMessage(
  tree: MessageTree,
  key: string
): string | undefined {
  let node: string | MessageTree | undefined = tree
  for (const part of key.split(".")) {
    if (
      node === undefined ||
      typeof node === "string" ||
      !Object.hasOwn(node, part)
    ) {
      return undefined
    }
    node = node[part]
  }
  return typeof node === "string" ? node : undefined
}

/**
 * Deep-merges two message trees without mutating either.
 *
 * Leaves of `overlay` win. A leaf in one tree never replaces an inner node of
 * the other (or the reverse): that would silently drop a whole namespace, so
 * it throws instead.
 *
 * @param base - the tree that supplies keys `overlay` lacks (the English pack).
 * @param overlay - the tree whose phrases win (the Hungarian file).
 * @returns a new tree with every key of both inputs.
 * @throws {Error} when the two trees disagree on whether a key is a leaf.
 */
export function mergeMessages(
  base: MessageTree,
  overlay: MessageTree
): MessageTree {
  const merged: Record<string, string | MessageTree> = { ...base }
  for (const [name, value] of Object.entries(overlay)) {
    const existing = merged[name]
    if (existing === undefined) {
      merged[name] = value
    } else if (typeof existing === "string" && typeof value === "string") {
      merged[name] = value
    } else if (typeof existing !== "string" && typeof value !== "string") {
      merged[name] = mergeMessages(existing, value)
    } else {
      throw new Error(
        `Message trees disagree on "${name}": one side has a phrase and the other a namespace. ` +
          `Rename the key in admin/src/i18n/hu.ts so it matches the shape of ra-language-english.`
      )
    }
  }
  return merged
}

/**
 * Extracts the interpolation variables of a polyglot phrase.
 *
 * @param phrase - a phrase such as `"%{name} törlése"`; plural forms separated
 *   by `||||` are all scanned.
 * @returns the distinct variable names, sorted.
 */
export function placeholders(phrase: string): string[] {
  const names = new Set<string>()
  for (const match of phrase.matchAll(/%\{([^}]+)\}/g)) {
    names.add(match[1])
  }
  return [...names].sort()
}

/**
 * Counts the plural forms of a polyglot phrase.
 *
 * @param phrase - a phrase whose forms are separated by `||||`.
 * @returns the number of forms (1 for a phrase without `||||`).
 */
export function pluralFormCount(phrase: string): number {
  return phrase.split("||||").length
}
