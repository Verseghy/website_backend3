# Sanitiser regression fixtures

Permanent inputs for the content-v1 sanitiser (editor.md §13.6, P1). The
runner is `tests/rich_text/regressions.rs`.

Each fixture is a pair:

- `<name>.html`: the input, read as bytes (a trailing newline is part of it);
- `<name>.out.html`: what `Sanitizer::sanitize` must write for it. An empty
  file means that nothing survives.

For every pair the runner checks that the output equals `<name>.out.html`,
that sanitising `<name>.out.html` again returns it unchanged with an empty
removal report (`S(S(x)) == S(x)`), and that independent sanitiser instances
write the same bytes and the same report (P5).

## Adding a fixture

- Every shrunk failure of the P1 or P5 proptests
  (`tests/rich_text/properties.rs`) becomes a fixture, named after what it
  exercises, in lower-case words joined by `-`.
- Write the input by hand. Fixtures are invented markup, never copied from
  production content (`CLAUDE.md`, public-safety rules).
- Write the expected output only after checking it against editor.md §3 and
  §7.5, not by copying whatever the sanitiser printed.
- The admin's editor loads every pair too: `tests/fixtures/e1/` holds a golden
  file of each output and of each input that the editor stores differently
  (editor.md §13.6, P2 and P3). After adding or changing a pair, run
  `npm run test:e1` in `admin/` (Node.js as `admin/.node-version` says) and
  commit what it wrote under `tests/fixtures/e1/`; the `Contract` CI job
  fails until then, and `tests/rich_text/golden.rs` checks the new files.

## When an expected output changes

A change that alters an output updates the `.out.html` file in the same PR
and says why in the PR body. The CSS grammar did so when it landed: it
keeps the `style` values the sanitiser dropped before, and gives every
`table` and `tbody` its locked style (editor.md §3.3, §7.3), so the
expected outputs with a table gained those attributes. `canonical-shapes`
became every editor.md §3.7 shape with its styles: the shapes it held got
their `style` values back, and the shapes it lacked (the styled `span` and
`ul`, the blockquote, and the YouTube and Padlet embeds) were added.

The canonical style writer and the structural pass's two style rules did so
too (editor.md §3.1, §7.5). Every `img` that had no `style` now carries its
four locked declarations, so `image-src-rejected` and `upper-case-markup`
gained them:
`style="display: block; height: auto; margin: 0 auto; max-width: 100%"`.
Nothing else changed: every style in the other outputs was already sorted
and written in full.

The editor's tables changed a fixture, not an output: the table of
`canonical-shapes` gained a third row, in the input and the expected output
alike. Its `rowspan="2"` ran past the last row, which the server keeps but
the editor does not: prosemirror-tables shortens such a rowspan when a body
loads, so the editor would have stored the shape with another `rowspan`
(editor.md §13.4, `toStoredHtml(load(z)) == z`). A canonical table is
rectangular.

The YouTube rules of ADR 0043 changed fixtures and added one (items D129,
D132 and D133). A video id now has exactly 11 characters, so the fixtures
that used `VIDEOID` or `x` as a YouTube id use 11-character ids; each case
of `embed-queries-and-markers`, `embed-look-alike-hosts` and
`iframe-text-removed` still fails or passes for the reason it was written
for, not for its id. `canonical-shapes` stores its YouTube embed with the
§3.7 style, which gained `min-height: 200px`. The new
`youtube-id-rules-and-minimum-height` pins the rules themselves: ids of the
wrong length, `videoseries` without `list` and the `listType` form are
refused, `videoseries` with `list` is kept, and `min-height` is kept only
at its locked `200px`.
