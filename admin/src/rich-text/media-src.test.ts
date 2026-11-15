import fc from "fast-check"
import { describe, expect, it } from "vitest"

import { decodedSegmentsArePlain, embedSrc, imageSrc } from "./media-src"
import { CONTENT_V1_POLICY } from "./policy"
import { policyWith } from "@/test/rich-text-policy"

const URLS = CONTENT_V1_POLICY.urls

/**
 * The cases mirror the server's `url_rules.rs` tests, so the two rules are
 * checked against the same inputs.
 */
describe("imageSrc", () => {
  it.each([
    [
      "/storage/uploads/posts/612/kep.jpg",
      "/storage/uploads/posts/612/kep.jpg",
    ],
    ["/storage/uploads/a.JPEG", "/storage/uploads/a.JPEG"],
    ["/storage/uploads/kép (1).png", "/storage/uploads/k%C3%A9p%20(1).png"],
    [
      "https://verseghy-gimnazium.net/storage/uploads/b.png",
      "/storage/uploads/b.png",
    ],
    [
      "http://backend3.microshift.verseghy-gimnazium.net/storage/uploads/c.gif",
      "/storage/uploads/c.gif",
    ],
    [
      "https://verseghy-gimnazium.net./storage/uploads/d.webp",
      "/storage/uploads/d.webp",
    ],
    ["/storage/uploads/x/../y.jpg", "/storage/uploads/y.jpg"],
  ])("keeps %s as %s", (value, stored) => {
    expect(imageSrc(value, URLS)).toBe(stored)
  })

  it.each([
    "",
    "a.jpg",
    "../storage/uploads/a.jpg",
    "//evil.example/storage/uploads/a.jpg",
    "/\\evil.example/storage/uploads/a.jpg",
    "https://evil.example/storage/uploads/a.jpg",
    "https://verseghy-gimnazium.net:8443/storage/uploads/a.jpg",
    "https://u@verseghy-gimnazium.net/storage/uploads/a.jpg",
    "ftp://verseghy-gimnazium.net/storage/uploads/a.jpg",
    "data:image/png;base64,AA",
    "blob:https://verseghy-gimnazium.net/1",
    "/storage/other/a.jpg",
    "/storage/uploads/a.svg",
    "/storage/uploads/a.jpg?x=1",
    "/storage/uploads/a.jpg?",
    "/storage/uploads/a.jpg#",
    "/storage/uploads/a%2F..%2Fb.jpg",
    "/storage/uploads/a%5Cb.jpg",
    "/storage/uploads/a.svg%00.jpg",
    "/storage/uploads/%FF.jpg",
    "/%73torage/uploads/a.jpg",
    "/storage/uploads/a%2Ejpg",
    "https://verseghy-gimnazium.net//storage/uploads/a.jpg",
  ])("refuses %j", (value) => {
    expect(imageSrc(value, URLS)).toBeNull()
  })

  it("follows the policy's prefix and own hosts", () => {
    const policy = policyWith((json) => {
      const urls = json.urls as { [key: string]: { [key: string]: unknown } }
      urls.image.path_prefix = "/kepek/"
      ;(json.urls as { own_hosts: string[] }).own_hosts = ["iskola.example"]
    })
    expect(imageSrc("/kepek/a.jpg", policy.urls)).toBe("/kepek/a.jpg")
    expect(imageSrc("https://iskola.example/kepek/a.jpg", policy.urls)).toBe(
      "/kepek/a.jpg"
    )
    expect(imageSrc("/storage/uploads/a.jpg", policy.urls)).toBeNull()
  })
})

describe("decodedSegmentsArePlain", () => {
  it.each([
    ["/storage/uploads/a.jpg", true],
    ["/storage/uploads/k%C3%A9p.jpg", true],
    ["/storage/uploads/a%2Fb.jpg", false],
    ["/storage/uploads/a%5Cb.jpg", false],
    ["/storage/uploads/%2E%2E/a.jpg", false],
    ["/storage/uploads/a%00.jpg", false],
    ["/storage/uploads/%C3.jpg", false],
  ])("judges %s as %s", (path, plain) => {
    expect(decodedSegmentsArePlain(path)).toBe(plain)
  })
})

