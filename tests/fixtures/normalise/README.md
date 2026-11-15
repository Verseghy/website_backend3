# Normaliser fixtures

Before-and-after fixtures for the import normaliser (data-migration.md §7.1,
§7.4, §12; `src/domain/normalise/`). The runner is
`tests/normalise/fixtures.rs`.

Each fixture is a pair:

- `<name>.html`: a legacy body, read as bytes (a trailing newline is part of
  it);
- `<name>.out.html`: what `normalise` must write for it.

The name starts with the step it shows: `n0-` (the parse: character
references, tag case, line ends), `n3-` (`<center>`), `n5-` (legacy
presentational markup: the block rules (`p[align]`, `table[align]`, `h5` and
`h6`, the italic `h2`, `p[lang]`) and the inline marks), `n6-` (the dead
host's links), `n8-` (the serialisation: only five character references
left, double-quoted attributes, void tags without a slash), `n9-` (a
content-v1 body, which comes out unchanged), or `alt-` (images without alt
text, which are reported, never changed).

For every pair the runner checks that the output equals `<name>.out.html`,
that normalising `<name>.out.html` again returns it with no rule hit (P4),
that the step of the name fired, that the text a reader reads is the same in
both files (V5(c), compared as search text), and that `<name>.out.html` is
the sanitiser's fixed point too (N9). An output that still holds markup for a
later step is listed in the runner's `NOT_YET_CONTENT_V1` with that step;
for those the sanitiser check must fail.

The `.out.html` files are also the bodies that the admin's editor round trip
(ADR 0013 §10, editor.md §13.4) loads and saves, so each one is written in
the editor's stored form wherever the steps in place allow it. A content-v1
output that the editor would still change is listed in the runner's
`NOT_YET_EDITOR_FORM` with the step that will settle it, and the runner
fails on an unlisted output with a line feed, tab or form feed in its text,
which the editor's whitespace rule (editor.md §3.8) never leaves.

Some inputs hold `\r\n` line ends on purpose (`n0-upper-case-markup`,
`n3-center-around-embed`, listed in the runner's `CRLF_INPUTS`).
`.gitattributes` turns off line-end conversion for the inputs here, and the
runner fails if one of those loses its `\r\n`.

## Adding a fixture

- Every rule a later slice adds (N1, N2, N4, N5b, N7) gets at least one
  pair here, with its step as the prefix. The runner pins the exact hits of
  each `n5-` pair of the inline marks, fails when another `n5-` pair hits an
  inline-mark rule without being listed there, and fails when an inline-mark
  rule has no pair.
- Write the input by hand in the legacy form: CKEditor 4's named character
  references, upper-case tags typed in source mode, `\r\n` line ends.
  Fixtures are invented markup, never copied from production content
  (`CLAUDE.md`, public-safety rules).
- Write the expected output from data-migration.md §7.1 and §7.4 and
  editor.md §3 and §9, not by copying whatever the normaliser printed.

## When an expected output changes

Later steps change these outputs, and the admin round trip must expect
them to:

- N2 turns `n3-center-around-embed`'s legacy player into the editor's
  YouTube node (`NOT_YET_CONTENT_V1`);
- N5b gives the two tables of `n5-table-align` the content-v1 table shape
  (no table width, the locked `border-collapse`, `max-width` and `tbody`
  declarations), and N7 drops their `border`, `cellpadding` and
  `cellspacing` (`NOT_YET_CONTENT_V1`);
- N5b gives the table of `n5-marks-in-a-table` its content-v1 shape, and N7
  drops the table's `border` and `cellpadding` (`NOT_YET_CONTENT_V1`);
- N7 drops the `text-align: inherit` that `n3-center-unclear-alignment`
  keeps after N3's `text-align: center` (`NOT_YET_CONTENT_V1`);
- N5b's whitespace rule removes the line feeds left between the blocks of
  `n3-center-around-embed`, and the one after the `<br>` inside the
  paragraph of `n0-upper-case-markup` (`NOT_YET_EDITOR_FORM`).

The PR that makes such a change updates the `.out.html` files in the same
PR, removes the fixtures it settles from `NOT_YET_CONTENT_V1` or
`NOT_YET_EDITOR_FORM`, and says why in its body.
