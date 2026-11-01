# 0010. All instants stored as timestamptz

- Status: Accepted (2026-10-24)
- Decision questions: Q9

## Context

The legacy MySQL database mixes four kinds of time value, and nothing in the schema records which is which (research: SYNTHESIS §4.6.2, §4.10 R3):

| Kind | Legacy columns | What the value means |
|---|---|---|
| `TIMESTAMP` | `users`, `password_resets`, `roles`, `permissions`, `newsletter`, `menu_items` (created/updated/deleted) | a UTC instant; `mysqldump` writes it in UTC (`SET TIME_ZONE='+00:00'`) |
| `DATETIME` audit columns | `posts_data`, `pages`, `events_data`, `canteen_menus` (`created_at`, `updated_at`) | UTC wall time, written by Laravel with the application time zone `UTC` |
| `DATETIME` audit columns of `canteen_data` | `created_at`, `updated_at` | shifted by one or two hours to local time by backend2's hand-run fix 03 (website_backend2@7ba71da:`migrations/03-canteen_data_add_time.sql`) |
| `DATETIME` event times | `events_data.date_from`, `date_to` | Europe/Budapest wall time typed by an editor; 136 rows carry picker seconds such as `00:00:52` |
| `DATE` | `posts_data.date`, `canteen_data.date` | a calendar day with no time of day |

The mix has already caused a visible bug. Post dates were once a `TIMESTAMP` that held local midnight, which is 22:00 or 23:00 UTC of the previous day. backend2's fix 11 cast that column to `DATE` without correcting the zone, so every post created before 2022-01-21 is dated one day early (research: SYNTHESIS §4.6.2 #1, gap-03 §8.2; repaired by ADR 0017).

The consumers that cannot change (research: SYNTHESIS §5.6):

- The Angular site parses event times with date-fns 1.30.1. A zone-less string is read as local time. A `Z` suffix would move evening events to the next day for Budapest readers.
- Post and canteen dates are printed as `YYYY-MM-DD`. A time or an offset would shift the printed day in some zones. The Angular SSR pod runs in UTC.

Loco 1.2.0 creates `created_at` and `updated_at` as `timestamptz` (loco-rs@1.2.0:`src/schema.rs`, `timestamps_tz`).

The recommended default was option A, split by meaning: instants as `timestamptz`, event times as naive Budapest wall time, and post and canteen dates as `date` (research: QUESTIONS Q9). The maintainer chose option B instead. Decision 9 reads: "Everything `timestamptz`, events included (user overrode the split-by-meaning default). Importer converts event wall times Europe/Budapest -> UTC; the public API renders them back as naive Europe/Budapest `YYYY-MM-DD HH:MM:SS` so the Angular calendar is unchanged."

## Decision

### Storage

1. **Every instant is `timestamptz`**, stored in UTC. This covers:
   - audit and lifecycle columns: `created_at`, `updated_at`, `deleted_at`, revision and activity-log times (ADR 0011);
   - authentication instants: `last_login_at`, token and reset-link expiry, lockout windows (ADR 0006);
   - `posts.publish_at` (nullable, unused in release 1; ADR 0018);
   - **event start and end**: `events.starts_at` and `events.ends_at`. The importer converts the legacy wall times from Europe/Budapest to UTC, and the APIs convert them back.
2. **Calendar dates are `timestamptz` too.** `posts.date` (NULL on drafts only) and `canteen_days.date` are `timestamptz` holding the Europe/Budapest midnight of the day, with `CHECK ((date AT TIME ZONE 'Europe/Budapest')::time = time '00:00')` (ADR 0018 §1.3). Every later calendar-day column, such as the Diáktoll issue date of ADR 0019, follows the same rule. Since 1984 Budapest's clocks change at 02:00 and 03:00 local time, so every calendar day from 1984 on has exactly one local midnight. Under the older rules a change on a few days between 1916 and 1983 skipped or repeated local midnight, so such a day has no midnight (for example 1981-03-29) or two (for example 1981-09-27); the conversion module refuses a day with none and converts a day with two to the earlier one. Converting a day to its midnight and back goes through the module of point 5.
3. No column uses `timestamp without time zone` or `date`.

