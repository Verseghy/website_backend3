/**
 * PadletEmbed's reader of Padlet input (editor.md §4.3 row 10).
 *
 * A Padlet board is embedded as `https://padlet.com/embed/<key>`, the one
 * Padlet address that content-v1 stores (editor.md §3.6, §3.7). The parse
 * rules read it from an iframe's `src`, and the insert dialog from a URL
 * that staff paste. The reader takes exactly that address: `https`, the
 * provider's host and path from `policy.json`, a key of the id alphabet, no
 * query and no fragment.
 *
 * Padlet's own share links and its embed snippet (a wrapper `div` and a
 * "Made with Padlet" link around the iframe) have not been captured yet
 * (editor.md §16 item 2), so no other Padlet address is mapped to a board
 * here: reading a key out of an uncaptured link shape could embed the wrong
 * board. When the capture arrives, the mapping joins this module and the
 * paste filter reduces the snippet to its iframe (§4.3 row 10).
 */
import type { EmbedProvider, RichTextPolicy } from "./policy"
import { PolicyError } from "./policy"

/** The host of Padlet's embed addresses (editor.md §3.6). */
export const PADLET_HOST = "padlet.com"

/**
 * The path prefix of every Padlet address the node writes (editor.md §3.7).
 * {@link padletRules} refuses a policy whose Padlet prefix differs.
 */
export const PADLET_EMBED_PATH = "/embed/"

/** A board's height when its iframe gives none (editor.md §3.7). */
export const PADLET_DEFAULT_HEIGHT = 608

/** The title an inserted board gets (editor.md §3.7). */
export const PADLET_DEFAULT_TITLE = "Padlet"

/** The Padlet rules of `policy.json` that the reader applies. */
export interface PadletRules {
  /** The provider's hosts. */
  readonly hosts: readonly string[]
  /** The path prefix before the key (`/embed/`). */
  readonly pathPrefix: string
  /** The characters of a key. */
  readonly idAlphabet: string
  /** The key's exact length, or `null` for any length. */
  readonly idLength: number | null
}

/**
 * The Padlet rules of a policy: the provider whose hosts include
 * {@link PADLET_HOST}.
 *
 * @param policy - the content-v1 policy.
 * @returns the rules the reader applies.
 * @throws {PolicyError} when the policy has no such provider, or it allows a
 *   query, which the node never writes, or its path prefix is not
 *   {@link PADLET_EMBED_PATH}, the one the node writes.
 */
export function padletRules(policy: RichTextPolicy): PadletRules {
  const provider: EmbedProvider | undefined = policy.urls.embed.providers.find(
    (candidate) => candidate.hosts.includes(PADLET_HOST)
  )
  // The node writes its own address (padletEmbedSrc) on PADLET_HOST, which
  // finding the provider by that host guarantees the policy allows; a policy
  // with another prefix stops the schema here, as in youtubeRules.
  if (
    provider === undefined ||
    provider.path_prefix !== PADLET_EMBED_PATH ||
    provider.query.length > 0
  ) {
    throw new PolicyError(
      `The policy has no Padlet embed provider on ${PADLET_HOST} with the path prefix ${PADLET_EMBED_PATH} and without a query (editor.md §3.6), which the Padlet embed reads; found ${JSON.stringify(provider ?? null)}.`
    )
  }
  return {
    hosts: provider.hosts,
    pathPrefix: provider.path_prefix,
    idAlphabet: policy.urls.embed.id_alphabet,
    idLength: provider.id_length,
  }
}

/**
 * Whether a key is one the server keeps in a Padlet address.
 *
 * @param key - the candidate.
 * @param rules - the policy's Padlet rules ({@link padletRules}).
 * @returns `true` when the node may write it.
 */
export function isPadletKey(key: string, rules: PadletRules): boolean {
  return (
    key.length > 0 &&
    [...key].every((c) => rules.idAlphabet.includes(c)) &&
    (rules.idLength === null || key.length === rules.idLength)
  )
}

/**
 * The canonical address of a board (editor.md §3.7).
 *
 * @param key - the board's key.
 * @returns the `src` the node writes.
 */
export function padletEmbedSrc(key: string): string {
  return `https://${PADLET_HOST}${PADLET_EMBED_PATH}${key}`
}

/**
 * Reads a Padlet embed address, from an iframe's `src` or a pasted URL.
 *
 * @param text - the address; leading and trailing whitespace is ignored.
 * @param rules - the policy's Padlet rules ({@link padletRules}).
 * @returns the board's key, or `null` when the text is no Padlet embed
 *   address the server keeps.
 */
export function readPadletAddress(
  text: string,
  rules: PadletRules
): string | null {
  let url: URL
  try {
    url = new URL(text.trim())
  } catch {
    return null
  }
  const plain =
    url.protocol === "https:" &&
    url.username === "" &&
    url.password === "" &&
    url.port === "" &&
    // A bare `?` or `#` hides from `search` and `hash`; the server refuses
    // both, so the serialisation is read.
    !url.href.includes("?") &&
    !url.href.includes("#")
  if (
    !plain ||
    !rules.hosts.includes(url.hostname) ||
    !url.pathname.startsWith(rules.pathPrefix)
  ) {
    return null
  }
  const key = url.pathname.slice(rules.pathPrefix.length)
  return isPadletKey(key, rules) ? key : null
}
