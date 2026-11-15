import fc from "fast-check"
import { describe, expect, it } from "vitest"

import { embedSrc } from "./media-src"
import { CONTENT_V1_POLICY, PolicyError } from "./policy"
import type { YoutubeReading, YoutubeVideo } from "./youtube-input"
import {
  DROPPED_OPTIONS,
  embedCodeIframe,
  isYoutubeVideo,
  readYoutubeEmbedCode,
  readYoutubeEmbedSrc,
  readYoutubeIframe,
  readYoutubeInput,
  readYoutubeUrl,
  YOUTUBE_DEFAULT_TITLE,
  YOUTUBE_EMBED_HOST,
  YOUTUBE_EMBED_PATH,
  YOUTUBE_GENERIC_TITLE,
  youtubeEmbedSrc,
  youtubeRules,
  youtubeTitle,
} from "./youtube-input"
import { policyWith } from "@/test/rich-text-policy"
import { youtubeFixture, youtubeFixtures } from "@/test/youtube-fixtures"

const RULES = youtubeRules(CONTENT_V1_POLICY)
const parser = new DOMParser()

/** A video reading's parts, or the reading itself when it is no video. */
function summary(reading: YoutubeReading) {
  return reading.kind === "video"
    ? { ...reading.video, title: reading.title, dropped: reading.dropped }
    : reading
}

/** The parts of a reading that must be a video. */
function videoParts(reading: YoutubeReading) {
  if (reading.kind !== "video") {
    throw new Error(`not a video: ${JSON.stringify(reading)}`)
  }
  return { ...reading.video, title: reading.title, dropped: reading.dropped }
}

const video = (
  videoId: string,
  rest: Partial<{
    start: string
    list: string
    title: string | null
    dropped: string[]
  }> = {}
) => ({
  videoId,
  start: rest.start ?? null,
  list: rest.list ?? null,
  title: rest.title ?? null,
  dropped: rest.dropped ?? [],
})

const PLAYER_TITLE = "YouTube video player"

/**
 * What every research fixture reads as (ADR 0043, "What must be true in
 * code and tests", Editor (L1)): code through the code route, links through
 * the lone-URL route.
 */