### One zone, one module

4. The school's zone is **Europe/Budapest**. It is a constant in `src/domain/` (ADR 0008). It is not configuration, and no code reads the process `TZ`.
5. Every conversion between a Budapest wall time and an instant goes through one pure module in `src/domain/` that uses a compiled-in IANA time-zone database (for example the chrono-tz crate; the bootstrap PR pins the crate and its tz data version).
6. Conversion rules from wall time to instant:

| Case | Example (2026) | Rule |
|---|---|---|
| Unique local time | any normal time | that instant |
| Ambiguous local time (autumn: 02:00-02:59 occurs twice) | 2026-10-25 02:30 | the earlier instant, `+02:00` (CEST). Converting back gives the same wall time. |
| Non-existent local time (spring: 02:00-02:59 is skipped) | 2026-03-29 02:30 | rejected. The admin shows a validation error on the field; the importer stops and reports the row. |

   For every wall time that exists, converting to an instant and back returns exactly the wall time that was entered, seconds included.

### Wire formats

| Value | Public API (`/api/v1`) | Admin API (`/api/admin`) |
|---|---|---|
| Event start and end | naive Europe/Budapest `YYYY-MM-DD HH:MM:SS`, as backend2 sends today | the same naive Europe/Budapest format; the server converts in both directions |
| `posts.date`, canteen day `date` | `YYYY-MM-DD`, the Europe/Budapest calendar day of the stored midnight | `YYYY-MM-DD`; the server converts in both directions |
| Any other instant | RFC 3339 in UTC with `Z`, where the public API exposes one at all (for example a sitemap `lastmod`) | RFC 3339 in UTC with `Z`. The admin UI shows it in Europe/Budapest, whatever the browser's zone. |

A DTO field never carries a zone-less instant whose zone is implied by the reader's machine. The only zone-less date-times on the wire are the event fields, and their zone is Europe/Budapest by definition.

### "Today" and other calendar questions

7. "Today", "this week" and "this month" are computed in Europe/Budapest from an injected clock, never from the server's zone. This covers the default post date (Q21: "date required, default today"; stored as today's Budapest midnight), the current ISO week of the canteen, and the month grid of the events calendar.
8. The events calendar query turns the grid bounds (Budapest midnights) into instants and filters `ends_at >= start AND starts_at < end`.

### SQL rules

9. backend3 sets the database session `TimeZone` to `UTC` (ADR 0002).
10. SQL that derives a date, hour, month or year from a `timestamptz` names the zone: `(starts_at AT TIME ZONE 'Europe/Budapest')::date`. A bare `::date` cast, `now()::date` or `date_trunc` on a `timestamptz` without a zone is a review-blocking defect. This applies to `posts.date` and `canteen_days.date` as well, for example to the archive's year-and-month grouping.

### Import

11. The importer applies one rule per legacy column kind ([data-migration.md](../data-migration.md) §5.2):

| Legacy kind | Rule |
|---|---|
| `TIMESTAMP` | read through a MySQL session with `time_zone = '+00:00'`, stored as that UTC instant |
| `DATETIME` audit, written by Laravel | interpreted as UTC |
| `canteen_data.created_at` and `updated_at` | interpreted as Europe/Budapest wall time and converted to UTC (see open items) |
| `events_data.date_from` and `date_to` | interpreted as Europe/Budapest wall time and converted to UTC with the rules of point 6 |
| `DATE` | after ADR 0017's resolutions (+1 day repair, 1970 sentinels to NULL), converted to the Europe/Budapest midnight instant of that day |

## Consequences

Positive:

- One storage type for every moment in time. Ordering, durations and comparisons are correct across daylight-saving changes.
- The schema matches Loco's own default, so generated migrations need no edits for time columns.
- The Angular calendar and date rendering keep working unchanged, because the public API keeps today's formats.

Negative:

