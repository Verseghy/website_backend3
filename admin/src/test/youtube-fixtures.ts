/**
 * Test support for the YouTube input fixtures of `admin/test/fixtures/youtube/`
 * (editor.md §13.3 "YouTube input", ADR 0043): YouTube's embed code and links
 * as its Share dialog, share links and player menu wrote them, and the forms
 * its developer and Help pages print. The directory's README says where each
 * file comes from.
 *
 * Vite reads the files at transform time, so this works in every test
 * environment.
 */

/** The fixture files by path. */
const FILES = import.meta.glob<string>(
  "../../test/fixtures/youtube/youtube-*.{html,txt}",
  { query: "?raw", import: "default", eager: true }
)

/** One fixture: its file name without the extension, its kind and text. */
export interface YoutubeFixture {
  /** The file name without `.html` or `.txt`. */
  readonly name: string
  /** `code` for embed code (`.html`), `link` for one URL (`.txt`). */
  readonly kind: "code" | "link"
  /** The file's text, exactly as committed. */
  readonly text: string
}

/** Every YouTube fixture, in file-name order. */
export function youtubeFixtures(): YoutubeFixture[] {
  const fixtures = Object.keys(FILES)
    .sort()
    .map((path) => {
      const file = path.slice(path.lastIndexOf("/") + 1)
      const code = file.endsWith(".html")
      return {
        name: file.slice(0, code ? -".html".length : -".txt".length),
        kind: code ? ("code" as const) : ("link" as const),
        text: FILES[path],
      }
    })
  if (fixtures.length === 0) {
    throw new Error(
      "youtubeFixtures: no fixture found under admin/test/fixtures/youtube/."
    )
  }
  return fixtures
}

/**
 * One YouTube fixture's text.
 *
 * @param name - the file name without its extension.
 * @returns the text.
 * @throws {Error} when there is no such fixture.
 */
export function youtubeFixture(name: string): string {
  const fixture = youtubeFixtures().find((candidate) => candidate.name === name)
  if (fixture === undefined) {
    throw new Error(`youtubeFixture: no fixture "${name}".`)
  }
  return fixture.text
}
