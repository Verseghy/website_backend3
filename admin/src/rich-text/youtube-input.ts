/**
 * YoutubeEmbed's one reader of YouTube input (editor.md §4.3 row 9,
 * "YouTube input rules"; ADR 0043 §2-§4).
 *
 * Every route that turns YouTube input into a video goes through this
 * module, so the routes cannot read the same input differently: the node's
 * parse rules (an iframe's `src`), the lone-URL paste rule, the insert
 * dialog, and, from `paste-generic`, the plain-text route for embed code.
 * What it reads:
 *
 * - **Embed addresses** (`/embed/<id>`, `/embed/videoseries?list=`, and
 *   `/embed?listType=playlist&list=`, D129) on the two iframe hosts of
 *   `policy.json`. Only `start` is read there, as YouTube writes it.
 * - **Links** (D130): `youtu.be/<id>`; `/watch?v=<id>`, `/shorts/<id>` and
 *   `/playlist?list=<id>` on `www.youtube.com`, the bare `youtube.com`,
 *   `m.youtube.com` and `music.youtube.com`; and the embed addresses above.
 *   On a `youtu.be`, watch or shorts link a `t` of `^\d+s?$` becomes the
 *   start unless `start` is there (D131); `t=0` gives none.
 * - **Embed code** (D128): text that is exactly one `<iframe …></iframe>`,
 *   parsed in an inert `DOMParser` document, so its `src` is read as the
 *   browser reads it (`&amp;` decoded), never by a regular expression.
 *
 * `http` is read like `https` on every host and path (D136): the node always
 * writes its own `https://www.youtube-nocookie.com/embed/…` address. The
 * video id rule is the server's (D132): exactly the length that
 * `policy.json` gives the YouTube provider, of the shared id alphabet, or
 * the provider's list-only id (`videoseries`) together with a list. What a
 * reading drops that changes playback (`controls=0`, `end`, `autoplay=1`,
 * `loop`, `playlist`) is listed, so the routes can show the one notice of
 * D134; tracking and deprecated parameters (`si`, `pp`, `rel`, ...) are
 * dropped without a word.
 *
 * The module is pure apart from the `DOMParser` its caller passes in.
 */
import type { EmbedProvider, RichTextPolicy } from "./policy"
import { PolicyError } from "./policy"

/** The host the node writes every YouTube address on (editor.md §3.6). */
export const YOUTUBE_EMBED_HOST = "www.youtube-nocookie.com"

/**
 * The path prefix of every YouTube address the node writes (editor.md §3.7).
 * {@link youtubeRules} refuses a policy whose YouTube prefix differs.
 */
export const YOUTUBE_EMBED_PATH = "/embed/"

/** The title a YouTube embed gets when it has none of its own (D135). */
export const YOUTUBE_DEFAULT_TITLE = "Videó"

/**
 * The generic English title of YouTube's Share > Embed code, which the
 * title rule replaces like a missing one (D135).
 */
export const YOUTUBE_GENERIC_TITLE = "YouTube video player"

/**
 * The link hosts of D130 beside the iframe hosts: `/watch`, `/shorts` and
 * `/playlist` links are read on these. The bare host and `m.` and `music.`
 * are input mappings, which `policy.json` does not list (ADR 0043 §2).
 */
export const YOUTUBE_LINK_HOSTS: readonly string[] = [
  "www.youtube.com",
  "youtube.com",
  "m.youtube.com",
  "music.youtube.com",
]

/** The short-link host of YouTube's Share panel. */
export const YOUTUBE_SHORT_HOST = "youtu.be"

/**
 * The player parameters whose loss changes playback, in the order the
 * notice lists them (D134). `controls` and `autoplay` count only with the
 * values that change the player (`0` and `1`); the others count whenever
 * they are present.
 */
export const DROPPED_OPTIONS = [
  "controls",
  "end",
  "autoplay",
  "loop",
  "playlist",
] as const

/** One player option that a reading dropped and the notice reports. */
export type DroppedOption = (typeof DROPPED_OPTIONS)[number]

/** The YouTube rules of `policy.json` that the reader applies. */
export interface YoutubeRules {
  /** The iframe hosts (`www.youtube.com`, `www.youtube-nocookie.com`). */
  readonly embedHosts: readonly string[]
  /** The path prefix of an embed address (`/embed/`). */
  readonly pathPrefix: string
  /** The characters of a video id and of a `list` value. */
  readonly idAlphabet: string
  /** The exact length of a video id (11). */
  readonly idLength: number
  /** The path id that is a whole playlist, only with `list`. */
  readonly listOnlyId: string
}

