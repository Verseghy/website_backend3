# Paste fixtures of the generic path

Small clipboard documents shaped like what the sources outside v1's paste
targets write (editor.md §5.5, §13.5): Google Docs, Google Sheets,
LibreOffice Writer and Calc, Safari's sanitised Word HTML, and a selection
of a web page. They are inputs of the paste filter's degrade-safely suite
(`admin/src/rich-text/paste/degrade-safely.test.ts`), which pastes each one
through the whole paste path in Chromium, Firefox and WebKit and asserts
only invariants, and of the editor's golden files
`tests/fixtures/e1/paste-generic-*.html` (§13.6), so the server checks what
the editor stores for them.

They are synthetic: written by the project after the shapes that the
research describes (research: editor design, clipboard §2.3, §5; the
`docs-internal-guid` wrapper, `google-sheets-html-origin`, LibreOffice's
generator `<meta>` and `<colgroup width>`, Safari's computed inline styles,
its `WebKit-mso-list-quirks-style` block and `blob:` images), never captured
from a real document. Their text is invented, their links point to
`example.org`, and their picture addresses are invented
(`invented-image-0001`, `file:///invented/...`, `blob:https://admin.example.invalid/...`).
They are under the repository licence. Each file is the clipboard's
`text/html` exactly, with no trailing newline.

| File | Shape |
|---|---|
| `google-docs.html` | the `<b style="font-weight:normal" id="docs-internal-guid-…">` wrapper; runs as `span`s with every property spelled out; a two-level list; a table in a `div` with a `colgroup`; a link; a picture from Google's image host |
| `google-sheets.html` | `google-sheets-html-origin` around a `<style>` and a table with `col` widths, cell styles, `data-sheets-value` attributes and a merged cell with a line break |
| `libreoffice-writer.html` | a whole document with the generator `<meta>`, class rules, `p[align]`, `font[color]`, `ol[type]`, `col width="128*"`, a `file:` picture and an `h6` |
| `libreoffice-calc.html` | `colgroup width` without `col`s, cell hints (`align`, `valign`, `bgcolor`, `height`) and `sdval` attributes |
| `safari-word.html` | Word desktop HTML as Safari passes it on: computed inline styles, the list quirk `<style>`, a Word table and a `blob:` picture; it carries Word's namespace, so step 1 detects Word |
| `web-page.html` | a web page selection: `section`, `h5`, `mark`, `code`, refused and kept links, `picture` and `figure`, a YouTube iframe with `si` and `controls=0` (invented `si` value), a table with `caption`, `thead` and `tfoot` and a Padlet board in a cell, an image of the site in a list item, `script`, `svg`, a foreign iframe, `dl` and `pre` |
