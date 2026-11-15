# data/import

Files that the legacy import, the `import_mysql` task, reads in the pod
([ADR 0017](../../docs/design/adr/0017-legacy-data-import-normalisation-and-defect-resolution.md),
[data-migration.md](../../docs/design/data-migration.md)). The image ships
the repository's `data/` directory as `/app/data/` (the `Containerfile`'s
runtime stage, [ADR 0045](../../docs/design/adr/0045-versioned-task-data-in-the-image-at-app-data.md)),
so this directory is `/app/data/import/` there.

## What may be committed here

Exactly two files, which `tests/image_data/` checks:

| File | What it is |
|---|---|
| `README.md` | this file |
| `resolutions.toml` | the defect resolution list of data-migration.md §8: the editors' decisions on the data defects that need one, each with its precondition, and the date of their sign-off. In the image it is `/app/data/import/resolutions.toml`, the default of the task's `resolutions` argument (research: DECISIONS R9, D12). |

`resolutions.toml` follows the schema of
`src/domain/import_mysql/resolutions.rs` (data-migration.md §8.2). The task
refuses a file that fails its validation, and CI validates the committed
one, so a list that does not parse never reaches the nightly rehearsal.

## What never goes into it

This repository is public (`CLAUDE.md`, "Public-safety rules"). The list
holds only ids, dates, enum choices and source values, or the SHA-256 of a
source value:

- no names of people, no e-mail addresses and no other per-person facts:
  who signed the list, and when, is recorded in the private runbook, and
  the file only gains the date of the sign-off (`signed_off`);
- no content: no titles, bodies, slugs, label names, dish names or other
  text from the legacy database. A body is pinned by its SHA-256 only
  (`N7-accept`);
- nothing else from a dump or a run report. Dumps and reports stay private
  (data-migration.md §11.3); the proposed entries are copied from a report
  as ids, dates and digests only.

A PR that changes `resolutions.toml` cites the runbook entry of the
sign-off by its date only.

## The empty list

Until the proposed entries are filled in from a staging dry-run report,
`resolutions.toml` is the valid empty list: `version = 1`, no entries and
no `signed_off`. The nightly rehearsal runs with it, so once the defect
detectors run, its `apply` stops at the first defect that the list does not
cover, as designed (ADR 0017 §3).
