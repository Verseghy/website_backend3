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

| Module               | What it is                                                                                                                                                 |
| -------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `policy.ts`          | `policy.json` read, validated and typed (`CONTENT_V1_POLICY`); a manifest the admin does not understand fails at load with the member and the fix          |
| `site-extensions.ts` | `siteExtensions({ policy, media })`, the extension list of every editor                                                                                    |
| `extensions/`        | the site versions of the document, headings, lists, link, alignment, sub and superscript and text style, one module each, with why each differs from stock |
| `style-decls.ts`     | the raw style reader of the parse rules: content-v1's own spelling only, resolved in source order, checked against the policy's grammars                   |
| `link-href.ts`       | the server's `a[href]` rule, for the link mark                                                                                                             |
| `load-document.ts`   | `loadDocument(editor, html)`, the one way a body enters an editor                                                                                          |

The schema takes the policy as an argument (editor.md §1.2 D6); pass
`CONTENT_V1_POLICY`. Its tests compare what it renders with `policy.json`
(`site-extensions.policy.test.ts`), so a policy change that the schema does
not follow fails there. `pins.test.ts` holds the package pins of editor.md
§4.1: every `@tiptap/*` package at one exact version, and prosemirror-view,
prosemirror-model and prosemirror-tables through `overrides` in
`package.json`, each installed once.