const EXPECTED: Readonly<Record<string, ReturnType<typeof video>>> = {
  "youtube-embed-dialog-default": video("dQw4w9WgXcQ", { title: PLAYER_TITLE }),
  "youtube-embed-dialog-start": video("dQw4w9WgXcQ", {
    start: "90",
    title: PLAYER_TITLE,
  }),
  "youtube-embed-dialog-controls-off": video("dQw4w9WgXcQ", {
    title: PLAYER_TITLE,
    dropped: ["controls"],
  }),
  "youtube-embed-dialog-privacy": video("dQw4w9WgXcQ", { title: PLAYER_TITLE }),
  "youtube-embed-dialog-all-options": video("dQw4w9WgXcQ", {
    start: "90",
    title: PLAYER_TITLE,
    dropped: ["controls"],
  }),
  "youtube-embed-dialog-short": video("B5T4ZCfxp1Y", { title: PLAYER_TITLE }),
  "youtube-embed-dialog-playlist": video("videoseries", {
    list: "PLMOZDTW7ZvXM",
    title: PLAYER_TITLE,
  }),
  "youtube-embed-dialog-playlist-all-options": video("videoseries", {
    list: "PLMOZDTW7ZvXM",
    title: PLAYER_TITLE,
    dropped: ["controls"],
  }),
  "youtube-embed-context-menu-video": video("dQw4w9WgXcQ", {
    title:
      "Rick Astley - Never Gonna Give You Up (Official Video) (4K Remaster)",
  }),
  "youtube-embed-context-menu-playlist": video("71ya_JC_IbM", {
    list: "PLMOZDTW7ZvXM",
    title: "20 years of esports history explained",
  }),
  "youtube-dev-sample-autoplay-origin": video("M7lc1UVf-VE", {
    dropped: ["autoplay"],
  }),
  "youtube-help-autoplay": video("D6Ac5JpCHmI", { dropped: ["autoplay"] }),
  "youtube-help-privacy-before": video("7cjVj1ZyzyE"),
  "youtube-help-start-http": video("UkWd0azv3fQ", { start: "90" }),
  "youtube-context-menu-url-playlist-time": video("71ya_JC_IbM", {
    start: "8",
    list: "PLMOZDTW7ZvXM",
  }),
  "youtube-context-menu-url-time": video("dQw4w9WgXcQ", { start: "97" }),
  "youtube-documented-embed-deprecated-params": video("dQw4w9WgXcQ"),
  "youtube-documented-embed-listtype-playlist": video("videoseries", {
    list: "PLC77007E23FF423C6",
  }),
  "youtube-documented-embed-loop": video("dQw4w9WgXcQ", {
    dropped: ["loop", "playlist"],
  }),
  "youtube-documented-embed-start-end": video("dQw4w9WgXcQ", {
    start: "30",
    dropped: ["end"],
  }),
  "youtube-share-link-playlist": video("videoseries", {
    list: "PLMOZDTW7ZvXM",
  }),
  "youtube-share-link-short": video("B5T4ZCfxp1Y"),
  "youtube-share-link-video-start": video("dQw4w9WgXcQ", { start: "90" }),
  "youtube-share-link-video": video("dQw4w9WgXcQ"),
  "youtube-shorts-url": video("B5T4ZCfxp1Y"),
  // A Mix list is kept as given, as editor.md §3.6 keeps every list.
  "youtube-watch-url-mix": video("71ya_JC_IbM", { list: "RD71ya_JC_IbM" }),
  "youtube-watch-url-playlist": video("71ya_JC_IbM", {
    list: "PLMOZDTW7ZvXM",
  }),
  "youtube-watch-url-time": video("dQw4w9WgXcQ", { start: "90" }),
}

describe("youtubeRules", () => {
  it("reads the YouTube provider of policy.json", () => {
    expect(RULES).toEqual({
      embedHosts: ["www.youtube.com", "www.youtube-nocookie.com"],
      pathPrefix: "/embed/",
      idAlphabet: CONTENT_V1_POLICY.urls.embed.id_alphabet,
      idLength: 11,
      listOnlyId: "videoseries",
    })
  })

  it.each<[string, (json: Record<string, unknown>) => void]>([
    [
      "no id length",
      (provider) => {
        provider.id_length = null
      },
    ],
    [
      "no list-only id",
      (provider) => {
        provider.list_only_id = null
      },
    ],
    [
      "another query",
      (provider) => {
        provider.query = [{ name: "start", value: "digits" }]
      },
    ],
    [
      "no nocookie host",
      (provider) => {
        provider.hosts = ["www.youtube.com"]
      },
    ],
    [
      "another path prefix, which the node would not write",
      (provider) => {
        provider.path_prefix = "/v/"
      },
    ],
  ])("refuses a policy with %s", (_name, change) => {
    const policy = policyWith((json) => {
      const urls = json.urls as Record<string, Record<string, unknown>>
      const providers = urls.embed.providers as Record<string, unknown>[]
      change(providers[0])
    })
    expect(() => youtubeRules(policy)).toThrow(PolicyError)
    expect(() => youtubeRules(policy)).toThrow("no YouTube embed provider")
  })

  it("writes addresses on a host and path prefix the policy allows", () => {
    const src = youtubeEmbedSrc({
      videoId: "dQw4w9WgXcQ",
      start: null,
      list: null,
    })
    expect(src).toBe(
      `https://${YOUTUBE_EMBED_HOST}${YOUTUBE_EMBED_PATH}dQw4w9WgXcQ`
    )
    expect(RULES.embedHosts).toContain(YOUTUBE_EMBED_HOST)
    expect(RULES.pathPrefix).toBe(YOUTUBE_EMBED_PATH)
  })
})

