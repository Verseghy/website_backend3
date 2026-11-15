/**
 * The `img[src]` and `iframe[src]` rules of content-v1 on the client
 * (editor.md §3.6), for the image and embed nodes.
 *
 * The editor keeps an image or an embed only when the server would keep its
 * address, so a body the editor writes is a fixed point of the sanitiser
 * (editor.md §6, P2), and an address the server refuses never reaches it as
 * an `image_src_rejected` or `embed_rejected` write. The rules are the
 * server's `image_src` and `embed_src` (`src/domain/rich_text/url_rules.rs`)
 * written again on the WHATWG URL parser that the browsers and jsdom
 * implement, the same standard as the Rust `url` crate, and both read their
 * parameters from `policy.json` (editor.md §3.9), so the two sides cannot
 * drift apart:
 *
 * - an image is a root-relative path under the image prefix, or an `http` or
 *   `https` URL on one of backend3's own hosts (stored as its path), ending
 *   in an image extension, with no query and no fragment, whose segments
 *   decode to plain names (no `.`, `..`, separator, NUL or invalid UTF-8);
 * - an embed is an `https` URL on a provider's host, with no user info,
 *   port or fragment, whose path is the provider's prefix and an id, and
 *   whose query holds only the provider's parameters, in order. A provider
 *   may fix the id's length (11 for a YouTube video) and name one id that
 *   is a path only together with `list` (`videoseries`, ADR 0043 §3).
 *
 * The URL API hides an empty query or fragment (`search` and `hash` are
 * `""` for both "none" and a bare `?` or `#`), while the server refuses a
 * bare `?`. The serialisation tells them apart: a parsed path never holds a
 * raw `?` or `#`, so their presence in `href` is the presence of a query or
 * fragment.
 */
import {
  endsWithExtension,
  isOwnHost,
  leadingSlashes,
  SAME_ORIGIN_BASE,
} from "./link-href"
import type { EmbedProvider, UrlPolicy } from "./policy"

/** The origin of {@link SAME_ORIGIN_BASE}. */
const SAME_ORIGIN = new URL(SAME_ORIGIN_BASE).origin

/** Parses an absolute URL, or returns `null`. */
function parseAbsolute(value: string): URL | null {
  try {
    return new URL(value)
  } catch {
    return null
  }
}

/** Whether the serialised URL carries a query, even an empty one. */
function hasQuery(url: URL): boolean {
  const beforeFragment = url.href.split("#", 1)[0]
  return beforeFragment.includes("?")
}

/** Whether the serialised URL carries a fragment, even an empty one. */
function hasFragment(url: URL): boolean {
  return url.href.includes("#")
}

/**
 * Percent-decodes one path segment as strict UTF-8, or returns `null` when
 * the bytes are not UTF-8 (the server's `decode_utf8`).
 */
function decodeStrict(segment: string): string | null {
  const bytes: number[] = []
  const encoder = new TextEncoder()
  for (let i = 0; i < segment.length; i += 1) {
    const hex = segment.slice(i + 1, i + 3)
    if (segment[i] === "%" && /^[0-9A-Fa-f]{2}$/.test(hex)) {
      bytes.push(parseInt(hex, 16))
      i += 2
    } else {
      bytes.push(...encoder.encode(segment[i]))
    }
  }
  try {
    return new TextDecoder("utf-8", { fatal: true }).decode(
      new Uint8Array(bytes)
    )
  } catch {
    return null
  }
}

/**
 * Whether every `/`-separated segment of a serialised path decodes to a
 * plain name: valid UTF-8, neither `.` nor `..`, and no `/`, `\` or NUL
 * (research: DECISIONS R11; the server's `decoded_segments_are_plain`).
 *
 * @param path - a path as the URL parser serialises it.
 * @returns `true` when the path names the file it spells.
 */
export function decodedSegmentsArePlain(path: string): boolean {
  return path.split("/").every((segment) => {
    const decoded = decodeStrict(segment)
    return (
      decoded !== null &&
      decoded !== "." &&
      decoded !== ".." &&
      !decoded.includes("/") &&
      !decoded.includes("\\") &&
      !decoded.includes("\u0000")
    )
  })
}

