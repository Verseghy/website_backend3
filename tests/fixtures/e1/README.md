# The editor's golden files

The cross-language golden files of editor.md §13.6 (P2 and P3). Each
`<name>.html` is `J(x)`: what the admin's editor stores after it loads the
fixture `x` and saves it (editor.md §6, `J`). Never edit them by hand.

- **Written by** the admin's tests: `admin/src/rich-text/e1-golden.test.ts`,
  part of `npm test`, brings this directory to what the editor stores now;
  `admin/src/test/golden-e1.ts` lists the fixtures and names the files.
- **Read by** `tests/rich_text/golden.rs`, which asserts for every file that
  the server's sanitiser keeps it byte for byte with an empty removal report
  (P2, `S(e1) == e1`), that it removes no element, attribute or style
  property from it (P3, naming each one it drops), and that the P0 oracle
  accepts it.
- **Checked for drift** by the `Contract` CI job, which runs
  `npm run test:e1` in `admin/` and then `scripts/ci/check-contract-drift.sh`:
  a PR whose editor stores other bytes than the committed files fails until
  the files the test writes are committed.

The files are named after their group and fixture:

| Prefix | Fixture |
|---|---|
| `regression-` | the expected output of a sanitiser regression fixture (`tests/fixtures/regressions/`), loaded and saved |
| `regression-input-` | the input of a regression fixture, loaded and saved, where that differs from the output's |
| `canonical-` | a canonical body of `admin/src/test/editor-fixtures.ts` |
| `table-` | a canonical table shape of that module |
| `table-input-` | table HTML that the editor rewrites on its first load |
| `media-` | a canonical image, YouTube or Padlet body of `admin/src/test/editor-fixtures.ts` |
| `media-input-` | image and embed HTML that the editor rewrites on its first load |
| `embed-` | a YouTube input fixture of `admin/test/fixtures/youtube/` (ADR 0043): embed code loaded and saved, or a link pasted on its own into an empty body |
| `paste-` | a paste fixture of `admin/test/fixtures/paste/` (`paste-generic-`) or `admin/test/fixtures/third_party/roosterjs/` (`paste-roosterjs-`) pasted as HTML into an empty body, through the paste filter (editor.md §5, §13.5) |
| `command-` | a document built with the editor's commands, the ones the toolbar's buttons call, the media commands included |

All of it is invented markup or, for `embed-` and `paste-roosterjs-`, what the editor stores for
YouTube's own code and links and for roosterjs' web samples (`THIRD_PARTY.md`), never production content
(`CLAUDE.md`, public-safety rules). A file holds the stored HTML as is, with no trailing
newline.

After a change to the editor (`admin/src/rich-text/`), to its fixtures or to
the sanitiser's regression fixtures (`tests/fixtures/regressions/`), run
`npm test` or `npm run test:e1` in `admin/` and commit what changed here. When
`tests/rich_text/golden.rs` then fails, the editor writes something the
server does not store: fix the editor, or, when content-v1 lacks something
the editor needs, change the policy (editor.md §3).
