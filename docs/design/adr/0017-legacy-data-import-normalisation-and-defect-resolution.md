# 0017. Legacy data import: normalisation and defect resolution

- Status: Accepted (2026-10-24)
- Decision questions: Q15b, Q19, Q20, Q32b

## Context

backend3 starts with a copy of the legacy MySQL data (ADR 0002), taken in one rehearsed flip (ADR 0003). Three questions shape that copy: what happens to the stored rich text, what happens to the data defects, and how a consistent source is obtained.

### Stored rich text

The bodies were written by CKEditor 4.12.1 (posts since 2020-10-05, pages always) and carry its habits (research: SYNTHESIS §4.7, gap-05 §2-3, gap-06 §1-4):

| Fact | Count |
|---|---|
| Bodies with named HTML entities (`&eacute;` and the like, from the `entities` plugin's factory defaults) | 485 of 541 published posts, 27 of 35 drafts, 38 of 39 pages |
| Body words that contain an accented letter, so legacy body search cannot match them | 43 % of body words **[500]** |
| Legacy oEmbed wrappers `div[data-oembed-url]` (Iframely demo endpoint, at most 320 px wide) | 8 in 7 published posts, 1 in a draft |
| Bare YouTube iframes saved in source mode | 13 posts **[500]** |
| `<center>` elements | 9 posts **[500]** |
| Inline `data:image/jpeg` images | 16 in 6 posts (posts 91 and 92 are 892 KB and about 727 KB, mostly these), 1 in a page |
| Links to the dead host `backend.verseghy-gimnazium.net` (no DNS record) | 15 attributes (14 `img src`, 1 `href`) in 5 posts, plus 1 `href` in a page **[500]**; the host string appears in 11 published posts, 4 drafts and 1 page |
| Legacy layout that differs from the content-v1 shape (decision 15b) | 114 tables that stretch to 1000 px, 7 floated images, 19 YouTube players with fixed or 320 px sizes **[500]** |

**[500]** marks counts measured on a 500-post harvest of the public API rather than the full dump.

A DOM-based editor loads entity-encoded HTML without help, but its first save writes literal UTF-8, drops what its schema does not know, and changes many bytes (research: gap-05 §4.3). The shadcn-admin-kit default schema flattens tables and drops every iframe (research: gap-06 §6.1). So byte-identical storage cannot survive the first edit, and a raw-byte checksum after any normalisation differs on hundreds of rows by design. Checksums have to compare a DOM-normalised form.

### Data defects

Some defects are visible on the public site, and some block the constraints of ADR 0009 (research: SYNTHESIS §4.6.2, §4.6.5, gap-03 §8):

| Defect | Rows |
|---|---|
| Post dates one day early (a `TIMESTAMP` holding local midnight was cast to `DATE` without the zone correction) | 181 rows created before 2022-01-21 |
| Published posts without an author | 3 (55, 61, 66) |
| Published post without an index image; blocks `CHECK (NOT published OR index_image IS NOT NULL)` (ADR 0009) | 1 (70) |
| Posts without a body | 28 published, 6 drafts |
| Drafts with the `1970-01-01` sentinel date | 11 (and 5 with a NULL date) |
| Duplicate canteen dates | 8 dates, 16 rows; four pairs have different menus |
| Canteen days that break the 2-3 dish rule | 16 |
| Duplicate label names | 2 groups, 5 rows |
| Menu items without a position, served first | 5 |

Most of these block a `UNIQUE` or `CHECK` constraint or are wrong on the public site; the rest need an explicit decision, so that the import does not change them silently. The data is the only place where they can be fixed, because the Angular site does not change before the flip.

### The source database

The 20 legacy tables are MyISAM, and the database default charset is `latin1` (research: SYNTHESIS §4.1). A dump of tables that are not transactional is consistent only under a global read lock. Changing the production tables before the cut-over (an InnoDB conversion, a charset change, a hygiene script) was researched; all data fixes would still have to live in the importer, because the old admin can recreate them (research: SYNTHESIS §4.12.1, gap-02 §3.2). backend2's hand-run migration 12 fails on today's data and, after a back-fill, would break menu creation in the old admin (research: gap-07 §4).

## Decision

### 1. One import task, rehearsed

backend3 has a Loco task `import_mysql` that reads the legacy database and writes backend3's PostgreSQL database. [data-migration.md](../data-migration.md) is its specification: every table and column mapping, every transformation, the defect list, the verification checks and the refusal rules. In short:

- It reads the whole source under one `LOCK TABLES … READ` and checks the source schema against the expected DDL first.
- It computes every target row with pure functions in `src/domain/` (ADR 0008), writes everything in one transaction, runs the verification checks inside that transaction, and commits only if all pass.
- `mode:dry-run` is the default: the full run, then a rollback, plus a report with a per-row diff of every changed body. `mode:apply` commits. The nightly and dress rehearsals on staging run `apply` into a freshly recreated database, so the commit path is rehearsed too (data-migration.md §11.1).
- It refuses to write into a database that already holds data, so it cannot run again after go-live.
- The MySQL driver is compiled in behind a Cargo feature `import-mysql` for as long as the importer exists.

### 2. Bodies are normalised once, to the editor's fixed point (Q19)

The importer rewrites every `posts.content` and `pages.content` (data-migration.md §7):

1. Parse with html5ever, which decodes every character reference.
2. Convert legacy oEmbed wrappers and bare YouTube iframes to the editor's YouTube node, the padlet iframe to the editor's padlet node, and any other embed to a link (iframes are allowed only from YouTube and padlet, ADR 0013).
3. Unwrap `<center>` into `text-align: center`.
4. Extract every inline `data:` image to `/storage/uploads/posts/<id>/inline-<n>.<ext>` (for the one page image, see data-migration.md §13) and point the `img` at it; drop 1×1 px artefacts.
5. Map the remaining legacy presentational markup onto the editor schema.
6. Apply the content-v1 layout of decision 15b: floated images (7) are centred; old YouTube players become the responsive player of at most 640 px; the 114 tables that stretch to 1000 px stop doing so; images wider than 700 px are untouched. No legacy node variants are kept.
7. Rewrite dead-host `/storage/` links to root-relative `/storage/…`.
8. Run the write-path sanitiser of ADR 0013.
9. Serialise. Only `&amp; &lt; &gt; &nbsp;` (and `&quot;` in attributes) remain as entities.

The output is the editor's **fixed point**: normalising it again, sanitising it again, or loading and saving it in the admin editor leaves every byte unchanged. Table cell styles from Word and Excel are kept (decision 15), and cell layout (borders, fills, widths, alignment) is always kept (decision 15c).

The original bytes of every changed body are kept in the row's `import_original` revision, which precedes its `import` revision (ADR 0011). The same normaliser is also a standalone task, `content_normalize` (`mode:dry-run` by default), that runs over backend3's own posts and pages; right after an import it reports zero changes.

### 3. Defects are fixed from an editor-signed list (Q20)

The importer applies a resolution list that editors sign off before the cut-over (data-migration.md §8): +1 day on the 181 early post dates; real or generic (`Szerkesztőség`) authors for posts 55, 61 and 66; an editor-chosen index image for post 70, set in the current admin before the final dump (the importer refuses a published post without one); NULL instead of the 1970 sentinel on drafts; one row kept per duplicate canteen date; one label kept per duplicate-name group; today's served order for the 5 unpositioned menu items. Rows without a body and days that break the dish rule are imported unchanged; the dish rule applies when such a day is next saved.

- The list is a versioned file in the repository holding only ids, dates, choices and source values. Who signed is recorded in the private runbook.
- Every entry carries a precondition: the source value at sign-off. A changed row stops the import until the entry is re-signed.
- Every defect is detected from its rule on every run. An instance the list does not cover stops the import, so the nightly rehearsal reports it long before the window.
- The constraints of ADR 0009 are created by the schema migrations and therefore apply as the rows are inserted. A row that still violates one fails the import.

### 4. Production MySQL stays untouched; dumps lock all tables (Q32b)

- Until the cut-over, the production tables keep their engine (MyISAM) and their charset defaults, and no clean-up changes their data. No hygiene script runs on production. Every clean-up lives in the importer. Editors keep working in the current admin as usual; the post 70 index image of §3 is one such ordinary edit.
- Every dump used as an import source is taken with `mysqldump --lock-all-tables` (decision 32b), because the legacy tables are not transactional. The global read lock is held for the few seconds the dump takes.
- backend2's migration 12 (`migrations/12-menu_items_fix_types.sql`) must never be applied.

## Consequences

Positive:

- Body search works for accented words from day one, because the stored bodies are literal UTF-8 (the derived search column of ADR 0020 is computed from them).
- Every legacy embed keeps working in the new editor, and the old Iframely and bare YouTube embeds become the responsive content-v1 player of at most 640 px.
- The first save in the new admin does not silently rewrite or damage a post: the body is already in the editor's form.
- Posts 91 and 92 shrink from hundreds of KB to normal sizes, and the dead-host images and links also work in server-rendered HTML.
- The public site's dates, authors, canteen weeks and labels are right after the flip, and the new constraints hold from the first row.
- No change to production MySQL is needed before the cut-over, and nothing an old-admin edit does can undo an importer fix.

Negative:

- The stored bytes of about 550 rows change. They are DOM-identical or differ only by the planned conversions, but a raw-byte comparison with the legacy data no longer works. Verification uses DOM-normalised checksums and text comparisons instead (data-migration.md §10).
- Visible one-time changes on the live site (decision 15b): 114 tables stop stretching to 1000 px, 7 floated images are centred, and legacy YouTube players become responsive at most 640 px. The dry-run diff shows each one.
- The normaliser depends on the content-v1 schema of [ADR 0013](0013-rich-text-editor-and-html-sanitiser.md) ([editor.md](../editor.md) §3, §9). Any change to that schema bumps `POLICY_VERSION` and needs a `content_normalize` run (with revisions) or a proof that the stored bodies are still at the fixed point.
- 181 public post dates and some archive months change. Readers may notice older posts moving by one day.
- Merging labels and dropping duplicate canteen days removes 3 label ids and 8 canteen-day ids. Public links to the merged label ids need a decision (data-migration.md §13).
- The editors must review and sign the list before the cut-over, set post 70's index image in the old admin, and re-sign entries whose rows change in the old admin.
- `--lock-all-tables` blocks old-admin writes for the few seconds of each dump.
- The binary carries a MySQL driver until the importer is removed.

What must be true in code and tests:

- Every rule is a pure function with unit tests on hand-written fixtures that reproduce the production shapes; the normaliser has property tests for idempotence; the CI integration test runs the importer from a synthetic MySQL 8.4 fixture into Postgres (data-migration.md §12).
- The admin test suite asserts the editor round trip over the normaliser's fixture outputs.
- Dumps and run reports never reach the public repository or public CI, because they contain account data and draft content.

## Alternatives considered

| Question | Option | Why not |
|---|---|---|
| Q19 | B. Decode entities only; keep embeds, `data:` images and dead-host links | The new editor would still drop the legacy embeds and the bare iframes on the first save, and the broken images would stay broken. |
| Q19 | C. Byte-identical import plus a derived search column | The first edit in the new editor rewrites the bytes anyway and drops the embeds, so byte identity cannot be kept. Bodies would stay in two encodings for years. |
| Q15b | Keep the legacy layout (floats, fixed player sizes, stretched tables) through legacy editor node variants | Not chosen: decision 15b gives old and new content one content-v1 shape and keeps no legacy node variants. |
| Q20 | B. Import byte-for-byte; new constraints apply only to new writes; editors fix rows later | The posts without authors stay broken on the public site, and the unique and check constraints cannot exist while duplicates remain. |
| Q20 | C. Fix only what breaks pages (the null authors and post 70, through data or an API placeholder) | The wrong dates, duplicate canteen days and duplicate labels stay, and so does the lack of constraints. |
| Q32b | A. Convert the production tables to InnoDB and the default charset to `utf8mb4` now | Rejected by the maintainer: production stays untouched until the cut-over; `--lock-all-tables` gives consistent dumps without touching it. |
| Q32b | B. Run the full hygiene script on production now | The data fixes must live in the importer anyway, because the old admin recreates some of them on every edit. |
| Q32b | D. Leave production untouched and rely on the cut-over freeze alone for consistency | Rehearsal dumps taken without a lock could be inconsistent, so rehearsals would not prove what the final import will do. |

## References

- research: DECISIONS 15b, 19, 20, 32b (and 15, 15c, 21, 23b, 25b)
- research: SYNTHESIS §4.1, §4.6.2, §4.6.5, §4.7, §4.10 (R4-R9, R16-R18, R23-R25), §4.12.1, §4.13
- research: gap-02 §1.4, §3.2; gap-03 §8; gap-05 §2-§6; gap-06 §1-§7; gap-07 §4, §7.4; gap-09 §8
- research: QUESTIONS Q19, Q20, Q32
- website_backend2@7ba71da: `migrations/11-posts_data_fix_types.sql`, `migrations/12-menu_items_fix_types.sql`, `migrations/13-posts_fix_null_date.sql`
- [data-migration.md](../data-migration.md)
- ADR 0002, ADR 0003, ADR 0008, ADR 0009, ADR 0010, ADR 0011, ADR 0013, ADR 0018, ADR 0020