/** What a stored YouTube embed plays: the node's attributes but the title. */
export interface YoutubeVideo {
  /** The video id, or the list-only id together with {@link list}. */
  readonly videoId: string
  /**
   * The start in seconds, as the digits of the address (`^[0-9]+$`), or
   * `null` for the beginning. It is text, not a number, so that a stored
   * address loads back byte for byte whatever digits the server kept
   * (§3.6 keeps any `start=<digits>`): `start=007` stays `007`, and a value
   * beyond `Number.MAX_SAFE_INTEGER` is not lost.
   */
  readonly start: string | null
  /** The playlist id, or `null`. */
  readonly list: string | null
}

/**
 * Why YouTube input is refused although it is a YouTube address:
 *
 * - `video_id`: the video id is not exactly the policy's length of the id
 *   alphabet, or `videoseries` comes without a list (D132, D129);
 * - `list_type`: a `listType` other than `playlist`, such as `user_uploads`
 *   (a user name, not a playlist) or `search` (no longer served), D129.
 */
export type YoutubeRefusal = "video_id" | "list_type"

/** The result of reading YouTube input. */
export type YoutubeReading =
  | {
      readonly kind: "video"
      /** What the embed plays. */
      readonly video: YoutubeVideo
      /**
       * The iframe's own `title`, as written, or `null`: links carry none.
       * The routes that insert apply {@link youtubeTitle} to it; the parse
       * rules keep it as it is (D135).
       */
      readonly title: string | null
      /** The playback options that were dropped, for the D134 notice. */
      readonly dropped: readonly DroppedOption[]
    }
  | { readonly kind: "refused"; readonly reason: YoutubeRefusal }
  | { readonly kind: "none" }

/** Input that is not YouTube input at all. */
const NONE: YoutubeReading = { kind: "none" }

/**
 * The YouTube rules of a policy: the provider whose hosts include
 * {@link YOUTUBE_EMBED_HOST}.
 *
 * @param policy - the content-v1 policy.
 * @returns the rules the reader applies.
 * @throws {PolicyError} when the policy has no such provider, or it lacks
 *   the id length, the list-only id or the `start` and `list` parameters
 *   that the reader relies on (ADR 0043 §3), or its path prefix is not
 *   {@link YOUTUBE_EMBED_PATH}, the one the node writes.
 */
export function youtubeRules(policy: RichTextPolicy): YoutubeRules {
  const provider: EmbedProvider | undefined = policy.urls.embed.providers.find(
    (candidate) => candidate.hosts.includes(YOUTUBE_EMBED_HOST)
  )
  const query = provider?.query.map(({ name, value }) => `${name}=${value}`)
  // The node writes its own address (youtubeEmbedSrc) rather than one built
  // from the policy, so a policy with another prefix stops the schema here
  // instead of letting the node write addresses the server would refuse.
  if (
    provider === undefined ||
    provider.path_prefix !== YOUTUBE_EMBED_PATH ||
    provider.id_length === null ||
    provider.list_only_id === null ||
    query?.join("&") !== "start=digits&list=id"
  ) {
    throw new PolicyError(
      `The policy has no YouTube embed provider on ${YOUTUBE_EMBED_HOST} with the path prefix ${YOUTUBE_EMBED_PATH}, an id length, a list-only id and the query start=digits&list=id (editor.md §3.6, ADR 0043 §3), which the YouTube embed reads; found ${JSON.stringify(provider ?? null)}.`
    )
  }
  return {
    embedHosts: provider.hosts,
    pathPrefix: provider.path_prefix,
    idAlphabet: policy.urls.embed.id_alphabet,
    idLength: provider.id_length,
    listOnlyId: provider.list_only_id,
  }
}

/** Whether `text` is one or more characters of the id alphabet. */
function inAlphabet(text: unknown, rules: YoutubeRules): text is string {
  return (
    typeof text === "string" &&
    text.length > 0 &&
    [...text].every((c) => rules.idAlphabet.includes(c))
  )
}

/**
 * Whether a video can be stored: its id has the policy's length, or it is
 * the list-only id with a list; the list and the start are valid too.
 *
 * @param video - the candidate; node attributes of any type are checked.
 * @param rules - the policy's YouTube rules ({@link youtubeRules}).
 * @returns `true` when the server keeps the address the node writes for it.
 */
export function isYoutubeVideo(
  video: YoutubeVideo,
  rules: YoutubeRules
): boolean {
  const listOk = video.list === null || inAlphabet(video.list, rules)
  const startOk = video.start === null || digits(video.start) !== null
  const idOk =
    video.videoId === rules.listOnlyId
      ? video.list !== null
      : inAlphabet(video.videoId, rules) &&
        video.videoId.length === rules.idLength
  return listOk && startOk && idOk
}

/**
 * The canonical address of a video (editor.md §3.7): always on
 * {@link YOUTUBE_EMBED_HOST}, `start` before `list`.
 *
 * @param video - what the embed plays.
 * @returns the `src` the node writes.
 */
