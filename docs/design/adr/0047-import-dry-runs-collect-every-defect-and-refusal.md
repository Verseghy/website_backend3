# 0047. Import dry runs collect every defect and refusal

- Status: Accepted (2026-11-15)
- Decision questions: none: records the maintainer's answer of 2026-10-25 to the decision-register item D49 (research: DECISIONS R9), which settles what a dry run of the legacy import does with the defects and refusals that stop `apply`. It amends the second, third and fourth bullets of [ADR 0017](0017-legacy-data-import-normalisation-and-defect-resolution.md) §3.

## Context

[ADR 0017](0017-legacy-data-import-normalisation-and-defect-resolution.md) §3 lets the import resolve the defects of the legacy data from an editor-signed resolution list, and three of its bullets say what happens when the list and the data disagree, in every mode:

> - Every entry carries a precondition: the source value at sign-off. A changed row stops the import until the entry is re-signed.
> - Every defect is detected from its rule on every run. An instance the list does not cover stops the import, so the nightly rehearsal reports it long before the window.
> - The constraints of ADR 0009 are created by the schema migrations and therefore apply as the rows are inserted. A row that still violates one fails the import.

| Fact | Value | Source |
|---|---|---|
| Defect instances in the dump of 2026-10-23 that need an entry | 181 early post dates (D1), 3 posts without an author (D2), 1 post without an index image (D3), 11 sentinel dates (D5), 8 duplicate canteen dates (D6), 2 duplicate label groups (D8), the unpositioned menu items (D9), and the unplanned sanitiser removals that editors accept (N7) | [data-migration.md](../data-migration.md) §7.1, §8.1 |
| The committed resolution list | starts empty (`version = 1`), is filled from a staging dry-run report, and is signed by the editors before the window | data-migration.md §8.2; [cutover.md](../cutover.md) P2 |
| The nightly rehearsal | a dry run, then `apply` into a freshly recreated staging database, every night | data-migration.md §11.1; cutover.md §5.2 |
| What the editors see | only the sign-off sheet, an extract of a staging report: run reports stay private | data-migration.md §8.2, §11.3 |
| Defects whose entries work otherwise | D10: every page is imported as published, and editors *may* list pages to import as hidden, so an unlisted page needs no entry. D3: editors set post 70's index image in the current admin before the final dump, and the entry, signed after that edit, pins the image as its precondition, so once the edit is made the D3 rule no longer finds the post. D4 and D7: no section; their rows are imported unchanged. | data-migration.md §8.1, §8.2; ADR 0017 §3 |

With a stop on the first problem in every mode, the first nightly over the empty list stops at the first D1 post. Each later night shows one more problem, and only after the one before it has an entry, so neither the list nor the sign-off sheet can be complete from one report.

The register asked (research: decision register D49): "Do you accept the ADR amending ADR 0017 §3, data-migration §4.2 and §11.1? Dry runs would continue with proposed defaults, list every defect, refusal and constraint failure, and exit non-zero, while apply still stops at the first. Or do dry runs keep stop-on-first?" It recommended accepting, because "it serves ADR 0017 §3's own purpose, early and complete defect discovery, better. Apply semantics are unchanged, and the non-zero exit keeps the safety signal." The maintainer answered on 2026-10-25 (research: DECISIONS R9), with the recommended option:

- **D49**: "Accept". Its consequence: "One nightly report shows all defects, and signoff.md is complete early. The nightly stays red until the list covers them all."

## Decision

### 1. `apply` stops at the first problem, as before

In `mode:apply`, the first of these problems stops the run and rolls everything back, with the message of data-migration.md §4.2:

| Problem | The message names |
|---|---|
| a defect instance that no entry of the resolution list covers, where the defect needs one (every defect but D10, whose entries are optional, §4) | the defect id and the row id |
| an entry whose precondition does not hold: the row changed since the sign-off, no longer has its defect, or is gone; for D3, whose entry pins the fix that editors made in the source (§4), the post has another index image than the signed one, still has none, or is gone; an N7 entry whose body is unchanged but no longer has an unplanned removal is no problem: it is no longer needed, and the run lists it and goes on (§4) | the defect id, the row id, the expected and the found value |
| a row that violates a target `NOT NULL`, `CHECK`, unique or foreign-key rule | the table, the row id and the rule |

The cut-over, the dress rehearsals and the nightly's `apply` step never commit a resolution that the editors did not sign.

The N7 clause of the second row corrects this section under [ADR 0001](0001-record-architecture-decisions.md) §5 and §6, to the maintainer's answer of 2026-11-15 (research: DECISIONS R38 (D161)), which §4 quotes.

### 2. A dry run collects every problem and fails at its end

`mode:dry-run` meets the same problems and goes on:

| Problem | What the dry run does |
|---|---|
| an instance that no entry covers, where the defect needs one | applies the defect's proposed default, the proposed resolution of data-migration.md §8.1 where the defect's rule can compute it from the source; without one, it keeps the source value |
| an entry whose precondition does not hold, for an instance the rule finds | the same, for that instance |
| an entry for a row that the rule no longer finds and that does not hold | applies nothing |
| a row that a target rule refuses | reports it with its table, row id and rule, and goes on without it: the table steps retry a failing batch row by row inside savepoints to find each such row |