describe("the research fixtures", () => {
  const fixtures = youtubeFixtures()

  it("are all expected", () => {
    expect(fixtures.map(({ name }) => name).sort()).toEqual(
      Object.keys(EXPECTED).sort()
    )
  })

  it.each(fixtures.filter(({ kind }) => kind === "code"))(
    "embed code $name, fed as text, becomes its embed (D128)",
    ({ name, text }) => {
      expect(summary(readYoutubeEmbedCode(text, RULES, parser))).toEqual(
        EXPECTED[name]
      )
      expect(summary(readYoutubeInput(text, RULES, parser))).toEqual(
        EXPECTED[name]
      )
    }
  )

  it.each(fixtures.filter(({ kind }) => kind === "link"))(
    "the link $name becomes its embed (D129-D131)",
    ({ name, text }) => {
      expect(summary(readYoutubeUrl(text, RULES))).toEqual(EXPECTED[name])
      expect(summary(readYoutubeInput(text, RULES, parser))).toEqual(
        EXPECTED[name]
      )
    }
  )

  it("gives the t fixtures their start (D131)", () => {
    const src = (name: string) => {
      const reading = readYoutubeUrl(youtubeFixture(name), RULES)
      return reading.kind === "video" ? youtubeEmbedSrc(reading.video) : null
    }
    expect(src("youtube-share-link-video-start")).toBe(
      "https://www.youtube-nocookie.com/embed/dQw4w9WgXcQ?start=90"
    )
    expect(src("youtube-context-menu-url-time")).toBe(
      "https://www.youtube-nocookie.com/embed/dQw4w9WgXcQ?start=97"
    )
    expect(src("youtube-context-menu-url-playlist-time")).toBe(
      "https://www.youtube-nocookie.com/embed/71ya_JC_IbM?start=8&list=PLMOZDTW7ZvXM"
    )
    expect(src("youtube-watch-url-time")).toBe(
      "https://www.youtube-nocookie.com/embed/dQw4w9WgXcQ?start=90"
    )
  })

  it("upgrades YouTube Help's http example (D136)", () => {
    const reading = readYoutubeEmbedCode(
      youtubeFixture("youtube-help-start-http"),
      RULES,
      parser
    )
    expect(reading.kind === "video" && youtubeEmbedSrc(reading.video)).toBe(
      "https://www.youtube-nocookie.com/embed/UkWd0azv3fQ?start=90"
    )
  })

  it("embeds both captured share links, the Short and the playlist (D130)", () => {
    for (const [name, src] of [
      [
        "youtube-share-link-short",
        "https://www.youtube-nocookie.com/embed/B5T4ZCfxp1Y",
      ],
      [
        "youtube-share-link-playlist",
        "https://www.youtube-nocookie.com/embed/videoseries?list=PLMOZDTW7ZvXM",
      ],
    ]) {
      const reading = readYoutubeUrl(youtubeFixture(name), RULES)
      expect(reading.kind === "video" && youtubeEmbedSrc(reading.video)).toBe(
        src
      )
    }
  })

  it("reports dropped options exactly for the fixtures that set them (D134)", () => {
    const withNotice = fixtures
      .filter(({ name }) => EXPECTED[name].dropped.length > 0)
      .map(({ name }) => name)
      .sort()
    expect(withNotice).toEqual(
      [
        "youtube-dev-sample-autoplay-origin",
        "youtube-documented-embed-loop",
        "youtube-documented-embed-start-end",
        "youtube-embed-dialog-all-options",
        "youtube-embed-dialog-controls-off",
        "youtube-embed-dialog-playlist-all-options",
        "youtube-help-autoplay",
      ].sort()
    )
  })

  it("stores every fixture's embed at an address the server keeps", () => {
    for (const { name } of fixtures) {
      const { videoId, start, list } = EXPECTED[name]
      const src = youtubeEmbedSrc({ videoId, start, list })
      expect(embedSrc(src, CONTENT_V1_POLICY.urls), name).toBe(src)
    }
  })
})