export function youtubeEmbedSrc(video: YoutubeVideo): string {
  const query = [
    video.start === null ? null : `start=${video.start}`,
    video.list === null ? null : `list=${video.list}`,
  ].filter((part) => part !== null)
  return `https://${YOUTUBE_EMBED_HOST}${YOUTUBE_EMBED_PATH}${video.videoId}${
    query.length > 0 ? `?${query.join("&")}` : ""
  }`
}

/**
 * The title rule of D135: a missing or blank title, or exactly YouTube's
 * generic "YouTube video player" after trimming, becomes „Videó”; any other
 * title is kept as written. The routes that insert apply it; loading a
 * stored body does not, so a stored title survives.
 *
 * @param title - the title that came with the input, or `null`.
 * @returns the title to store.
 */
export function youtubeTitle(title: string | null): string {
  const trimmed = title?.trim() ?? ""
  return title === null || trimmed === "" || trimmed === YOUTUBE_GENERIC_TITLE
    ? YOUTUBE_DEFAULT_TITLE
    : title
}

/**
 * A seconds value as the server's `start=<digits>` takes it (§3.6): one or
 * more ASCII digits, kept as written.
 *
 * @param value - a query value, or anything else a node attribute holds.
 * @returns the digits, or `null` when `value` is not such text.
 */
function digits(value: unknown): string | null {
  return typeof value === "string" && /^[0-9]+$/.test(value) ? value : null
}

/**
 * The start of a link (D131): a valid `start` wins; otherwise a `t` of
 * `^\d+s?$`, where zero means the beginning. Any other `t` is dropped.
 */
function linkStart(params: URLSearchParams): string | null {
  const start = digits(params.get("start"))
  if (start !== null) {
    return start
  }
  const t = /^([0-9]+)s?$/.exec(params.get("t") ?? "")
  return t === null || /^0+$/.test(t[1]) ? null : t[1]
}

/** The playback options of a query that the reading drops (D134). */
function droppedOptions(params: URLSearchParams): DroppedOption[] {
  return DROPPED_OPTIONS.filter((option) => {
    const value = params.get(option)
    switch (option) {
      case "controls":
        return value === "0"
      case "autoplay":
        return value === "1"
      default:
        return value !== null
    }
  })
}

/** The video a path id names, checked, or the refusal. */
function video(
  videoId: string,
  start: string | null,
  params: URLSearchParams,
  rules: YoutubeRules
): YoutubeReading {
  const listValue = params.get("list")
  const list =
    listValue !== null && inAlphabet(listValue, rules) ? listValue : null
  const candidate: YoutubeVideo = { videoId, start, list }
  if (!isYoutubeVideo(candidate, rules)) {
    return { kind: "refused", reason: "video_id" }
  }
  return {
    kind: "video",
    video: candidate,
    title: null,
    dropped: droppedOptions(params),
  }
}

/** Parses an `http` or `https` URL with no user info and no port. */
function parseWebUrl(text: string): URL | null {
  let url: URL
  try {
    url = new URL(text)
  } catch {
    return null
  }
  const plain =
    (url.protocol === "https:" || url.protocol === "http:") &&
    url.username === "" &&
    url.password === "" &&
    url.port === ""
  return plain ? url : null
}

/** Reads an embed address whose host is one of the iframe hosts. */
function readEmbedPath(url: URL, rules: YoutubeRules): YoutubeReading {
  const params = url.searchParams
  const listType = params.get("listType")
  if (listType !== null && listType !== "playlist") {
    return { kind: "refused", reason: "list_type" }
  }
  const base = rules.pathPrefix.slice(0, -1)
  const start = digits(params.get("start"))
  if (url.pathname === base || url.pathname === rules.pathPrefix) {
    // `/embed?listType=playlist&list=<id>`: the only playlist form of
    // YouTube's docs, which becomes `/embed/videoseries?list=<id>` (D129).
    return listType === "playlist"
      ? video(rules.listOnlyId, start, params, rules)
      : NONE
  }
  if (!url.pathname.startsWith(rules.pathPrefix)) {
    return NONE
  }
  const id = url.pathname.slice(rules.pathPrefix.length)
  return id.includes("/") ? NONE : video(id, start, params, rules)
}

/**
 * Reads an iframe address: an embed address on one of the iframe hosts,
 * `http` included (D136). Links are not iframe addresses.
 *
 * @param src - the iframe's `src`, as the DOM reads it (`&amp;` decoded).
 * @param rules - the policy's YouTube rules ({@link youtubeRules}).
 * @returns the reading; `none` for an address that is not YouTube's.
 */
