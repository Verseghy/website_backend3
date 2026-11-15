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