describe("readYoutubeUrl", () => {
  const read = (text: string) => summary(readYoutubeUrl(text, RULES))
  const parts = (text: string) => videoParts(readYoutubeUrl(text, RULES))

  it.each([
    ["https://youtu.be/dQw4w9WgXcQ", "dQw4w9WgXcQ"],
    ["https://www.youtube.com/watch?v=dQw4w9WgXcQ", "dQw4w9WgXcQ"],
    ["https://youtube.com/watch?v=dQw4w9WgXcQ&feature=share", "dQw4w9WgXcQ"],
    ["https://m.youtube.com/watch?v=dQw4w9WgXcQ", "dQw4w9WgXcQ"],
    ["https://music.youtube.com/watch?v=dQw4w9WgXcQ", "dQw4w9WgXcQ"],
    ["https://m.youtube.com/shorts/B5T4ZCfxp1Y", "B5T4ZCfxp1Y"],
    ["https://www.youtube.com/embed/dQw4w9WgXcQ", "dQw4w9WgXcQ"],
    ["https://www.youtube-nocookie.com/embed/dQw4w9WgXcQ", "dQw4w9WgXcQ"],
    // http is read like https on every host and path (D136).
    ["http://youtu.be/dQw4w9WgXcQ", "dQw4w9WgXcQ"],
    ["http://www.youtube.com/watch?v=dQw4w9WgXcQ", "dQw4w9WgXcQ"],
    ["  https://youtu.be/dQw4w9WgXcQ\n", "dQw4w9WgXcQ"],
    ["https://WWW.YouTube.com/watch?v=dQw4w9WgXcQ#x", "dQw4w9WgXcQ"],
  ])("reads %s", (text, videoId) => {
    expect(read(text)).toEqual(video(videoId))
  })

  it("maps t only on youtu.be, watch and shorts links (D131)", () => {
    expect(parts("https://youtu.be/dQw4w9WgXcQ?t=90s").start).toBe("90")
    expect(parts("https://www.youtube.com/shorts/B5T4ZCfxp1Y?t=5").start).toBe(
      "5"
    )
    // start wins; t=0 is the beginning; other spellings are dropped.
    expect(
      parts("https://www.youtube.com/watch?v=dQw4w9WgXcQ&t=90&start=12").start
    ).toBe("12")
    expect(parts("https://youtu.be/dQw4w9WgXcQ?t=0").start).toBeNull()
    expect(parts("https://youtu.be/dQw4w9WgXcQ?t=000s").start).toBeNull()
    expect(parts("https://youtu.be/dQw4w9WgXcQ?t=1m30s").start).toBeNull()
    expect(parts("https://youtu.be/dQw4w9WgXcQ?t=-5").start).toBeNull()
    // An embed address reads only start, as YouTube writes it there.
    expect(parts("https://www.youtube.com/embed/dQw4w9WgXcQ?t=90").start).toBe(
      null
    )
    expect(
      parts("https://www.youtube.com/embed/dQw4w9WgXcQ?start=0").start
    ).toBe("0")
  })

  it("keeps a start's digits as written, so a stored address loads unchanged", () => {
    // The server keeps any `start=<digits>` (editor.md §3.6), so a body
    // written through the API may carry leading zeros or a value beyond a
    // safe integer; the node writes them back byte for byte.
    for (const start of ["007", "0", "99999999999999999999"]) {
      const src = `https://www.youtube-nocookie.com/embed/dQw4w9WgXcQ?start=${start}`
      expect(parts(src).start).toBe(start)
      expect(youtubeEmbedSrc(parts(src))).toBe(src)
    }
    expect(parts("https://youtu.be/dQw4w9WgXcQ?t=090s").start).toBe("090")
  })

  it("maps listType=playlist to videoseries and refuses the others (D129)", () => {
    expect(
      read("https://www.youtube.com/embed/?listType=playlist&list=PL1")
    ).toEqual(video("videoseries", { list: "PL1" }))
    for (const listType of ["user_uploads", "search", "other"]) {
      expect(
        read(`https://www.youtube.com/embed?listType=${listType}&list=x`)
      ).toEqual({ kind: "refused", reason: "list_type" })
    }
    expect(read("https://www.youtube.com/embed")).toEqual({ kind: "none" })
  })

  it.each([
    "https://youtu.be/abc",
    "https://youtu.be/dQw4w9WgXcQx",
    "https://www.youtube.com/watch?v=dQw4w9WgXc",
    "https://www.youtube.com/shorts/dQw4w9Wg%20Q",
    "https://www.youtube.com/embed/videoseries",
    "https://www.youtube.com/watch?v=videoseries",
    "https://youtube.com/playlist?list=a.b",
  ])("refuses %s for its id (D132)", (text) => {
    expect(read(text)).toEqual({ kind: "refused", reason: "video_id" })
  })

  it.each([
    "",
    "dQw4w9WgXcQ",
    "see https://youtu.be/dQw4w9WgXcQ",
    "https://youtu.be/dQw4w9WgXcQ https://youtu.be/dQw4w9WgXcQ",
    "https://vimeo.com/123",
    "https://www.youtube.com/@channel",
    "https://www.youtube.com/watch",
    "https://www.youtube.com/playlist",
    "https://youtu.be/",
    "https://youtu.be/dQw4w9WgXcQ/x",
    "https://youtube.com/embed/dQw4w9WgXcQ",
    "https://m.youtube.com/embed/dQw4w9WgXcQ",
    "https://www.youtube.com.evil.example/watch?v=dQw4w9WgXcQ",
    "https://user@www.youtube.com/watch?v=dQw4w9WgXcQ",
    "https://www.youtube.com:8443/watch?v=dQw4w9WgXcQ",
    "ftp://www.youtube.com/watch?v=dQw4w9WgXcQ",
    "javascript:alert(1)",
  ])("leaves %j alone", (text) => {
    expect(read(text)).toEqual({ kind: "none" })
  })

  it("drops tracking parameters without a notice and lists playback ones", () => {
    expect(
      parts("https://youtu.be/dQw4w9WgXcQ?si=x&pp=y&feature=z&rel=0").dropped
    ).toEqual([])
    expect(
      parts(
        "https://www.youtube.com/embed/dQw4w9WgXcQ?playlist=a&loop=0&autoplay=1&end=5&controls=0"
      ).dropped
    ).toEqual([...DROPPED_OPTIONS])
    expect(
      parts("https://www.youtube.com/embed/dQw4w9WgXcQ?autoplay=0&controls=1")
        .dropped
    ).toEqual([])
  })
})

