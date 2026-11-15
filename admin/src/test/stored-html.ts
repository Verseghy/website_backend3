/**
 * Test support for the stored-HTML serialiser (`src/rich-text/serialize/`).
 *
 * - {@link storedHtmlHarness} is editor.md's `J` (§6, §13.4): a headless
 *   TipTap editor with the site schema under jsdom, which loads a body with
 *   `loadDocument` and saves it with `toStoredHtml`.
 * - {@link regressionFixtures} reads the server's sanitiser regression
 *   fixtures (`tests/fixtures/regressions/`, editor.md §13.6): every input
 *   and the sanitiser's output for it, so the admin's L2 tests run over the
 *   same bodies as the server's (§13.4, "Over every committed fixture").
 * - {@link reparse} parses HTML the way the server's sanitiser does (a
 *   fragment in a `div`) and copies it into the stored tree.
 */
import { Editor } from "@tiptap/core"
import type { Node as ProseMirrorNode } from "@tiptap/pm/model"

import { loadDocument } from "@/rich-text/load-document"
import { CONTENT_V1_POLICY } from "@/rich-text/policy"
import type { StoredNode } from "@/rich-text/serialize/stored-tree"
import { treeFromDom } from "@/rich-text/serialize/stored-tree"
import { toStoredHtml } from "@/rich-text/serialize/to-stored-html"
import { siteExtensions } from "@/rich-text/site-extensions"

/** A headless editor that loads and saves bodies. */
export interface StoredHtmlHarness {
  /** The editor, for tests that edit the loaded document (a paste). */
  readonly editor: Editor
  /** Loads a body into the editor and returns its document. */
  load(html: string): ProseMirrorNode
  /** The stored HTML of a document. */
  save(doc: ProseMirrorNode): string
  /** `J(html)`: load, then save. */
  J(html: string): string
  /** Destroys the editor. */
  destroy(): void
}

/**
 * A headless editor with the site schema, for `J`.
 *
 * @returns the harness; call `destroy` when the test is done.
 */
export function storedHtmlHarness(): StoredHtmlHarness {
  const editor = new Editor({
    element: document.createElement("div"),
    extensions: siteExtensions({ policy: CONTENT_V1_POLICY }),
  })
  const save = (doc: ProseMirrorNode) =>
    toStoredHtml(doc, { policy: CONTENT_V1_POLICY, document })
  const load = (html: string) => {
    loadDocument(editor, html)
    return editor.state.doc
  }
  return {
    editor,
    load,
    save,
    J: (html) => save(load(html)),
    destroy: () => editor.destroy(),
  }
}

/** One sanitiser regression fixture. */
export interface RegressionFixture {
  /** The fixture's name, the file name without `.html`. */
  readonly name: string
  /** The input, as bytes read as UTF-8. */
  readonly input: string
  /** The sanitiser's output for it. */
  readonly output: string
}

/**
 * The server's regression fixtures, inputs and outputs, by path. Vite reads
 * them at transform time, so this works in every test environment (jsdom
 * has no `file:` module URL for `node:fs`).
 */
const REGRESSION_FILES = import.meta.glob<string>(
  "../../../tests/fixtures/regressions/*.html",
  { query: "?raw", import: "default", eager: true }
)

/** Every regression fixture, in file-name order. */
export function regressionFixtures(): RegressionFixture[] {
  const fixtures = Object.keys(REGRESSION_FILES)
    .filter((path) => path.endsWith(".out.html"))
    .sort()
    .map((path) => {
      const base = path.slice(0, -".out.html".length)
      return {
        name: base.slice(base.lastIndexOf("/") + 1),
        input: REGRESSION_FILES[`${base}.html`],
        output: REGRESSION_FILES[path],
      }
    })
  if (fixtures.length === 0) {
    throw new Error(
      "regressionFixtures: no fixture found under tests/fixtures/regressions/; the admin tests read the server's fixtures from there."
    )
  }
  return fixtures
}

/**
 * Parses an HTML fragment in a `div`, as the server's sanitiser does
 * (ammonia's fragment parse), into the stored tree.
 *
 * @param html - the fragment.
 */
export function reparse(html: string): StoredNode[] {
  const container = document.createElement("div")
  container.innerHTML = html
  return treeFromDom(container.childNodes)
}
