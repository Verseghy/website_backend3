# roosterjs paste samples

Clipboard HTML from Microsoft 365 on the web, copied from the end-to-end
paste tests of [roosterjs](https://github.com/microsoft/roosterjs), the
editor of Outlook on the web, at commit
`fc2d5c290d57001d73215d71109993522a2a8f64` (2026-10-23). They are inputs of
the paste filter's degrade-safely suite
(`admin/src/rich-text/paste/degrade-safely.test.ts`, editor.md §13.5) and of
the editor's golden files `tests/fixtures/e1/paste-roosterjs-*.html`
(§13.6): Excel and Word for the web are
not v1 paste targets (research: DECISIONS 15e), so the suite asserts only
that the filter neither throws nor falls back, that its output is content-v1,
and that the editor stores it as a fixed point. No fidelity snapshot of their
output is kept (editor.md §13.1): their golden files pin what the editor
stores only so that the server checks it (P2, P3), and they change with the
filter.

| File | roosterjs file (`packages/roosterjs-content-model-plugins/test/paste/e2e/`) | What it is |
|---|---|---|
| `roosterjs-excel-online-e2e.html` | `cmPasteFromExcelOnlineTest.ts`, the `rawHtml` of the shared `clipboardData` | Excel for the web: a whole Excel document with its `<style>` in a `div` in the body, and `<!--StartFragment-->` inside the table |
| `roosterjs-excel-online-text-colour.html` | the same file, the `rawHtml` of "E2E Table with table cells with text color" | Excel for the web: class rules with borders, fills and a coloured link |
| `roosterjs-word-online-e2e.html` | `cmPasteFromWacTest.ts`, the `rawHtml` of "E2E" | Word for the web: computed inline styles on every element, per-item list containers |
| `roosterjs-word-online-table.html` | the same file, the `rawHtml` of "Content from Word Online with table" | Word for the web: a table with its hover-handle `div`s |

## Licence

MIT, Copyright (c) Microsoft Corporation. `LICENSE` in this directory is a
byte copy of roosterjs' `LICENSE` at that commit. `THIRD_PARTY.md` at the
repository root lists the files (ADR 0012 §4, editor.md §13.1).

## How they were made

Each file is the value of one JavaScript string literal of the test file,
with the string's escapes decoded (`\r\n`, `\t`, `\"`, `\\`) and nothing
else changed: no trailing newline, the attribute values (Microsoft's
`ccp_infra_*` copy identifiers among them) and the text ("Test", "asd",
"Test Table") as upstream has them. The text holds no personal data.