describe("readYoutubeEmbedSrc", () => {
  it("reads only embed addresses on the iframe hosts", () => {
    expect(
      readYoutubeEmbedSrc("https://youtu.be/dQw4w9WgXcQ", RULES).kind
    ).toBe("none")
    expect(
      readYoutubeEmbedSrc("https://www.youtube.com/watch?v=dQw4w9WgXcQ", RULES)
        .kind
    ).toBe("none")
    expect(
      summary(
        readYoutubeEmbedSrc(
          "http://www.youtube.com/embed/dQw4w9WgXcQ?start=3&list=PL1",
          RULES
        )
      )
    ).toEqual(video("dQw4w9WgXcQ", { start: "3", list: "PL1" }))
  })
})

describe("embed code", () => {
  const code = youtubeFixture("youtube-embed-dialog-default")

  it("is exactly one iframe, apart from whitespace", () => {
    expect(embedCodeIframe(`\n  ${code}  \n`, parser)?.localName).toBe("iframe")
    expect(
      embedCodeIframe(code.replace("<iframe", "<IFRAME"), parser)
    ).not.toBeNull()
    for (const text of [
      `${code}${code}`,
      `${code} szöveg`,
      `<p>${code}</p>`,
      `<div class="padlet-embed">${code}</div>`,
      `<iframe src="x">szöveg</iframe>`,
      `<title>x</title>${code}`,
      code.replace("</iframe>", ""),
      "https://youtu.be/dQw4w9WgXcQ",
      "",
    ]) {
      expect(embedCodeIframe(text, parser), text).toBeNull()
    }
  })

  it("is read only from YouTube's hosts", () => {
    expect(
      readYoutubeEmbedCode(
        '<iframe src="https://padlet.com/embed/KEY"></iframe>',
        RULES,
        parser
      )
    ).toEqual({ kind: "none" })
    expect(
      readYoutubeEmbedCode(
        '<iframe src="https://evil.example/embed/dQw4w9WgXcQ"></iframe>',
        RULES,
        parser
      )
    ).toEqual({ kind: "none" })
  })

  it("reads the src through the DOM, so &amp; is decoded", () => {
    const reading = readYoutubeEmbedCode(
      '<iframe src="https://www.youtube.com/embed/dQw4w9WgXcQ?si=a&amp;start=7"></iframe>',
      RULES,
      parser
    )
    expect(summary(reading)).toEqual(video("dQw4w9WgXcQ", { start: "7" }))
  })

  it("reads an iframe element's own title, as written", () => {
    const iframe = document.createElement("iframe")
    iframe.setAttribute("src", "https://www.youtube.com/embed/dQw4w9WgXcQ")
    iframe.setAttribute("title", "  Saját cím  ")
    expect(videoParts(readYoutubeIframe(iframe, RULES)).title).toBe(
      "  Saját cím  "
    )
    const div = document.createElement("div")
    div.setAttribute("src", "https://www.youtube.com/embed/dQw4w9WgXcQ")
    expect(readYoutubeIframe(div, RULES)).toEqual({ kind: "none" })
  })
})