describe("embedSrc", () => {
  it.each([
    "https://www.youtube-nocookie.com/embed/dQw4w9WgXcQ",
    "https://www.youtube-nocookie.com/embed/dQw4w9WgXcQ?start=30",
    "https://www.youtube-nocookie.com/embed/dQw4w9WgXcQ?list=PLa_b-1",
    "https://www.youtube-nocookie.com/embed/dQw4w9WgXcQ?start=0&list=PL1",
    "https://www.youtube.com/embed/a-b_C9d-E_f",
    "https://www.youtube-nocookie.com/embed/videoseries?list=PLMOZDTW7ZvXM",
    "https://www.youtube.com/embed/videoseries?start=5&list=PL1",
    "https://padlet.com/embed/KEY123",
    "https://padlet.com/embed/k",
  ])("keeps %s", (value) => {
    expect(embedSrc(value, URLS)).toBe(value)
  })

  it("writes the host as the URL parser does", () => {
    expect(
      embedSrc("https://WWW.YouTube-NoCookie.com/embed/AbCdEfGhIjK", URLS)
    ).toBe("https://www.youtube-nocookie.com/embed/AbCdEfGhIjK")
    expect(embedSrc("https://padlet.com:443/embed/k", URLS)).toBe(
      "https://padlet.com/embed/k"
    )
  })

  it.each([
    "http://www.youtube-nocookie.com/embed/AbCdEfGhIjK",
    "https://youtube.com/embed/AbCdEfGhIjK",
    "https://m.youtube.com/embed/AbCdEfGhIjK",
    "https://www.youtube.com.evil.example/embed/AbCdEfGhIjK",
    "https://www.youtube.com./embed/AbCdEfGhIjK",
    "https://evil@www.youtube.com/embed/AbCdEfGhIjK",
    "https://www.youtube.com:8443/embed/AbCdEfGhIjK",
    "https://www.youtube.com/embed/",
    "https://www.youtube.com/embed/AbCdEfGhIjK/b",
    "https://www.youtube.com/embed/AbCdEfGh%20jK",
    "https://www.youtube.com/watch?v=AbCdEfGhIjK",
    "https://www.youtube.com/embed/AbCdEfGhIjK?si=abc",
    "https://www.youtube.com/embed/AbCdEfGhIjK?list=a&start=1",
    "https://www.youtube.com/embed/AbCdEfGhIjK?start=",
    "https://www.youtube.com/embed/AbCdEfGhIjK?start=-1",
    "https://www.youtube.com/embed/AbCdEfGhIjK?start=1&list=a&rel=0",
    "https://www.youtube.com/embed/AbCdEfGhIjK?start=1&start=2",
    "https://www.youtube.com/embed/AbCdEfGhIjK?",
    "https://www.youtube.com/embed/AbCdEfGhIjK#t=1",
    "https://www.youtube.com/embed/AbCdEfGhIjK#",
    "https://www.youtube.com/embed/x",
    "https://www.youtube.com/embed/AbCdEfGhIjKl",
    "https://www.youtube.com/embed/videoseries",
    "https://www.youtube.com/embed/videoseries?start=5",
    "https://www.youtube.com/embed?listType=playlist&list=PL1",
    "https://www.youtube.com/embed/videoseries?listType=playlist&list=PL1",
    "https://padlet.com/embed/k?x=1",
    "https://padlet.com/embed/videoseries?list=PL1",
    "https://evil.example/embed/AbCdEfGhIjK",
    "/embed/AbCdEfGhIjK",
    "javascript:alert(1)",
    "",
  ])("refuses %j", (value) => {
    expect(embedSrc(value, URLS)).toBeNull()
  })

  it("reads the provider's id rules from the policy", () => {
    const policy = policyWith((json) => {
      const urls = json.urls as {
        embed: { providers: { [key: string]: unknown }[] }
      }
      urls.embed.providers[0].id_length = null
      urls.embed.providers[1].id_length = 3
    })
    expect(
      embedSrc("https://www.youtube.com/embed/x", policy.urls)
    ).not.toBeNull()
    expect(embedSrc("https://padlet.com/embed/abc", policy.urls)).not.toBeNull()
    expect(embedSrc("https://padlet.com/embed/abcd", policy.urls)).toBeNull()
  })

  it("keeps exactly the YouTube addresses the server's rule keeps", () => {
    fc.assert(
      fc.property(
        fc.stringMatching(/^[A-Za-z0-9_-]{1,20}$/),
        fc.option(fc.integer({ min: 0, max: 100_000 }), { nil: null }),
        fc.option(fc.stringMatching(/^[A-Za-z0-9_-]{1,12}$/), { nil: null }),
        (id, start, list) => {
          const query = [
            start === null ? null : `start=${start}`,
            list === null ? null : `list=${list}`,
          ].filter((part) => part !== null)
          const url = `https://www.youtube-nocookie.com/embed/${id}${query.length > 0 ? `?${query.join("&")}` : ""}`
          const kept = id === "videoseries" ? list !== null : id.length === 11
          expect(embedSrc(url, URLS)).toBe(kept ? url : null)
          const padlet = `https://padlet.com/embed/${id}${query.length > 0 ? `?${query.join("&")}` : ""}`
          expect(embedSrc(padlet, URLS)).toBe(
            query.length === 0 ? padlet : null
          )
        }
      )
    )
  })

  it("returns only https addresses on a provider's host, for any input", () => {
    fc.assert(
      fc.property(fc.oneof(fc.string(), fc.webUrl()), (value) => {
        const kept = embedSrc(value, URLS)
        if (kept !== null) {
          const url = new URL(kept)
          expect(url.protocol).toBe("https:")
          expect(
            URLS.embed.providers.some((p) => p.hosts.includes(url.hostname))
          ).toBe(true)
          expect(embedSrc(kept, URLS)).toBe(kept)
        }
      })
    )
  })

  it("keeps only root-relative image paths under the prefix, for any input", () => {
    fc.assert(
      fc.property(
        fc.oneof(
          fc.string(),
          fc.webUrl(),
          fc.string().map((tail) => `/storage/uploads/${tail}.jpg`)
        ),
        (value) => {
          const kept = imageSrc(value, URLS)
          if (kept !== null) {
            expect(kept.startsWith(URLS.image.path_prefix)).toBe(true)
            expect(imageSrc(kept, URLS)).toBe(kept)
          }
        }
      )
    )
  })
})