/**
 * The `img[src]` rule: the root-relative path the server stores for an
 * image address, or `null` when it refuses the address.
 *
 * @param value - the attribute value, after HTML character references are
 *   decoded (as `getAttribute` returns it).
 * @param urls - the policy's URL rules (`policy.urls`): the own hosts, the
 *   image prefix and the image extensions.
 * @returns the path the server writes (the absolute form on an own host
 *   becomes its path), or `null`.
 */
export function imageSrc(value: string, urls: UrlPolicy): string | null {
  let url = parseAbsolute(value)
  if (url !== null) {
    const scheme = url.protocol.slice(0, -1)
    const plain = url.port === "" && url.username === "" && url.password === ""
    if (
      !(scheme === "http" || scheme === "https") ||
      !isOwnHost(url, urls) ||
      !plain
    ) {
      return null
    }
  } else {
    // Path-relative values resolve against the page's own path, which the
    // server cannot know, so only a path-absolute one is read.
    if (leadingSlashes(value) !== 1) {
      return null
    }
    try {
      url = new URL(value, SAME_ORIGIN_BASE)
    } catch {
      return null
    }
    if (url.origin !== SAME_ORIGIN) {
      return null
    }
  }
  if (hasQuery(url) || hasFragment(url)) {
    return null
  }
  const path = url.pathname
  return path.startsWith(urls.image.path_prefix) &&
    !path.startsWith("//") &&
    endsWithExtension(path, urls.image.extensions) &&
    decodedSegmentsArePlain(path)
    ? path
    : null
}

/** Whether `text` is one or more characters of the embed id alphabet. */
function isEmbedId(text: string, alphabet: string): boolean {
  return text.length > 0 && [...text].every((c) => alphabet.includes(c))
}

/**
 * Reads an embed query against a provider's parameters: whether it carries
 * `list`, or `null` when it is not a query the provider allows.
 */
function readQuery(
  query: string,
  provider: EmbedProvider,
  alphabet: string
): { readonly hasList: boolean } | null {
  let next = 0
  let hasList = false
  for (const parameter of query.split("&")) {
    const equals = parameter.indexOf("=")
    if (equals < 0) {
      return null
    }
    const name = parameter.slice(0, equals)
    const value = parameter.slice(equals + 1)
    const index = provider.query.findIndex(
      (rule, position) => position >= next && rule.name === name
    )
    if (index < 0) {
      return null
    }
    const rule = provider.query[index]
    const valid =
      rule.value === "digits"
        ? /^[0-9]+$/.test(value)
        : isEmbedId(value, alphabet)
    if (!valid) {
      return null
    }
    hasList ||= rule.value === "id"
    next = index + 1
  }
  return { hasList }
}

/**
 * The `iframe[src]` rule: the address the server stores for an embed, or
 * `null` when it refuses the address.
 *
 * @param value - the attribute value, after HTML character references are
 *   decoded.
 * @param urls - the policy's URL rules (`policy.urls`): the embed scheme, id
 *   alphabet and providers.
 * @returns the URL as the parser serialises it (an upper-case host comes
 *   back lower-case), or `null`.
 */
export function embedSrc(value: string, urls: UrlPolicy): string | null {
  const url = parseAbsolute(value)
  if (
    url === null ||
    url.protocol !== `${urls.embed.scheme}:` ||
    url.username !== "" ||
    url.password !== "" ||
    url.port !== "" ||
    hasFragment(url)
  ) {
    return null
  }
  const provider = urls.embed.providers.find((candidate) =>
    candidate.hosts.includes(url.hostname)
  )
  if (
    provider === undefined ||
    !url.pathname.startsWith(provider.path_prefix)
  ) {
    return null
  }
  const id = url.pathname.slice(provider.path_prefix.length)
  let hasList = false
  if (hasQuery(url)) {
    const query = url.href.split("#", 1)[0].split("?").slice(1).join("?")
    const read =
      provider.query.length === 0
        ? null
        : readQuery(query, provider, urls.embed.id_alphabet)
    if (read === null) {
      return null
    }
    hasList = read.hasList
  }
  const idOk =
    provider.list_only_id !== null && id === provider.list_only_id
      ? hasList
      : isEmbedId(id, urls.embed.id_alphabet) &&
        (provider.id_length === null || id.length === provider.id_length)
  return idOk ? url.href : null
}