describe("youtubeTitle (D135)", () => {
  it.each<[string | null, string]>([
    [null, YOUTUBE_DEFAULT_TITLE],
    ["", YOUTUBE_DEFAULT_TITLE],
    ["   ", YOUTUBE_DEFAULT_TITLE],
    [YOUTUBE_GENERIC_TITLE, YOUTUBE_DEFAULT_TITLE],
    [`  ${YOUTUBE_GENERIC_TITLE}\n`, YOUTUBE_DEFAULT_TITLE],
    ["youtube video player", "youtube video player"],
    ["A ballagás", "A ballagás"],
    [" A ballagás ", " A ballagás "],
  ])("makes %j into %j", (title, expected) => {
    expect(youtubeTitle(title)).toBe(expected)
  })
})

describe("isYoutubeVideo and youtubeEmbedSrc", () => {
  it.each<[YoutubeVideo, boolean]>([
    [{ videoId: "dQw4w9WgXcQ", start: null, list: null }, true],
    [{ videoId: "dQw4w9WgXcQ", start: "0", list: "PL1" }, true],
    [{ videoId: "videoseries", start: null, list: "PL1" }, true],
    [{ videoId: "videoseries", start: null, list: null }, false],
    [{ videoId: "dQw4w9WgXc", start: null, list: null }, false],
    [{ videoId: "dQw4w9WgXcQ", start: "-1", list: null }, false],
    [{ videoId: "dQw4w9WgXcQ", start: "1.5", list: null }, false],
    [{ videoId: "dQw4w9WgXcQ", start: "007", list: null }, true],
    [
      { videoId: "dQw4w9WgXcQ", start: "99999999999999999999", list: null },
      true,
    ],
    [{ videoId: "dQw4w9WgXcQ", start: "", list: null }, false],
    [{ videoId: "dQw4w9WgXcQ", start: " 5", list: null }, false],
    [
      {
        videoId: "dQw4w9WgXcQ",
        start: 5,
        list: null,
      } as unknown as YoutubeVideo,
      false,
    ],
    [{ videoId: "dQw4w9WgXcQ", start: null, list: "a.b" }, false],
    [
      { videoId: null, start: null, list: null } as unknown as YoutubeVideo,
      false,
    ],
  ])("judges %j as %s", (candidate, valid) => {
    expect(isYoutubeVideo(candidate, RULES)).toBe(valid)
  })

  it("writes start before list on the nocookie host", () => {
    expect(
      youtubeEmbedSrc({ videoId: "dQw4w9WgXcQ", start: "5", list: "PL1" })
    ).toBe(
      "https://www.youtube-nocookie.com/embed/dQw4w9WgXcQ?start=5&list=PL1"
    )
  })
})

