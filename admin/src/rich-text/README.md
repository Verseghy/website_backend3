# admin/src/rich-text

The admin's side of the rich-text editor (ADR 0013, `docs/design/editor.md`).

Two files here are generated, never edited by hand:

- `policy.json`: content-v1, the one HTML dialect backend3 stores, as data:
  the tags, each tag's attributes in canonical order with their value rules,
  the CSS grammar per element with the border constants, the URL rules, the
  embed providers, the elements removed with their content and the forbidden
  tags. The paste filter, `canonicalStyle` and the custom TipTap
  attributes read it. `docs/design/editor.md` §3.9 describes every member.
- `css-vectors.jsonl`: one `{element, input, output}` object per line, the
  cases that `canonicalStyle` must write exactly as the server does.

Both come from the Rust policy in `src/domain/rich_text/`. After a change
there, run this from the repository root and commit both files:

```sh
cargo loco task rich_text_manifest
```

The `Contract` CI job runs the task and fails when either file differs from
its output, or when the tag list holds a forbidden tag (editor.md §13.6).

## The site schema

The TipTap 3.32 schema of content-v1 (editor.md §4.2, §4.3):

| Module               | What it is                                                                                                                                                         |
| -------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| `policy.ts`          | `policy.json` read, validated and typed (`CONTENT_V1_POLICY`); a manifest the admin does not understand fails at load with the member and the fix                  |
| `site-extensions.ts` | `siteExtensions({ policy, media })`, the extension list of every editor                                                                                            |
| `extensions/`        | the site versions of the document, headings, lists, link, alignment, sub and superscript, text style and tables, one module each, with why each differs from stock |
| `style-decls.ts`     | the raw style reader of the parse rules: every `style` through `canonicalStyle`, so a rule reads what the server stores; the checks of the one spelling            |
| `link-href.ts`       | the server's `a[href]` rule, for the link mark                                                                                                                     |
| `load-document.ts`   | `loadDocument(editor, html)`, the one way a body enters an editor                                                                                                  |

The schema takes the policy as an argument (editor.md §1.2 D6); pass
`CONTENT_V1_POLICY`. Its tests compare what it renders with `policy.json`
(`site-extensions.policy.test.ts`), so a policy change that the schema does
not follow fails there. `pins.test.ts` holds the package pins of editor.md
§4.1: every `@tiptap/*` package at one exact version, and prosemirror-view,
prosemirror-model and prosemirror-tables through `overrides` in
`package.json`, each installed once.

## Canonical styles

`css/` is `canonicalStyle(policy, tag, style)`, the admin's half of
content-v1's one spelling of every style (editor.md §3.1). The server's
grammar in `src/domain/rich_text/css/` is the other half, and both write the
output of every line of `css-vectors.jsonl` (`css/canonical-style.test.ts`).

| Module                   | What it is                                                                                                   |
| ------------------------ | ------------------------------------------------------------------------------------------------------------ |
| `css/declarations.ts`    | the declaration reader: cssparser 0.38's tokenizer and recovery rules, as `declarations.rs` uses them        |
| `css/values.ts`          | lengths in px, `em`, percentages, numbers in hundredths, colours (hex, named, `rgb()`), each in one spelling |
| `css/named-colours.ts`   | the 148 named colours of CSS Color 4                                                                         |
| `css/canonical-style.ts` | the grammar from the policy, the cascade of §3.1 rule 1, the writer of rules 2 and 5, the alphabet guard     |

## The stored-HTML serialiser

`serialize/` is `toStoredHtml(doc, { policy, document })`, the only way a
document becomes the HTML that is saved (editor.md §6). Never use
`editor.getHTML()`, which returns the engine's CSSOM spelling. The editor
input calls it as
`toStoredHtml(editor.state.doc, { policy: CONTENT_V1_POLICY, document: editor.view.dom.ownerDocument })`:
nothing is created in that document, only in the inert one `inertDocument`
makes from it, so a save never fetches an image.

| Module                        | What it is                                                                                                                                                              |
| ----------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `serialize/to-stored-html.ts` | the three steps; `verbatimDocument`, which keeps the CSSOM out of prosemirror-model's rendering; `inertDocument`, the document without a browsing context it renders in |
| `serialize/whitespace.ts`     | the whitespace rule of §3.8, applied to every textblock of the document, after U+0000 is dropped                                                                        |
| `serialize/stored-tree.ts`    | the plain tree that the rendered DOM is copied into before the post-pass                                                                                                |
| `serialize/post-pass.ts`      | canonical styles, bare spans and equal neighbours, tightening (with editor.md §3.7's empty-paragraph exception, item D148), attribute order                             |
| `serialize/write-html.ts`     | the writer in html5ever 0.40.1's dialect, writing each value as an HTML parse reads it back, so the server writes the same bytes again                                  |

Its tests are editor.md §13.4's L2: `toStoredHtml(load(z)) == z` for the
canonical shapes, `J(J(x)) == J(x)` over the server's regression fixtures
(`tests/fixtures/regressions/`, which `vitest.config.ts` lets Vite read),
fast-check documents with every kind of whitespace and U+0000, and the guard that the
output never holds `rgb(` or `border-width:` and that prosemirror-model's
`style` branch is unchanged. `src/test/stored-html.ts` has the `J` harness.
The tables have their own (`serialize/tables.test.ts`,
`tables.property.test.ts`): every canonical table is a fixed point, every
table input reaches its pinned form in one load, a span far wider than its
table loads within a size and time budget, and random tables and table HTML
are stored as fixed points with canonical cell styles and without padding
beyond their real grid.

## Tables

`extensions/site-table.ts` holds the table nodes (editor.md §4.3 rows 5-7):
the public table shape with the forced `table` and `tbody` styles and no
table view or column widths, the cell declarations as cell attributes named
after the policy's properties, read by `readCellDecls` (the `style` over the
`align`, `valign`, `bgcolor`, `width` and `height` hints, each hint left out
when the `style` declares its property) and written as one sorted style
string by `cellStyle`, and `fixTablesFully`, a plugin before
prosemirror-tables' own: in a table that `fixTables` would pad, it first
narrows every span that crosses a column in which no cell starts
(`unanchoredSpans`), so a `colspan="1000"` adds no thousand cells to each
short row, and it repeats `fixTables` until every table is rectangular. In a
cell, the alignment commands of `extensions/site-text-align.ts` align the
innermost cell that holds the selection.

## The golden files (P2 and P3)

`e1-golden.test.ts`, part of `npm test`, writes `J(x)` for every editor
fixture to `tests/fixtures/e1/` at the repository root (editor.md §13.6):
the server's regression fixtures, the canonical bodies and tables, the table
inputs and the command fixtures of `src/test/editor-fixtures.ts`, named as
`src/test/golden-e1.ts` says. The server's `tests/rich_text/golden.rs`
asserts that the sanitiser stores each file unchanged and removes nothing
from it. The `Contract` CI job runs `npm run test:e1` and fails when a file
differs from the committed one, so after a change here run `npm test` and
commit what it wrote there. A slice that adds an editor feature or a paste
fixture adds it to `editor-fixtures.ts` (or a group of its own in
`golden-e1.ts`), so the server checks what it writes.
