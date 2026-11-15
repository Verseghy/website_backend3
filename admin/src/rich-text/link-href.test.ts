// @vitest-environment node
import { readFileSync } from "node:fs"

import fc from "fast-check"
import { describe, expect, it } from "vitest"

import {
  endsWithExtension,
  isAllowedHref,
  isOwnHost,
  leadingSlashes,
  SAME_ORIGIN_BASE,
} from "./link-href"
import { CONTENT_V1_POLICY } from "./policy"
import { policyWith } from "@/test/rich-text-policy"

const URLS = CONTENT_V1_POLICY.urls
const allowed = (href: string) => isAllowedHref(href, URLS)

/**
 * The cases mirror the server's tests of the same rule
 * (`src/domain/rich_text/url_rules.rs`: scheme_admitted, is_relative,
 * same_origin_relative and link_href), so the two sides agree on them.
 */
describe("isAllowedHref", () => {
  it.each([
    ["an https URL", "https://x.example/"],
    ["an http URL without a path", "http://x"],
    ["mailto", "mailto:a@b.example"],
    ["tel", "tel:+3612345"],
    ["an upper-case scheme", "HTTPS://x.example/"],
    ["a root-relative path", "/x"],
    ["a path-relative link", "x/y"],
    ["a parent-relative link", "../x"],
    ["a fragment", "#top"],
    ["a query", "?q=1"],
    ["an empty href", ""],
    ["an encoded slash after the first", "/%2F/evil"],
    ["a colon that is no scheme", ":x"],
    ["a percent sign before a colon", "%41:x"],
    ["a single backslash", "\\x"],
    ["a document outside /storage/", "/storage/uploads/doc.pdf"],
    ["an image under /storage/", "/storage/uploads/kep.jpg"],
    ["a directory that only looks like a document", "/storage/x.svg/"],
    ["a dot segment that climbs out of /storage/", "/storage/..%2Fx.svg"],
    ["a document on another host", "https://example.org/storage/x.svg"],
    ["another site", "https://example.org/"],
    ["an e-mail address", "mailto:titkarsag@example.org"],
    ["a page path", "/hirek/12"],
  ])("keeps %s", (_name, href) => {
    expect(allowed(href)).toBe(true)
  })

  it.each([
    ["javascript", "javascript:alert(1)"],
    ["javascript split by a tab", "java\tscript:x"],
    ["javascript after a space", " javascript:x"],
    ["data", "data:image/png;base64,AA"],
    ["vbscript", "vbscript:x"],
    ["blob", "blob:https://x/1"],
    ["file", "file:///etc/passwd"],
    ["cid, which TipTap's stock rule allows", "cid:abc"],
    ["ftp, which TipTap's stock rule allows", "ftp://x.example/"],
    ["sms, which TipTap's stock rule allows", "sms:+3612345"],
    ["an unparsable host", "http://[::1"],
    ["a scheme without a host", "https://"],
    ["a scheme-relative URL", "//evil.example/x"],
    ["a slash and a backslash", "/\\evil.example/x"],
    ["two backslashes", "\\\\evil.example"],
    ["a scheme-relative URL after a tab", "\t//evil.example"],
    ["slashes split by a tab", "/\t/evil.example"],
    ["a scheme-relative URL after a space", " //evil.example"],
    ["a backslash and a slash", "\\/evil.example"],
    ["the same-origin base itself", "//same-origin.invalid/x"],
    ["the base behind backslashes", "\\\\same-origin.invalid/x"],
    ["an SVG under /storage/", "/storage/x.svg"],
    ["HTML in an upload folder, in any case", "/storage/uploads/a/b.HTML"],
    ["XML through a relative path", "storage/x.xml"],
    ["XSL through a parent path", "../storage/x.xsl"],
    ["an SVG with a query", "/storage/x.svg?download=1"],
    ["an encoded dot", "/storage/x%2Esvg"],
    ["an encoded letter of the prefix", "/%73torage/x.svg"],
    ["an upper-case prefix and extension", "/STORAGE/x.SVGZ"],
    ["an encoded climb into /storage/", "/x/..%2Fstorage/x.svg"],
    ["an encoded climb with encoded slashes", "/a%2F..%2Fstorage%2Fx.svg"],
    ["an encoded leading slash", "/%2Fstorage/x.svg"],
    ["an encoded dot segment", "/storage/uploads/%2e%2e/x.html"],
    [
      "an own host",
      "https://backend3.microshift.verseghy-gimnazium.net/storage/x.xhtml",
    ],
    [
      "an own host with a trailing dot",
      "http://verseghy-gimnazium.net./storage/x.xht",
    ],
    [
      "an own host with a port",
      "https://verseghy-gimnazium.net:8443/storage/x.htm",
    ],
  ])("refuses %s", (_name, href) => {
    expect(allowed(href)).toBe(false)
  })

  it("follows the policy's schemes and hosts", () => {
    const urls = policyWith((json) => {
      const policyUrls = json.urls as { schemes: string[]; own_hosts: string[] }
      policyUrls.schemes = ["https"]
      policyUrls.own_hosts = ["example.org"]
    }).urls
    expect(isAllowedHref("mailto:a@b.example", urls)).toBe(false)
    expect(isAllowedHref("https://example.org/storage/x.svg", urls)).toBe(false)
    expect(
      isAllowedHref("https://verseghy-gimnazium.net/storage/x.svg", urls)
    ).toBe(true)
  })

  it("never throws, whatever the input", () => {
    fc.assert(
      fc.property(fc.string({ unit: "binary" }), (href) => {
        expect(typeof allowed(href)).toBe("boolean")
      })
    )
  })

  it("keeps root-relative paths and refuses their scheme-relative forms", () => {
    fc.assert(
      fc.property(
        fc.stringMatching(/^[a-z0-9._~-]{0,20}$/),
        fc.stringMatching(/^[a-z]{1,10}\.example$/),
        (path, host) => {
          expect(allowed(`/${path}`)).toBe(true)
          for (const href of [
            `//${host}/${path}`,
            `/\\${host}/${path}`,
            `\\\\${host}`,
            `//same-origin.invalid/${path}`,
          ]) {
            expect(allowed(href)).toBe(false)
          }
        }
      )
    )
  })

  it("refuses every document under /storage/, however it is spelt", () => {
    fc.assert(
      fc.property(
        fc.stringMatching(/^[a-z0-9]{1,10}$/),
        fc.constantFrom(...URLS.link.forbidden_storage_extensions),
        fc.constantFrom(...URLS.own_hosts),
        (name, extension, host) => {
          for (const href of [
            `/storage/${name}.${extension}`,
            `/${name}/..%2Fstorage/${name}.${extension}`,
            `/%2Fstorage%2F${name}.${extension}`,
            `https://${host}/storage/${name}.${extension.toUpperCase()}`,
          ]) {
            expect(allowed(href), href).toBe(false)
          }
        }
      )
    )
  })

  it("refuses every scheme outside the policy", () => {
    fc.assert(
      fc.property(
        fc
          .stringMatching(/^[a-z][a-z0-9+.-]{0,10}$/)
          .filter((scheme) => !URLS.schemes.includes(scheme)),
        (scheme) => {
          expect(allowed(`${scheme}:x`)).toBe(false)
          expect(allowed(`${scheme}://x.example/`)).toBe(false)
        }
      )
    )
  })
})

