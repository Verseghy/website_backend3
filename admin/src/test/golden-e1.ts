/**
 * The cross-language golden files of editor.md §13.6 (P2 and P3):
 * `tests/fixtures/e1/<name>.html`, each `J(x)` for one editor fixture `x`.
 *
 * The admin writes them (`src/rich-text/e1-golden.test.ts`, part of
 * `npm test`, and `npm run test:e1` in the Contract CI job); the server
 * reads them (`tests/rich_text/golden.rs`) and asserts that its sanitiser
 * keeps each byte for byte (P2, `S(e1) == e1`) and removes no element,
 * attribute or style property from it (P3). The Contract job fails when the
 * files the admin writes differ from the committed ones, so an editor change
 * that changes what the editor stores also changes the files the server
 * checks, in the same PR (ADR 0005 §4's drift rule, applied to the editor's
 * output).
 *
 * This module is the pure part: which files exist and what they hold
 * ({@link editorGoldenFiles}), and what a directory needs to match them
 * ({@link goldenChanges}). The test that writes them does the I/O.
 *
 * The fixtures are:
 *
 * - `regression-<name>`: `J` of the sanitiser's expected output for each
 *   regression fixture of `tests/fixtures/regressions/` (a stored body,
 *   loaded and saved), and `regression-input-<name>`: `J` of its input,
 *   when that differs;
 * - `canonical-<name>`, `table-<name>`, `table-input-<name>`,
 *   `media-<name>`, `media-input-<name>`: `J` of the bodies of
 *   `editor-fixtures.ts`;
 * - `embed-<name>`: what the editor stores for a YouTube input fixture of
 *   `admin/test/fixtures/youtube/` (ADR 0043): `J` of embed code, and the
 *   lone-URL paste rule's embed for a link, pasted into an empty body;
 * - `command-<name>`: the stored HTML of a document built with the editor's
 *   commands (the media commands included), which is `J` of itself (the
 *   tests check that).
 */
import type { StoredHtmlHarness } from "./stored-html"
import { regressionFixtures } from "./stored-html"
import {
  CANONICAL_BODIES,
  COMMAND_FIXTURES,
  MEDIA_BODIES,
  MEDIA_COMMAND_FIXTURES,
  MEDIA_INPUTS,
  TABLE_INPUTS,
  TABLE_SHAPES,
} from "./editor-fixtures"
import { pasteEvent } from "./site-schema"
import { youtubeFixtures } from "./youtube-fixtures"

/** The repository path of the golden files, from the repository root. */
export const GOLDEN_DIRECTORY = "tests/fixtures/e1"

/** A golden file's name without `.html`: lower-case words joined by `-`. */
export const GOLDEN_NAME = /^[a-z0-9]+(?:-[a-z0-9]+)*$/

/** The body the command fixtures start from. */
export const COMMAND_START = "<p>Táblázat:</p>"

/** One golden file to write. */
export interface GoldenSource {
  /** The file name without `.html` ({@link GOLDEN_NAME}). */
  readonly name: string
  /** The stored HTML, written as is: no trailing newline is added. */
  readonly html: string
}

/**
 * The golden files by file name (`<name>.html`), in name order.
 *
 * @param sources - the files to write.
 * @returns the contents by file name.
 * @throws {Error} when a name is not lower-case words joined by `-`, or two
 *   sources share one, so a fixture can never silently replace another.
 */
export function goldenFiles(
  sources: Iterable<GoldenSource>
): Map<string, string> {
  const files = new Map<string, string>()
  for (const { name, html } of sources) {
    if (!GOLDEN_NAME.test(name)) {
      throw new Error(
        `goldenFiles: "${name}" is not a golden file name; use lower-case words and digits joined by "-".`
      )
    }
    const file = `${name}.html`
    if (files.has(file)) {
      throw new Error(
        `goldenFiles: two fixtures are named "${name}"; give one of them another name in editor-fixtures.ts.`
      )
    }
    files.set(file, html)
  }
  return new Map([...files].sort(([a], [b]) => (a < b ? -1 : a > b ? 1 : 0)))
}

/**
 * Every golden file of the editor: the fixtures this module's
 * documentation lists, each saved with the harness.
 *
 * @param harness - a `J` harness (`storedHtmlHarness()`); its editor is
 *   loaded with each body in turn.
 * @returns the contents by file name ({@link goldenFiles}).
 */
export function editorGoldenFiles(
  harness: StoredHtmlHarness
): Map<string, string> {
  const { J, editor, load, save } = harness
  const sources: GoldenSource[] = []
  for (const { name, input, output } of regressionFixtures()) {
    const stored = J(output)
    sources.push({ name: `regression-${name}`, html: stored })
    const read = J(input)
    if (read !== stored) {
      sources.push({ name: `regression-input-${name}`, html: read })
    }
  }
  const groups: [string, Readonly<Record<string, string>>][] = [
    ["canonical", CANONICAL_BODIES],
    ["table", TABLE_SHAPES],
    ["table-input", TABLE_INPUTS],
    ["media", MEDIA_BODIES],
    ["media-input", MEDIA_INPUTS],
  ]
  for (const [group, bodies] of groups) {
    for (const [name, body] of Object.entries(bodies)) {
      sources.push({ name: `${group}-${name}`, html: J(body) })
    }
  }
  for (const [name, script] of Object.entries({
    ...COMMAND_FIXTURES,
    ...MEDIA_COMMAND_FIXTURES,
  })) {
    load(COMMAND_START)
    script(editor)
    sources.push({ name: `command-${name}`, html: save(editor.state.doc) })
  }
  for (const { name, kind, text } of youtubeFixtures()) {
    if (kind === "code") {
      sources.push({ name: `embed-${name}`, html: J(text) })
    } else {
      // The lone-URL paste rule, as a browser pastes a copied link.
      load("<p></p>")
      editor.commands.setTextSelection(1)
      editor.view.dom.dispatchEvent(pasteEvent({ "text/plain": text }))
      sources.push({ name: `embed-${name}`, html: save(editor.state.doc) })
    }
  }
  return goldenFiles(sources)
}

/** What a directory needs so that its golden files match a plan. */
export interface GoldenChanges {
  /** The files to write: missing, or with other contents. */
  readonly write: readonly string[]
  /** The `.html` files the plan no longer has. */
  readonly remove: readonly string[]
}

/**
 * The changes that make a directory's golden files equal to a plan. Only
 * `.html` files count: the directory's `README.md` is never removed.
 *
 * @param existing - the directory's files by name, with their contents.
 * @param planned - the golden files by name ({@link goldenFiles}).
 * @returns the files to write and to remove, each in name order.
 */
export function goldenChanges(
  existing: ReadonlyMap<string, string>,
  planned: ReadonlyMap<string, string>
): GoldenChanges {
  const write = [...planned]
    .filter(([file, html]) => existing.get(file) !== html)
    .map(([file]) => file)
  const remove = [...existing.keys()]
    .filter((file) => file.endsWith(".html") && !planned.has(file))
    .sort()
  return { write, remove }
}