The later steps and the verification checks run over what the dry run wrote. At the end, a dry run that collected any problem fails: its error counts the problems and names each of them (the first 50, then a count of the rest), so the task exits non-zero, and the report lists every one. The transaction is rolled back, as in every dry run. Every other refusal of data-migration.md §4.2 (the target, the source schema, `snapshot_at`, the resolution file's validation, an inline image that would overwrite another file, a failed verification check) stops a dry run as it stops `apply`.

### 3. The report and the sign-off sheet

- The report's Defects part (data-migration.md §11.3) lists every instance of every defect whose detector ran, with its entry, the entry's precondition, the source value and the resolution the import proposes (the valid entry's, otherwise the defect's default, or none where the source value is kept), which a dry run applies, and every entry whose row the rule no longer finds. A stopped `apply` run rolled back, so the part never says that it applied anything. `summary.json` holds the same under `defects`, with `complete`, which says whether the run reached its end.
- Every run writes the editors' sign-off sheet, `signoff.md`, next to `report.md` and `summary.json`, readable by the task's user only like them: per defect, the row ids, the paths to open on the live site (`/posts/<id>`, `/information/<slug>`) and the proposed resolution of each instance (the entry's where a valid entry covers it, otherwise the default), and each row that the editors fixed in the source as its entry pins (D3). The maintainer takes it from the newest staging dry-run report (data-migration.md §8.2).
- Neither shows a person's name, and the Defects part shows no content. Their values come from types that hold only legacy ids, calendar days, instants, SHA-256 digests, index image paths of the legacy shape, a closed set of fixed words (`generic`, `published`, `hidden`, `accepted`), and those two kinds of public path. A detector that needs another column on the sheet adds it with its own test. The one column with content is the one that data-migration.md §8.1 and §8.2 already plan: D6's two menus side by side, the dish names of the two rows on a date, which the public site already shows and which hold no person's name; the D6 detector adds it to the sheet, never to the Defects part.

### 4. One detector for every defect

`DefectDetector` in `src/domain/import_mysql/defects.rs` applies sections 1 to 3 to every defect. Each table step brings the defect's rule, which detects every instance from the source rows on every run (never from the list), and the detector pairs the instances with the entries of the defect's section by what they are about (a post, a page, a canteen date, a label group or a menu level), compares every precondition, and returns, per instance, the entry's resolution, the default or the source value. The task's run ends by concluding the defect check, which fails a dry run that collected a problem. No step stops or goes on by rules of its own.

How a defect's entries work comes from data-migration.md §8.1, so it belongs to the defect (`DefectId::entry_policy`), never to the rule a step writes:

| Policy | Defects | An instance without an entry | An entry for a row the rule no longer finds |
|---|---|---|---|
| required | D1, D2, D5, D6, D8, D9 | a problem (§1, §2) | a problem: the row no longer has its defect, or is gone |
| acceptance | N7 | a problem (§1, §2) | no problem when the source has the signed value: the body is unchanged, but a later normaliser rule plans what the entry accepted, so the entry is no longer needed; the run lists it, applies nothing and goes on. Otherwise a problem: the body changed since the sign-off, or the row has no body |
| optional | D10 | no problem: it takes the default, `published`, in both modes, and is listed | a problem, as under required |
| source fix | D3 | a problem: the editors have not fixed it yet | holds, and applies nothing, when the source has the value the entry pins (the post has the signed index image); otherwise a problem: the fix differs, or the post is gone |
| listed | D4, D7 | no problem: imported unchanged and listed | none: no section |

An instance that an entry covers is judged the same under every policy: the entry's resolution when its precondition holds, otherwise a problem.

The acceptance row, and N7's place in it rather than under required, correct this table under [ADR 0001](0001-record-architecture-decisions.md) §5 and §6, to the maintainer's answer of 2026-11-15 (research: DECISIONS R38 (D161)). The table had put N7 under required: the decision register D49 that this ADR records said nothing about N7's entries, and this ADR had not foreseen point 5 of the review of `N7-accept` entries that normaliser-n7-plan built later ([data-migration.md](../data-migration.md) §7.4), by which an entry whose unchanged body no longer has an unplanned removal outlives its defect as no longer needed. The register asked (research: third decision register D161): "An `N7-accept` entry lets `apply` go on past a body's unplanned sanitiser removals. Its precondition is the SHA-256 of the source body. data-migration.md §7.4 describes how the import judges these entries (five points, below) and says this review waits for your confirmation. Do you confirm all five as built? Or should each entry also pin what the editors accepted, either as the reviewed removals or as the SHA-256 of the output, so that a later normaliser or sanitiser change that removes more gets reviewed again?" The maintainer answered "Confirm all as built", the recommended option, whose consequence includes: "ADR 0047 §4 sends every defect through one detector, so #318's rebase gives N7 an entry policy of its own in defects.rs, with tests: an entry whose precondition holds but whose body no longer has an unplanned removal is listed, not counted as a problem. ADR 0047 §1, §4's N7 row and §5, and data-migration.md §4.2 and §8.2, get an editorial correction that cites this answer. D49 said nothing about N7 entries, so this is a correction under ADR 0001 §5-§6, as with D157, not an amending ADR." The policy is `EntryPolicy::Acceptance`, whose outcome for such an entry is `entry_unused`. An entry accepts whatever N7 removes from the bytes it was signed for, so a later change to the sanitiser, the normaliser or a dependency that removes more from an accepted body goes through without a new review; only the dry run's per-body diff shows it, and V5(c) still stops on lost text (data-migration.md §7.4, §10).

### 5. What this ADR amends

| Amended text | Change |
|---|---|
| ADR 0017 §3, the second bullet: "A changed row stops the import until the entry is re-signed." | "In `apply`, a changed row stops the import until the entry is re-signed; a dry run lists it and goes on with the defect's proposed default (ADR 0047)." |
| ADR 0017 §3, the third bullet: "An instance the list does not cover stops the import, so the nightly rehearsal reports it long before the window." | "In `apply`, an instance the list does not cover stops the import (D10's entries are optional, so an unlisted page is imported as published), and so does an entry for a row that no longer has its defect (except a D3 entry whose post has the index image it pins, which holds, and an N7 entry whose body is unchanged but no longer has an unplanned removal, which is no longer needed and only listed). A dry run lists every such instance and entry, goes on with the defect's proposed default and fails at its end, so one nightly report shows them all long before the window (ADR 0047)." |
| ADR 0017 §3, the fourth bullet: "A row that still violates one fails the import." | "A row that still violates one fails the import; a dry run lists every such row, goes on without it and fails at its end (ADR 0047)." |

[data-migration.md](../data-migration.md) §4.2, §8.2, §11.1, §11.3 and §12 follow this ADR. The policies of §4 add no rule of their own: D10's optional entries and D3's entry that pins the fix made in the source are those of data-migration.md §8.1 and of the first paragraph of ADR 0017 §3, which the amended bullets now name. N7's acceptance is the review of `N7-accept` entries in data-migration.md §7.4, which the maintainer confirmed as built (research: DECISIONS R38 (D161)); the N7 clause of the third bullet's new text corrects this section under [ADR 0001](0001-record-architecture-decisions.md) §5 and §6, as §4 says.

## Consequences

**Positive**

- One nightly report shows every defect, refusal and constraint failure, so the resolution list can be filled from the first staging dry run, and the sign-off sheet is complete early (D49's consequence).
- `apply` is unchanged: the cut-over commits only what the editors signed.
- The non-zero exit keeps the safety signal: a dry run with an open problem never passes.

**Negative**

- The nightly stays red until the list covers every instance (D49's consequence). Until then, a red night means "see the Defects part" as often as "something broke", so the report's error says which it is.
- A dry run's later steps run on proposed defaults, so its row counts, digests and checks describe the defaults rather than a signed list, and a check can fail only because a default was applied. The Defects part names every row that took a default.
- While problems remain, a dry run and an `apply` run of the same dump end in different places: the dry run reaches its end, `apply` stops at the first problem.

**Follow-up work**

- Each table step of the import brings the rules of its defects (D1 to D10, and N7 in the normaliser) and adds its columns to the sign-off sheet.
- The first step that writes rows adds the row-by-row savepoint retry that collects constraint failures in a dry run.
- The nightly CronJob in website_k8s runs the dry run and then `apply` (cutover.md §5.2) and records both exits.

**What must be true in code and tests**

- The detector's tests cover, in both modes, an instance that an entry covers, an uncovered instance, a precondition mismatch, and an entry for a row that no longer has its defect or is gone, under each policy of §4: an unlisted D10 page takes the default without a problem, and a D3 entry holds once the post has the signed index image and fails while the image is missing, differs or the post is gone. A property test checks every policy against an independent statement of §4's table and that `apply` stops exactly at the dry run's first problem with its message.
- A property test checks every line of every sign-off sheet that a run can write against the allowed fields, over runs that reach every outcome and every kind of value.
- The task's run concludes the defect check last, so a dry run that collected a problem exits non-zero after its report and sheet are written; a test runs the task's `run` against the compose source and fails without it.

## Alternatives considered

| Option | Source | Why not |
|---|---|---|
| Keep stop-on-first in dry runs | research: decision register D49 | Not chosen: with D1 to D10 and N7, the list fills up one defect per night, which pushes the resolution list and the editors' sign-off later. |

## References

- research: DECISIONS R9 (D49, the answer of 2026-10-25); research: decision register D49 (the question, its context and the options)
- research: DECISIONS R38 (D161, the answer of 2026-11-15, which corrects §1, §4 and §5 for N7); research: third decision register D161 (the question, its context and the options)
- [data-migration.md](../data-migration.md) §4.2, §7.1, §7.4, §8.1, §8.2, §11.1, §11.3; [cutover.md](../cutover.md) §5.2, P2
- `src/domain/import_mysql/defects.rs`, `src/domain/import_mysql/n7_review.rs`, `src/domain/import_mysql/signoff.rs`, `src/domain/import_mysql/report.rs`, `src/tasks/import_mysql/mod.rs`
- Amended: [ADR 0017](0017-legacy-data-import-normalisation-and-defect-resolution.md) §3, its second, third and fourth bullets