export function readYoutubeEmbedSrc(
  src: string,
  rules: YoutubeRules
): YoutubeReading {
  const url = parseWebUrl(src.trim())
  return url !== null && rules.embedHosts.includes(url.hostname)
    ? readEmbedPath(url, rules)
    : NONE
}

/**
 * Reads a lone URL (D130, D131, D136): a share link, a watch, shorts or
 * playlist page, or an embed address.
 *
 * @param text - the URL; leading and trailing whitespace is ignored.
 * @param rules - the policy's YouTube rules ({@link youtubeRules}).
 * @returns the reading; `none` for text that is not one YouTube URL.
 */
export function readYoutubeUrl(
  text: string,
  rules: YoutubeRules
): YoutubeReading {
  const trimmed = text.trim()
  if (/\s/.test(trimmed)) {
    return NONE
  }
  const url = parseWebUrl(trimmed)
  if (url === null) {
    return NONE
  }
  const params = url.searchParams
  const path = url.pathname
  if (url.hostname === YOUTUBE_SHORT_HOST) {
    const id = /^\/([^/]+)$/.exec(path)
    return id === null ? NONE : video(id[1], linkStart(params), params, rules)
  }
  if (YOUTUBE_LINK_HOSTS.includes(url.hostname)) {
    if (path === "/watch") {
      const id = params.get("v")
      return id === null ? NONE : video(id, linkStart(params), params, rules)
    }
    const short = /^\/shorts\/([^/]+)$/.exec(path)
    if (short !== null) {
      return video(short[1], linkStart(params), params, rules)
    }
    if (path === "/playlist") {
      // A playlist page plays the whole list (D130).
      return params.get("list") === null
        ? NONE
        : video(rules.listOnlyId, null, params, rules)
    }
  }
  return rules.embedHosts.includes(url.hostname)
    ? readEmbedPath(url, rules)
    : NONE
}

/**
 * Reads a YouTube iframe element: its `src` through the DOM, and its own
 * `title` as written.
 *
 * @param iframe - an `iframe` element.
 * @param rules - the policy's YouTube rules ({@link youtubeRules}).
 * @returns the reading; `none` when the element is no YouTube embed.
 */
export function readYoutubeIframe(
  iframe: Element,
  rules: YoutubeRules
): YoutubeReading {
  const src = iframe.localName === "iframe" ? iframe.getAttribute("src") : null
  const reading = src === null ? NONE : readYoutubeEmbedSrc(src, rules)
  return reading.kind === "video"
    ? { ...reading, title: iframe.getAttribute("title") }
    : reading
}

/**
 * The one `iframe` of embed code: text that is exactly one
 * `<iframe …></iframe>` element, apart from surrounding whitespace, parsed
 * in an inert document (D128).
 *
 * @param text - the text to read.
 * @param parser - the parser; its documents have no browsing context, so
 *   nothing in them loads or runs.
 * @returns the element, or `null` when the text is anything else.
 */
export function embedCodeIframe(
  text: string,
  parser: DOMParser
): Element | null {
  const trimmed = text.trim()
  if (!/^<iframe[\s/>][\s\S]*<\/iframe>$/i.test(trimmed)) {
    return null
  }
  const document = parser.parseFromString(trimmed, "text/html")
  const nodes = [...document.body.childNodes]
  const [iframe] = nodes
  return nodes.length === 1 &&
    document.head.childNodes.length === 0 &&
    iframe.nodeType === iframe.ELEMENT_NODE &&
    (iframe as Element).localName === "iframe" &&
    iframe.childNodes.length === 0
    ? (iframe as Element)
    : null
}

/**
 * Reads YouTube's embed code (D128): exactly one iframe whose address is a
 * YouTube embed address.
 *
 * @param text - the code.
 * @param rules - the policy's YouTube rules ({@link youtubeRules}).
 * @param parser - the parser of {@link embedCodeIframe}.
 * @returns the reading, with the iframe's own title; `none` for anything
 *   else, an iframe from another host included.
 */
export function readYoutubeEmbedCode(
  text: string,
  rules: YoutubeRules,
  parser: DOMParser
): YoutubeReading {
  const iframe = embedCodeIframe(text, parser)
  return iframe === null ? NONE : readYoutubeIframe(iframe, rules)
}

/**
 * Reads what staff type or paste into the insert dialog: embed code when
 * the text starts with `<`, otherwise a lone URL.
 *
 * @param text - the dialog's input.
 * @param rules - the policy's YouTube rules ({@link youtubeRules}).
 * @param parser - the parser of {@link embedCodeIframe}.
 * @returns the reading.
 */
export function readYoutubeInput(
  text: string,
  rules: YoutubeRules,
  parser: DOMParser
): YoutubeReading {
  return text.trim().startsWith("<")
    ? readYoutubeEmbedCode(text, rules, parser)
    : readYoutubeUrl(text, rules)
}