- Event times depend on the time-zone rules. An instant stays fixed when the rules change, but its Budapest wall time would then render differently. If Hungary's daylight-saving rules ever change, already stored future events would shift by an hour unless they are re-converted. A tz-data bump PR states the new tz data version. Before it merges, the operator runs a read-only task `events_tz_report` on staging, against the newest copy of the production data, under the old and the new build. The task prints the Europe/Budapest wall times of future events. The operator diffs the two outputs and re-enters any event that moved. Only the count of changed events goes into the PR, because production data never goes into a PR (ADR 0003).
- Every event read and write converts. A forgotten conversion is a silent one- or two-hour error, so the conversion lives in one module and is tested.
- Calendar dates need the conversion on every read and write as well. A stored midnight rendered in UTC shows the previous day, so the date conversion lives in the same module, the check of point 2 refuses any value that is not a Budapest midnight, and tests pin both.
- Local times in the spring gap cannot be stored. The admin has to explain the rejection.

What must be true in code and tests:

- Unit tests of the conversion module cover both 2026 transitions (2026-03-29 and 2026-10-25), the year boundary and leap days, for wall times and for calendar dates.
- A property test (proptest) asserts the round trip `instant_to_wall(wall_to_instant(w)) == w` for every wall time outside the spring gap, and the same round trip for every calendar date through its Budapest midnight.
- Database tests show that the check of point 2 refuses a `posts.date` or `canteen_days.date` that is not a Budapest midnight (ADR 0018).
- API snapshot tests pin the event format `YYYY-MM-DD HH:MM:SS` and the date format `YYYY-MM-DD`.
- The test suite runs with at least two different process `TZ` values in CI, and the outputs are identical.
- The import test feeds an event at 23:30 Budapest time in summer and in winter and asserts the stored UTC instant and the rendered wall time.
- A lint or review rule flags `::date`, `now()::date` and `date_trunc(` on `timestamptz` columns without `AT TIME ZONE`.

Open items:

- **Canteen audit columns.** backend2's fix 03 was a one-off `UPDATE` in January 2022 that shifted the `created_at` and `updated_at` of the rows that existed then by one or two hours (website_backend2@7ba71da:`migrations/03-canteen_data_add_time.sql`; 322 pre-fix rows, research: SYNTHESIS §4.4.2). Rows written later by Laravel hold UTC, because the application time zone is `UTC` (research: SYNTHESIS §4.1). The importer takes the cut-off as the parameter `canteen_audit_local_before`, which defaults to `2022-01-21T00:00:00Z`, the end of the backend2 fix window and the cut-off that the post-date repair also uses. Values before it are read as Budapest wall time, values at or after it as UTC. The first import rehearsal compares the hour-of-day distribution of `canteen_data.created_at` before and after the cut-off and confirms or moves it. The error is at most two hours on admin-only metadata.

## Alternatives considered

| Option | Why not |
|---|---|
| A. Split by meaning: instants as `timestamptz`, event times as `timestamp without time zone` documented as Budapest wall time, post and canteen dates as `date` | The recommended default. The maintainer chose B instead (user override): event times and the two calendar-date columns are stored as `timestamptz` as well. |
| C. Mirror MySQL: naive timestamps everywhere, no conversion, the convention only documented | Keeps the ambiguity that produced the off-by-one post dates, and fights Loco's `timestamptz` default. |

## References

- research: DECISIONS 9 (and 21 for the default post date)
- research: SYNTHESIS §4.1, §4.4.2, §4.6.2, §4.10 R3-R5, §5.6
- research: gap-03 §4, §6.16, §8.2
- research: QUESTIONS Q9
- website_backend2@7ba71da: `migrations/03-canteen_data_add_time.sql`, `migrations/11-posts_data_fix_types.sql`
- loco-rs@1.2.0: `src/schema.rs`
- [data-migration.md](../data-migration.md)
- ADR 0002, ADR 0003, ADR 0006, ADR 0008, ADR 0009, ADR 0011, ADR 0017, ADR 0018