/** Ids of exactly 11 characters of the id alphabet. */
const elevenCharacterId = fc.stringMatching(/^[A-Za-z0-9_-]{11}$/)

describe("properties", () => {
  it("reads every link form of one video to the same embed, which the server keeps", () => {
    fc.assert(
      fc.property(
        elevenCharacterId,
        // Any digits with one that is not zero (t=0 is no start), leading
        // zeros and values beyond a safe integer included.
        fc.option(fc.stringMatching(/^[0-9]{0,3}[1-9][0-9]{0,20}$/), {
          nil: null,
        }),
        fc.constantFrom("http", "https"),
        (id, start, scheme) => {
          const t = start === null ? "" : `t=${start}`
          const forms = [
            `${scheme}://youtu.be/${id}${t ? `?${t}` : ""}`,
            ...[
              "www.youtube.com",
              "youtube.com",
              "m.youtube.com",
              "music.youtube.com",
            ].flatMap((host) => [
              `${scheme}://${host}/watch?v=${id}${t ? `&${t}` : ""}`,
              `${scheme}://${host}/shorts/${id}${t ? `?${t}` : ""}`,
            ]),
            ...RULES.embedHosts.map(
              (host) =>
                `${scheme}://${host}/embed/${id}${start === null ? "" : `?start=${start}`}`
            ),
          ]
          for (const form of forms) {
            const reading = readYoutubeUrl(form, RULES)
            expect(summary(reading), form).toEqual(
              video(id, start === null ? {} : { start })
            )
            const src = youtubeEmbedSrc({ videoId: id, start, list: null })
            expect(embedSrc(src, CONTENT_V1_POLICY.urls)).toBe(src)
          }
        }
      )
    )
  })

  it("refuses video ids of any other length", () => {
    fc.assert(
      fc.property(
        fc
          .stringMatching(/^[A-Za-z0-9_-]{1,20}$/)
          .filter((id) => id.length !== 11),
        (id) => {
          expect(readYoutubeUrl(`https://youtu.be/${id}`, RULES)).toEqual({
            kind: "refused",
            reason: "video_id",
          })
        }
      )
    )
  })

  it("never throws, and every video it reads is stored as the server keeps it", () => {
    fc.assert(
      fc.property(
        fc.oneof(
          fc.string(),
          fc.webUrl({ withQueryParameters: true, withFragments: true }),
          fc
            .tuple(
              fc.constantFrom(
                "https://youtu.be/",
                "https://www.youtube.com/watch?v=",
                "https://www.youtube.com/embed/",
                '<iframe src="https://www.youtube.com/embed/'
              ),
              fc.string(),
              fc.constantFrom("", '"></iframe>', "&list=PL1", "?list=x&t=4s")
            )
            .map((parts) => parts.join(""))
        ),
        (text) => {
          const reading = readYoutubeInput(text, RULES, parser)
          if (reading.kind === "video") {
            expect(isYoutubeVideo(reading.video, RULES)).toBe(true)
            const src = youtubeEmbedSrc(reading.video)
            expect(embedSrc(src, CONTENT_V1_POLICY.urls)).toBe(src)
          }
        }
      )
    )
  })
})
