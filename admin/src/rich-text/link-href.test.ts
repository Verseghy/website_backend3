// @vitest-environment node
import fc from "fast-check"
import { describe, expect, it } from "vitest"

import { isAllowedHref } from "./link-href"
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