/**
 * The helpers that the image rule (`media-src.ts`) shares with the link
 * rule, tested on their own so that a change to one rule cannot change the
 * other unseen.
 */
describe("SAME_ORIGIN_BASE", () => {
  it("is the server's base, on a reserved host that no site can own", () => {
    const rust = readFileSync(
      new URL("../../../src/domain/rich_text/url_rules.rs", import.meta.url),
      "utf8"
    )
    const server = /pub const SAME_ORIGIN_BASE: &str = "([^"]+)";/.exec(rust)
    expect(server?.[1]).toBe(SAME_ORIGIN_BASE)
    const base = new URL(SAME_ORIGIN_BASE)
    expect(base.hostname.endsWith(".invalid")).toBe(true)
    expect(URLS.own_hosts).not.toContain(base.hostname)
  })
})

describe("leadingSlashes", () => {
  it.each([
    ["an empty value", "", 0],
    ["blanks only", " \t\n ", 0],
    ["a path-relative value", "x/y", 0],
    ["a query", "?/x", 0],
    ["a path", "/x", 1],
    ["a backslash", "\\x", 1],
    ["a path after a space and a C0 control", " \u0001/x", 1],
    ["a path followed by blanks", "/ ", 1],
    ["two slashes", "//x", 2],
    ["three slashes, counted up to two", "///x", 2],
    ["a slash and a backslash", "/\\x", 2],
    ["two backslashes", "\\\\x", 2],
    ["slashes split by a tab", "/\t/x", 2],
    ["slashes split by a newline and a return", "/\n\r/x", 2],
    ["slashes after a tab", "\t//x", 2],
  ])("counts %s", (_name, href, count) => {
    expect(leadingSlashes(href)).toBe(count)
  })

  it("counts the slashes in front, whatever blanks surround them", () => {
    fc.assert(
      fc.property(
        fc.constantFrom("", " ", "\u0000", "\u001f ", "\t"),
        fc.array(
          fc.tuple(
            fc.constantFrom("/", "\\"),
            fc.constantFrom("", "\t", "\n", "\r", "\t\n")
          ),
          { maxLength: 4 }
        ),
        fc.stringMatching(/^[a-z0-9.?#:-]{0,10}$/),
        (lead, slashes, rest) => {
          const href =
            lead + slashes.map(([s, noise]) => s + noise).join("") + rest
          expect(leadingSlashes(href)).toBe(Math.min(slashes.length, 2))
        }
      )
    )
  })

  it("is two or more exactly when the URL parser leaves the page's origin", () => {
    fc.assert(
      fc.property(
        fc.integer({ min: 0, max: 3 }),
        fc.constantFrom("", "\t", "\n"),
        fc.stringMatching(/^[a-z]{1,10}\.example$/),
        (count, noise, host) => {
          const href = ` ${"/".repeat(count).split("").join(noise)}${host}/x`
          const resolved = new URL(href, SAME_ORIGIN_BASE)
          expect(leadingSlashes(href) >= 2).toBe(
            resolved.origin !== new URL(SAME_ORIGIN_BASE).origin
          )
        }
      )
    )
  })
})

describe("endsWithExtension", () => {
  const SVG = ["svg"]
  it.each([
    ["a file with the extension", "/storage/x.svg", SVG, true],
    ["the extension in upper case", "/storage/x.SVG", SVG, true],
    ["one of several extensions", "/x.tar.gz", ["zip", "gz"], true],
    ["a name that is only the extension", "/.svg", SVG, true],
    ["a path without a slash", "x.svg", SVG, true],
    ["a directory that looks like a file", "/storage/x.svg/", SVG, false],
    ["the extension in a parent segment", "/a.svg/b", SVG, false],
    ["the extension without its dot", "/storage/xsvg", SVG, false],
    ["another extension", "/storage/x.svgz", SVG, false],
    ["an empty path", "", SVG, false],
    ["no extensions", "/x.svg", [], false],
  ])("answers for %s", (_name, path, extensions, expected) => {
    expect(endsWithExtension(path, extensions)).toBe(expected)
  })

  it("looks at the last segment only, in any case", () => {
    const extensions = URLS.image.extensions
    fc.assert(
      fc.property(
        fc.stringMatching(/^[a-z]{0,5}$/),
        fc.stringMatching(/^[a-z0-9]{1,10}$/),
        fc.constantFrom(...extensions),
        fc.boolean(),
        (dir, name, extension, upper) => {
          const written = upper ? extension.toUpperCase() : extension
          expect(
            endsWithExtension(`/${dir}/${name}.${written}`, extensions)
          ).toBe(true)
          expect(endsWithExtension(`/${name}.${written}/`, extensions)).toBe(
            false
          )
          expect(
            endsWithExtension(`/${name}.${written}/${name}`, extensions)
          ).toBe(false)
        }
      )
    )
  })
})

describe("isOwnHost", () => {
  it.each([
    ["an own host", "https://verseghy-gimnazium.net/x", true],
    [
      "an own host with a trailing dot",
      "https://verseghy-gimnazium.net./x",
      true,
    ],
    ["an own host in upper case", "HTTPS://VERSEGHY-GIMNAZIUM.NET/", true],
    ["an own host with a port", "http://verseghy-gimnazium.net:8443/", true],
    ["a sub-domain of an own host", "https://x.verseghy-gimnazium.net/", false],
    [
      "an own host as a prefix",
      "https://verseghy-gimnazium.net.example/",
      false,
    ],
    [
      "an own host as user info",
      "https://verseghy-gimnazium.net@x.example/",
      false,
    ],
    ["another host", "https://example.org/", false],
  ])("answers for %s", (_name, url, expected) => {
    expect(isOwnHost(new URL(url), URLS)).toBe(expected)
  })

  it("follows the policy's own hosts", () => {
    const urls = policyWith((json) => {
      ;(json.urls as { own_hosts: string[] }).own_hosts = ["example.org"]
    }).urls
    expect(isOwnHost(new URL("https://example.org/"), urls)).toBe(true)
    expect(isOwnHost(new URL("https://verseghy-gimnazium.net/"), urls)).toBe(
      false
    )
  })

  it("knows every own host, however it is written, and no other", () => {
    fc.assert(
      fc.property(
        fc.constantFrom(...URLS.own_hosts),
        fc.boolean(),
        fc.boolean(),
        fc.stringMatching(/^[a-z]{1,10}$/),
        (host, upper, trailingDot, label) => {
          const written =
            (upper ? host.toUpperCase() : host) + (trailingDot ? "." : "")
          expect(isOwnHost(new URL(`https://${written}/${label}`), URLS)).toBe(
            true
          )
          expect(isOwnHost(new URL(`https://${label}.example/`), URLS)).toBe(
            false
          )
          expect(isOwnHost(new URL(`https://${host}.${label}/`), URLS)).toBe(
            false
          )
          // `backend.` in front of the school's domain is itself an own host.
          const subdomain = `${label}.${host}`
          expect(isOwnHost(new URL(`https://${subdomain}/`), URLS)).toBe(
            URLS.own_hosts.includes(subdomain)
          )
        }
      )
    )
  })
})
