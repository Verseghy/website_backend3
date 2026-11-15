/**
 * The `a[href]` rule of content-v1 on the client (editor.md §3.6), for the
 * Link mark's `isAllowedUri` (editor.md §4.3 row 1b).
 *
 * The editor keeps a link only when the server would keep it, so a body the
 * editor writes is a fixed point of the sanitiser (editor.md §6, P2). The
 * rule is the server's `link_href`, `scheme_admitted` and
 * `same_origin_relative` (`src/domain/rich_text/url_rules.rs`) written again
 * on the WHATWG URL parser that the browsers and jsdom implement, the same
 * standard as the Rust `url` crate:
 *
 * - an absolute URL must use one of the policy's schemes (http, https,
 *   mailto, tel);
 * - a relative URL must stay on the page's own origin, so the
 *   scheme-relative forms `//host`, `/\host` and their tab- or
 *   newline-obfuscated spellings are refused by their shape;
 * - a link that resolves to a document under `/storage/` on one of
 *   backend3's own hosts (`.svg`, `.html`, `.xml`, ...) is refused, compared
 *   the way a static server maps the path to a file: percent-decoded, dot
 *   segments resolved, repeated slashes merged, lower case.
 *
 * TipTap's stock check accepts more (ftp, sms, cid, scheme-relative URLs),
 * which the server would then remove on save and report through the drift
 * canary (ADR 0013 §5).
 */
import type { UrlPolicy } from "./policy"

/**
 * The base relative URLs are resolved against. `.invalid` is reserved
 * (RFC 2606), so it stands for "the page's own origin" and can never be a
 * real host; the server uses the same base.
 */
const SAME_ORIGIN_BASE = "https://same-origin.invalid/"

/** The origin of {@link SAME_ORIGIN_BASE}. */
const SAME_ORIGIN = new URL(SAME_ORIGIN_BASE).origin

/** The parsed form of a candidate `href`. */
type Parsed =
  | { readonly kind: "absolute"; readonly url: URL }
  | { readonly kind: "relative"; readonly url: URL }
  | { readonly kind: "invalid" }

/**
 * Parses an `href` the way the server tells its cases apart: absolute (it has
 * a scheme), relative (it has none and resolves against a base), or invalid.
 */
function parse(href: string): Parsed {
  try {
    return { kind: "absolute", url: new URL(href) }
  } catch {
    // Not an absolute URL: either relative or unparsable.
  }
  try {
    return { kind: "relative", url: new URL(href, SAME_ORIGIN_BASE) }
  } catch {
    return { kind: "invalid" }
  }
}

/**
 * How many of the first two characters of a relative URL are `/` or `\`
 * (which WHATWG parsing treats as `/`), after the parser's own clean-up:
 * leading and trailing C0 controls and spaces trimmed, tabs and newlines
 * removed. One means path-absolute; two means scheme-relative.
 */
function leadingSlashes(href: string): number {
  let start = 0
  let end = href.length
  while (start < end && href.charCodeAt(start) <= 0x20) {
    start += 1
  }
  while (end > start && href.charCodeAt(end - 1) <= 0x20) {
    end -= 1
  }
  let count = 0
  for (const character of href.slice(start, end)) {
    if (character === "\t" || character === "\n" || character === "\r") {
      continue
    }
    if (count === 2 || (character !== "/" && character !== "\\")) {
      break
    }
    count += 1
  }
  return count
}

/**
 * Percent-decodes a URL path to text, replacing invalid UTF-8 with U+FFFD,
 * as the server's `percent_decode_str(...).decode_utf8_lossy()` does.
 * `decodeURIComponent` throws on such input instead.
 */
function decodeLossy(path: string): string {
  const encoder = new TextEncoder()
  const bytes: number[] = []
  for (let i = 0; i < path.length; i += 1) {
    const hex = path.slice(i + 1, i + 3)
    if (path[i] === "%" && /^[0-9A-Fa-f]{2}$/.test(hex)) {
      bytes.push(parseInt(hex, 16))
      i += 2
    } else {
      bytes.push(...encoder.encode(path[i]))
    }
  }
  return new TextDecoder("utf-8").decode(new Uint8Array(bytes))
}

/**
 * The file path a static server serves for a decoded, `/`-led path: `.` and
 * `..` segments resolved and empty segments merged. A trailing slash is kept,
 * because it names a directory rather than a file.
 */
function servedPath(path: string): string {
  const segments: string[] = []
  const parts = path.split("/").slice(1)
  parts.forEach((segment, index) => {
    const last = index === parts.length - 1
    if (segment === "..") {
      segments.pop()
    } else if (segment !== "." && (segment !== "" || last)) {
      segments.push(segment)
    }
    if (last && (segment === "." || segment === "..")) {
      segments.push("")
    }
  })
  return `/${segments.join("/")}`
}

/** Whether the last segment of `path` ends in `.` plus one of `extensions`. */
function endsWithExtension(
  path: string,
  extensions: readonly string[]
): boolean {
  const name = path.slice(path.lastIndexOf("/") + 1).toLowerCase()
  return extensions.some((extension) => name.endsWith(`.${extension}`))
}

/** Whether a URL's host is one of the own hosts; a trailing dot is ignored. */
function isOwnHost(url: URL, urls: UrlPolicy): boolean {
  const host = url.hostname.endsWith(".")
    ? url.hostname.slice(0, -1)
    : url.hostname
  return urls.own_hosts.includes(host)
}

/**
 * Whether the server keeps `href` on a link (editor.md §3.6, `a[href]`).
 *
 * @param href - the attribute value, after HTML character references are
 *   decoded (as `getAttribute` returns it).
 * @param urls - the policy's URL rules (`policy.urls`).
 * @returns `true` when the link is kept unchanged, `false` when the server
 *   would drop the `href` (and the editor keeps the text without a link).
 */
export function isAllowedHref(href: string, urls: UrlPolicy): boolean {
  const parsed = parse(href)
  let target: URL | null = null
  switch (parsed.kind) {
    case "invalid":
      return false
    case "absolute": {
      const scheme = parsed.url.protocol.slice(0, -1)
      if (!urls.schemes.includes(scheme)) {
        return false
      }
      if (
        (scheme === "http" || scheme === "https") &&
        isOwnHost(parsed.url, urls)
      ) {
        target = parsed.url
      }
      break
    }
    case "relative":
      if (leadingSlashes(href) > 1 || parsed.url.origin !== SAME_ORIGIN) {
        return false
      }
      target = parsed.url
      break
  }
  if (target === null) {
    return true
  }
  const served = servedPath(decodeLossy(target.pathname).toLowerCase())
  return !(
    served.startsWith(urls.storage_path_prefix) &&
    endsWithExtension(served, urls.link.forbidden_storage_extensions)
  )
}
