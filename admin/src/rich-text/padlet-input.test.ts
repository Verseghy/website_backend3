import fc from "fast-check"
import { describe, expect, it } from "vitest"

import { embedSrc } from "./media-src"
import {
  isPadletKey,
  PADLET_DEFAULT_HEIGHT,
  PADLET_DEFAULT_TITLE,
  PADLET_EMBED_PATH,
  PADLET_HOST,
  padletEmbedSrc,
  padletRules,
  readPadletAddress,
} from "./padlet-input"
import { CONTENT_V1_POLICY, PolicyError } from "./policy"
import { policyWith } from "@/test/rich-text-policy"

const RULES = padletRules(CONTENT_V1_POLICY)

describe("padletRules", () => {
  it("reads the Padlet provider of policy.json", () => {
    expect(RULES).toEqual({
      hosts: ["padlet.com"],
      pathPrefix: "/embed/",
      idAlphabet: CONTENT_V1_POLICY.urls.embed.id_alphabet,
      idLength: null,
    })
    expect(PADLET_DEFAULT_HEIGHT).toBe(608)
    expect(PADLET_DEFAULT_TITLE).toBe("Padlet")
  })

  it.each<[string, (provider: Record<string, unknown>) => void]>([
    [
      "takes a query",
      (provider) => {
        provider.query = [{ name: "start", value: "digits" }]
      },
    ],
    [
      "has another path prefix, which the node would not write",
      (provider) => {
        provider.path_prefix = "/board/"
      },
    ],
    [
      "is not on padlet.com",
      (provider) => {
        provider.hosts = ["www.padlet.com"]
      },
    ],
  ])("refuses a policy whose Padlet provider %s", (_name, change) => {
    const policy = policyWith((json) => {
      const urls = json.urls as {
        embed: { providers: Record<string, unknown>[] }
      }
      change(urls.embed.providers[1])
    })
    expect(() => padletRules(policy)).toThrow(PolicyError)
    expect(() => padletRules(policy)).toThrow("no Padlet embed provider")
  })

  it("writes addresses on the host and path prefix it checked", () => {
    expect(padletEmbedSrc("abc")).toBe(
      `https://${PADLET_HOST}${RULES.pathPrefix}abc`
    )
    expect(RULES.hosts).toContain(PADLET_HOST)
    expect(RULES.pathPrefix).toBe(PADLET_EMBED_PATH)
  })
})

describe("readPadletAddress", () => {
  it.each([
    ["https://padlet.com/embed/bbcgkx6oq8valf3l", "bbcgkx6oq8valf3l"],
    ["  https://PADLET.com/embed/KEY_1-a \n", "KEY_1-a"],
    ["https://padlet.com:443/embed/k", "k"],
  ])("reads %j as the key %s", (text, key) => {
    expect(readPadletAddress(text, RULES)).toBe(key)
  })

  it.each([
    "",
    "KEY",
    "http://padlet.com/embed/KEY",
    "https://padlet.com/embed/",
    "https://padlet.com/embed/KEY/x",
    "https://padlet.com/embed/KEY?x=1",
    "https://padlet.com/embed/KEY?",
    "https://padlet.com/embed/KEY#a",
    "https://padlet.com/embed/K%20Y",
    "https://user@padlet.com/embed/KEY",
    "https://padlet.com:8443/embed/KEY",
    "https://www.padlet.com/embed/KEY",
    "https://padlet.com/someone/KEY",
    "https://padlet.com.evil.example/embed/KEY",
    '<iframe src="https://padlet.com/embed/KEY"></iframe>',
  ])("refuses %j", (text) => {
    expect(readPadletAddress(text, RULES)).toBeNull()
  })

  it("reads every key the server keeps, and only those", () => {
    fc.assert(
      fc.property(fc.string({ minLength: 1, maxLength: 24 }), (key) => {
        const src = `https://padlet.com/embed/${key}`
        const read = readPadletAddress(src, RULES)
        const stored = embedSrc(src, CONTENT_V1_POLICY.urls)
        if (read !== null) {
          expect(isPadletKey(read, RULES)).toBe(true)
          expect(padletEmbedSrc(read)).toBe(stored)
        }
        if (isPadletKey(key, RULES)) {
          expect(read).toBe(key)
        }
      })
    )
  })
})

describe("isPadletKey and padletEmbedSrc", () => {
  it("follows the alphabet and the policy's length", () => {
    expect(isPadletKey("aB3_-", RULES)).toBe(true)
    expect(isPadletKey("", RULES)).toBe(false)
    expect(isPadletKey("a b", RULES)).toBe(false)
    expect(isPadletKey("abc", { ...RULES, idLength: 4 })).toBe(false)
    expect(isPadletKey("abcd", { ...RULES, idLength: 4 })).toBe(true)
    expect(padletEmbedSrc("KEY")).toBe("https://padlet.com/embed/KEY")
  })
})
