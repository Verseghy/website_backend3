/**
 * Test support for the paste filter's fixtures (editor.md §13.5): the
 * synthetic shapes of `admin/test/fixtures/paste/` and the roosterjs web
 * samples of `admin/test/fixtures/third_party/roosterjs/`, whose READMEs say
 * where each comes from, and {@link clipboardPasteEvent}, which pastes them
 * the way a browser does.
 *
 * Vite reads the files at transform time, so this works under jsdom and in
 * Vitest's browser mode alike.
 */
import { pasteEvent } from "./site-schema"

/** The synthetic generic-path fixtures, by path. */
const GENERIC = import.meta.glob<string>("../../test/fixtures/paste/*.html", {
  query: "?raw",
  import: "default",
  eager: true,
})

/** The roosterjs samples, by path. */
const ROOSTERJS = import.meta.glob<string>(
  "../../test/fixtures/third_party/roosterjs/*.html",
  { query: "?raw", import: "default", eager: true }
)

/** One paste fixture. */
export interface PasteFixture {
  /**
   * The fixture's name: its group and its file name without `.html`, such
   * as `generic-google-docs` or `roosterjs-word-online-table`.
   */
  readonly name: string
  /** `generic` for the synthetic shapes, `roosterjs` for the web samples. */
  readonly group: "generic" | "roosterjs"
  /** The clipboard's `text/html`, exactly as committed. */
  readonly html: string
}

/** The fixtures of one glob, named after their group and file. */
function fixturesOf(
  files: Record<string, string>,
  group: PasteFixture["group"]
): PasteFixture[] {
  return Object.keys(files)
    .sort()
    .map((path) => {
      const file = path.slice(path.lastIndexOf("/") + 1, -".html".length)
      // The roosterjs files already start with their group's name.
      const name = file.startsWith(`${group}-`) ? file : `${group}-${file}`
      return { name, group, html: files[path] }
    })
}

/**
 * Every paste fixture, generic shapes first, each group in file-name order.
 *
 * @throws {Error} when a group has no file, so a moved directory fails the
 *   tests instead of emptying them.
 */
export function pasteFixtures(): PasteFixture[] {
  const generic = fixturesOf(GENERIC, "generic")
  const roosterjs = fixturesOf(ROOSTERJS, "roosterjs")
  if (generic.length === 0 || roosterjs.length === 0) {
    throw new Error(
      "pasteFixtures: no fixture found under admin/test/fixtures/paste/ or admin/test/fixtures/third_party/roosterjs/."
    )
  }
  return [...generic, ...roosterjs]
}

/**
 * A `paste` event carrying clipboard data. In a browser that can build a
 * real `ClipboardEvent` with a `DataTransfer` (editor.md §13.5) that is what
 * it returns; otherwise, as under jsdom, the stand-in of `site-schema.ts`.
 *
 * @param data - the clipboard formats, by MIME type.
 */
export function clipboardPasteEvent(
  data: Readonly<Record<string, string>>
): ClipboardEvent {
  try {
    if (
      typeof DataTransfer === "function" &&
      typeof ClipboardEvent === "function"
    ) {
      const transfer = new DataTransfer()
      for (const [type, value] of Object.entries(data)) {
        transfer.setData(type, value)
      }
      const event = new ClipboardEvent("paste", { clipboardData: transfer })
      if (
        Object.entries(data).every(
          ([type, value]) => event.clipboardData?.getData(type) === value
        )
      ) {
        return event
      }
    }
  } catch {
    // An engine that cannot build the event takes the stand-in.
  }
  return pasteEvent(data)
}
