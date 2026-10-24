# 0020. Search and Hungarian ordering

- Status: Accepted (2026-10-24)
- Decision questions: Q38

## Context

The public site has one search: the header search box, which lists published posts whose title, description or body contains the typed term. backend2 implements it, and its behaviour is a parity requirement. The admin also searches: every backend1 list screen has a search box, and the new canteen week editor needs a fast dish type-ahead (ADR 0018). Names are sorted in several places, and the colleagues page has a Hungarian ordering rule of its own.

MySQL gave case- and accent-insensitive matching for free through its collation. PostgreSQL (ADR 0002) does not.

| Fact | Value | Source |
|---|---|---|
| backend2 search | `content LIKE p OR description LIKE p OR title LIKE p` with `p = '%term%'`, on published posts, ordered `date DESC, id DESC` | website_backend2@7ba71da:src/graphql/resolvers/posts.rs; research: SYNTHESIS §5.4 |
| Case and accent folding | Every MySQL text column is `utf8mb4_unicode_ci`, so `LIKE` ignores case and accents. The backend2 test `search_ignores_case_and_accents` pins it: `UNNEP` finds posts containing „ünnep”. | research: SYNTHESIS §5.8; website_backend2@ced400b:tests/posts.rs |
| Wildcards | `%` and `_` in the term are not escaped and act as wildcards; an empty term matches every post | research: SYNTHESIS §5.4, §5.8 |
| Stored bodies | CKEditor stored á, é, í, ó, ö, ú, ü (and capitals) as named entities in text nodes: 485 of the 513 published posts that have a body (541 published in all) and 27 of the 29 drafts that have one. No term containing one of these letters can match such a body; that is 43 % of body word tokens. Conversely, `LIKE` matches markup such as `eacute`, `width` or `youtube`. | research: SYNTHESIS §4.7.1, §4.7.2; gap-05 §4.1 |
| Import | ADR 0017 decodes the entities once at import, so stored bodies become literal UTF-8. They are still HTML: tags and attribute values stay in the column, and a phrase split by a tag (`<strong>Ünnepi</strong> műsor`) cannot match a plain `LIKE`. | research: DECISIONS row 19 |
| Plain-text columns | `title` and `description` hold 0 entities; `description` holds 0 tags (titles were not checked for tags; the search PR's import test asserts it) | research: SYNTHESIS §2.4.1, §2.4.6 (POS-C06), §4.7.1 |
| PostgreSQL collations | Deterministic collations are case- and accent-sensitive. `LIKE` with a nondeterministic ICU collation is allowed only from PostgreSQL 18; the probe below confirmed that PostgreSQL 16 refuses it. The PostgreSQL major is not fixed yet (ADR 0002). | research: SYNTHESIS §4.10 R6 |
| Colleague order | backend2 sorts colleagues with an ICU `hu` collator after removing a leading `"Dr. "` (exact prefix, case-sensitive). The test `colleagues_are_sorted_by_name_in_hungarian_order` pins the order. backend1 issue #342 ("Name ordering with 'Dr.' prefix is broken") is where the rule came from. 3 of 70 names start with `Dr.`. | website_backend2@7ba71da:src/graphql/resolvers/colleagues.rs; website_backend2@ced400b:tests/content.rs; research: SYNTHESIS §2.7, §7.9.6 |
| Client re-sort | The Angular site re-buckets the colleague list by category and re-sorts part of it with a plain string compare. That is frontend behaviour; backend3 only has to keep the server order. | research: SYNTHESIS §5.8 |
| Hungarian alphabet | `unicode_ci` is not Hungarian order either. In Hungarian, `cs`, `dz`, `dzs`, `gy`, `ly`, `ny`, `sz`, `ty` and `zs` are letters of their own, and `ö`/`ő` follow `o`/`ó`. | website_backend2@ced400b (branch `test/integration`):src/graphql/resolvers/colleagues.rs (doc comment of `compare_names`) |
| Admin search in backend1 | the list search box runs `LIKE '%term%'` over the screen's text columns under the same collation | research: SYNTHESIS §2.2 (SHR-04), §2.12, §3.5.1 |
| Size | 576 posts; the whole database is 6.05 MiB in 6,677 rows | research: SYNTHESIS §0 fact 2, §2.4.1 |
| Image contents | Whether the production PostgreSQL image ships `unaccent` and ICU collations is not verified (open item O16); the image itself is chosen under ADR 0002 | research: SYNTHESIS §8 O16 |

A probe on 2026-10-24, run against an official-image-based PostgreSQL 16.14 container, settled the engine facts this ADR relies on:

| Check | Result |
|---|---|
| `unaccent` | installed with the image's contrib files; its control file says `trusted = true`; both `unaccent(text)` and `unaccent(regdictionary, text)` are `STABLE`, and `CREATE INDEX … (unaccent(c))` fails with "functions in index expression must be marked IMMUTABLE" |
| `unaccent('ÁÉÍÓÖŐÚÜŰ áéíóöőúüű')` | `AEIOOOUUU aeiooouuu` (ő and ű are covered) |
| `lower('ÁÉÍÓÖŐÚÜŰ' COLLATE "C")` | unchanged: under the C collation `lower()` folds ASCII only, so `unaccent(lower('Ünnepi'))` gives `Unnepi`, which does not match `unnep` |
| `lower(… COLLATE hu_icu)` | `áéíóöőúüű`, and `unaccent(lower('ÜNNEPI Műsor' COLLATE hu_icu))` gives `unnepi musor` |
| `CREATE COLLATION hu_icu (provider = icu, locale = 'hu')` | succeeds; deterministic; the image also predefines `hu-x-icu` |
| Nondeterministic collation with `LIKE` on 16.14 | `ERROR: nondeterministic collations are not supported for LIKE` |
| `LIKE … ESCAPE '\'` | `'%0\% s%'` matches „100% siker” and not „1000 siker” |
| `unaccent` and the `LIKE` metacharacters | the image's `unaccent.rules` folds six code points to them: U+FF05 and U+FE6A to `%`, U+FF3F to `_`, U+FF3C, U+FE68 and U+2216 to `\` |
| Escaping before folding | the term `0％ s` (U+FF05) matches „1000 siker”, `a＿b` (U+FF3F) matches „axb”, and `x＼` (U+FF3C) misses a text that contains `x＼`, because the folded `\` escapes the closing `%` |
| Folding before escaping | the same three terms match only literally (`0％ s` matches „100% siker”, `a＿b` matches „a_b”, `x＼` matches „x＼y”), and `UNNEP` still matches „Ünnepi” |

Ordering of the same ten words under three collations (probe output):

| Collation | Order |
|---|---|
| `hu_icu` (ICU `hu`) | ábra, alma, cukor, csiga, olló, óra, öröm, őz, zúgó, zsák |
| `und-x-icu` (root ICU) | ábra, alma, csiga, cukor, olló, óra, öröm, őz, zsák, zúgó |
| `C` (bytes) | alma, csiga, cukor, olló, zsák, zúgó, ábra, óra, öröm, őz |

Only `hu` puts every `c…` word before every `cs…` word and every `z…` word before every `zs…` word.

## Decision

backend3 keeps backend2's semantics: case- and accent-insensitive substring matching. It searches a derived plain-text copy of the post body, matches with `unaccent(lower(column)) LIKE unaccent(lower(pattern))` with `%` and `_` (and the escape character `\`) escaped after folding, and sorts names with an explicit ICU `hu` collation, stripping the `"Dr. "` prefix for colleagues. This works the same on PostgreSQL 16 and 18.

### 1. Database objects

One migration creates both objects and fails loudly if the image lacks either; there is no silent fallback.

```sql
CREATE EXTENSION IF NOT EXISTS unaccent;
CREATE COLLATION IF NOT EXISTS hu_icu (provider = icu, locale = 'hu');
```

The collation gets its own name instead of relying on the predefined `hu-x-icu`, so the migration states what it needs and the schema does not depend on which ICU locales `initdb` imported. `unaccent` is a trusted extension, so a database owner with `CREATE` on the database can install it without superuser rights; the database owner role `website_backend3` runs the migrations (ADR 0002).

### 2. The derived plain-text column

| Item | Rule |
|---|---|
| Column | `posts.content_text text NOT NULL DEFAULT ''`; never part of any API response |
| Function | `domain::rich_text::html_to_search_text(html: &str) -> String`, a pure function (ADR 0008) |
| What it keeps | text nodes only, with every entity decoded by the HTML parser |
| What it drops | tags, attribute values (`href`, `src`, `alt`, `style`), and the contents of `script`, `style` and `template` |
| Spacing | a single space at every block-element boundary and at `<br>`; U+00A0 (no-break space) becomes a space; whitespace runs collapse to one space; the result is trimmed |
| Parser | the same HTML5 parser family as the sanitiser (html5ever, ADR 0013); the exact crate API is verified in the search PR |
| Maintenance | the post model's `before_save` hook recomputes it whenever `content` changes; the importer fills it after body normalisation (ADR 0017); an idempotent `cargo loco` task recomputes every row, to be run whenever the function changes |
| Scope | posts only. `title` and `description` are already plain text and are searched directly. |

### 3. Matching

| Step | Rule |
|---|---|
| Term, public search | [public-api.md](../public-api.md) §7.2 and §8 own the rule: the term is trimmed and must be 1 to 200 characters without U+0000; a missing, empty, white-space-only or overlong term gives `400 invalid_query`. |
| Term, admin | Admin list search and type-aheads trim the term the same way. An empty term applies no filter. More than 200 characters gives 422 `search.term_too_long` („A keresett kifejezés legfeljebb 200 karakter lehet.”), and a U+0000 gives 422 `search.invalid_term` („A keresett kifejezés érvénytelen karaktert tartalmaz.”), both in the admin error envelope (ADR 0021 §4). A type-ahead returns at most 20 results. |
| Matcher input | The matcher always receives a non-empty, trimmed term and binds it as a parameter unchanged. It is never pre-escaped in Rust and never interpolated into SQL. |
| Folding | `fold(x) = unaccent(lower(x COLLATE hu_icu))`. The collation sits inside `lower()` because `lower()` follows the collation of its argument: under the C collation it leaves `Ü` and `Ő` unchanged (probe above). Pinning it makes folding independent of the database's default locale. |
| Escaping | done in SQL, after folding: `replace(replace(replace(fold($term), '\', '\\'), '%', '\%'), '_', '\_')`, `\` first. Folding first matters because `unaccent` turns six code points into `%`, `_` or `\` (probe above); escaping before folding would let them act as wildcards or as a stray escape. |
| Predicate | `fold(column) LIKE ('%' || replace(replace(replace(fold($term), '\', '\\'), '%', '\%'), '_', '\_') || '%') ESCAPE '\'` |
| One implementation | a single Rust helper emits this predicate for a given column, with the raw trimmed term as its one bound parameter; no query writes it by hand. A unit test pins the SQL the helper emits. |

**Public post search.** Published posts where any of `title`, `description` or `content_text` matches, ordered `date DESC, id DESC`, paged as the REST spec says ([public-api.md](../public-api.md) §7.2).

**Admin list search.** The same helper over the columns backend1's search box covers on each screen (for example posts: title and author name; colleagues: name and jobs; pages: title, internal note and slug, because backend1 searched `name` and `slug` and `name` usually equalled the title; research: SYNTHESIS §2.12, §3.5.1). The parity checklist (ADR 0019) records the column list per resource.

**Type-aheads.** The dish picker of the canteen week editor (ADR 0018) and every other admin type-ahead use the same helper.

**No index.** At this size a sequential scan is cheap. `unaccent` is `STABLE`, so PostgreSQL rejects it in an index expression. If search latency ever matters, the follow-up is a `pg_trgm` index over an `IMMUTABLE` wrapper that names the unaccent dictionary explicitly; that is a separate decision, recorded here so nobody tries to index `unaccent()` directly.

**What changes for a reader of the public site:**

| Term and content | backend2 today | backend3 |
|---|---|---|
| `UNNEP` against a title containing „Ünnep” | match | match |
| `ünnep` against „ünnepség” | match (substring) | match (substring) |
| `eredmenyek` against a body containing „eredmények” | no match (the body was entity-encoded) | match |
| `öt` against „őt” | match (`unicode_ci`) | match (both fold to `ot`) |
| `Ünnepi műsor` against `<strong>Ünnepi</strong> műsor` | no match (a tag sits between the words) | match |
| `eacute`, `width`, `youtube` against markup only | match | no match |
| `50%` or `a_b` | wildcards | literal characters |

### 4. Hungarian ordering

| Where | Order |
|---|---|
| Public colleagues list | `ORDER BY regexp_replace(name, '^Dr\. ', '') COLLATE hu_icu, id`: one leading `"Dr. "` is ignored, matched exactly and case-sensitively as backend2 does. The prefix is one constant in the code. The order no longer depends on which fields a client selects (backend2 sorted only when `name` was selected). |
| Admin colleagues list | the same sort key, so editors see the public order |
| Every other sort on a name or title | `ORDER BY <column> COLLATE hu_icu, id` for public and admin lists: labels, authors, dishes, page titles, users, and admin list sorts on any text column |
| Orders that are not by name | unchanged from the REST spec, for example posts by `date DESC, id DESC` and labels on a post by `id DESC` |
| Sorting in the browser | admin option lists and other client-side sorts use `Intl.Collator('hu')` (ADR 0021), so client and server agree |

`id` is the tie-break everywhere, so the order is total and stable across pages.

## Consequences

**Positive**

- Header search keeps backend2's case- and accent-insensitive substring behaviour and starts finding accented words in post bodies.
- Searches no longer match HTML markup, and `%`, `_` or `\` in a term mean themselves, including the full-width and small forms that `unaccent` folds to them.
- One folding helper serves the public search, the admin search boxes and the type-aheads, so they cannot disagree.
- Name order is Hungarian everywhere, independent of the database's default locale and of the PostgreSQL major.

**Negative**

- `content_text` stores a plain-text copy of every post body (smaller than the HTML, a few MiB at most at today's volume) and must be kept in step with `content`; the write hook, the import and the recompute task are the three places that do it.
- Queries scan the posts table. That is fine at 576 posts; a much larger archive would need the `pg_trgm` follow-up.
- ICU upgrades can shift collation order slightly. No index uses `hu_icu`, so nothing breaks; the order of a few names might change after an image upgrade, which is acceptable for display order.
- The migration hard-fails on an image without `unaccent` or ICU. That is intended, but it makes the image choice (ADR 0002) a precondition.

**What must be true in code and tests**

- The ADR 0002 image dry run executes this ADR's migration (`CREATE EXTENSION unaccent` and `CREATE COLLATION hu_icu`), which closes O16 for the chosen image. CI runs the tests against the same PostgreSQL major and image family.
- Search tests written from the REST spec (ADR 0019) cover every row of the "What changes for a reader of the public site" table in §3, including an entity-encoded fixture body imported through the normaliser, as decision 25b requires for production-only shapes.
- Unit and property tests for `html_to_search_text`: the entity-encoded and the literal form of the same body give identical output; the output never has leading, trailing or repeated whitespace; random text wrapped in random inline tags comes back unchanged apart from whitespace.
- A unit test that pins the predicate the helper emits (fold, then escape, then the `%` wrappers). A database literal-match test with `%`, `_`, `\`, mixed input, and the six code points U+2216, U+FE68, U+FE6A, U+FF05, U+FF3C and U+FF3F that `unaccent` folds to `\`, `%` or `_`: each term matches only texts that contain it literally, after folding.
- Admin search tests: an empty term returns the unfiltered list; 201 characters gives 422 `search.term_too_long`; a U+0000 gives 422 `search.invalid_term`; a type-ahead never returns more than 20 results.
- An ordering test with the probe's ten words, and a colleagues test with `Dr. ` and `dr. ` prefixes (only the first is stripped).
- A test that saving a post updates `content_text`, and a test that the recompute task is idempotent.

## Alternatives considered

- B. PostgreSQL 18 with a nondeterministic ICU collation (for example `und-u-ks-level1`) on text columns, so plain `LIKE` and `=` behave like MySQL's `unicode_ci`: rejected; it ties the design to PostgreSQL 18, still needs the decoded column for bodies, and PostgreSQL 18 restricts mixing collations across primary and foreign keys.
- C. Full-text search with a `tsvector`, the `hungarian` snowball configuration plus unaccent, and ranked results: rejected; matching becomes stemmed whole words, so substring matches that work today (a partial name, the start of a word) would stop working.

## References

- research: QUESTIONS Q38; DECISIONS rows 19 and 38
- research: SYNTHESIS §2.2 (SHR-04), §2.7, §2.12, §3.5.1 (admin search columns), §4.7 (stored rich text), §4.10 R6 (collation risk), §5.4 and §5.8 (search semantics and colleague sort), §7.9.6 (#342), §8 O16
- research: gap-05 §4 (entity encoding and search), gap-10 §4 (REST draft for search)
- website_backend2@7ba71da:src/graphql/resolvers/posts.rs, src/graphql/resolvers/colleagues.rs
- website_backend2@ced400b (branch `test/integration`): tests/posts.rs (`search_ignores_case_and_accents`), tests/content.rs (`colleagues_are_sorted_by_name_in_hungarian_order`), src/graphql/resolvers/colleagues.rs (doc comment of `compare_names`)
- [public-api.md](../public-api.md) §7.2 (search parameters) and §8 (error codes)
- PostgreSQL documentation: the `unaccent` module, collation support (ICU), `LIKE` and `ESCAPE`
- Probe on PostgreSQL 16.14, 2026-10-24: results quoted in Context, including the `unaccent.rules` entries that fold to `%`, `_` and `\` and both escaping orders
- Related ADRs: [0002](0002-postgresql-as-the-database.md), [0005](0005-rust-dtos-are-the-api-contract-ts-rs-utoipa-openapi.md), [0008](0008-code-layout-a-pure-src-domain-beside-loco-models.md), [0013](0013-rich-text-editor-and-html-sanitiser.md), [0017](0017-legacy-data-import-normalisation-and-defect-resolution.md), [0018](0018-content-model-publishing-pages-menu-tree-canteen-week-editor.md), [0019](0019-scope-parity-tracking-retired-features-release-1-additions.md), [0021](0021-hungarian-only-admin-ui.md), [0023](0023-operations-hardening-rate-limits-monitoring-backups-secrets.md)
