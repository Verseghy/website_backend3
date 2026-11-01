//! The tz-data parity test: PostgreSQL and backend3 agree on every Budapest
//! midnight (ADR 0041 §2, amending ADR 0010).
//!
//! A calendar day is stored as the instant of its Budapest midnight (ADR 0010
//! point 2), and two tz databases look at that instant. backend3 computes it
//! with the tz data that chrono-tz compiles in (`budapest::TZ_DATA_VERSION`).
//! PostgreSQL checks it with its own: the `CHECK ((date AT TIME ZONE
//! 'Europe/Budapest')::time = time '00:00')` of ADR 0010 point 2, and SQL such
//! as the canteen days' unique index derives the day with `(date AT TIME ZONE
//! 'Europe/Budapest')::date`. PostgreSQL reads the tz data of its image, which
//! moves on another schedule than backend3's. Where the two disagree, a write
//! of that day is refused, or a stored value reads back as no day.
//!
//! The test finds the midnights of every day from 0001-01-01, the first day
//! the wire forms carry (`date_text::FIRST_YEAR`), to the end of the last
//! year whose clock changes chrono-tz lists
//! (`budapest::CHRONO_TZ_LAST_CHANGE_YEAR`), on both sides, and fails on any
//! day where they differ. backend3 stores no later day
//! (`budapest::date_to_stored_midnight`), so the range covers every day that a
//! calendar-day column can hold. Both sides find a day's midnights the same
//! way:
//!
//! - Every UTC offset that Budapest has used lies between `+01:00` and
//!   `+02:00` (local mean time, until 1890, was `+01:16:20`), so its midnight
//!   lies between two hours and one hour before 00:00 UTC of the day.
//! - The offset is read three hours before and one hour after 00:00 UTC
//!   ([`PROBE_BEFORE`], [`PROBE_AFTER`]). Each probed offset `o` gives the
//!   candidate instant `00:00 UTC - o`, which is a midnight of the day when
//!   its Budapest wall time is 00:00:00 on that day.
//! - That finds every midnight unless the offset changes twice within those
//!   four hours, which the tz data never does for Budapest: its changes lie
//!   months apart. On backend3's side the test also checks, for every day,
//!   that the first midnight found is the one `budapest::date_to_midnight`
//!   writes, and that a day with none is a day it refuses.
//!
//! backend3's side goes through `src/domain/budapest.rs`, the module that
//! every conversion uses, never through chrono-tz directly. Its midnights
//! arrive in PostgreSQL as runs of consecutive days with the same midnights,
//! about three hundred of them, and one query computes PostgreSQL's
//! midnights for each day with the expressions that the check and the index
//! use, and returns only the days that differ. A failure lists them; the
//! tz-data procedure of ADR 0041 §3 says what to do next.
//!
//! The parity test only compares the two sides with each other, so a change
//! that moves a midnight on both sides at once, such as one PR that moves
//! chrono-tz and compose.yaml's image together, would pass it. The same runs
//! are therefore also pinned in the committed snapshot
//! `snapshots/budapest_midnights.snap`: any chrono-tz change that moves a
//! midnight fails [`backend3_keeps_the_recorded_budapest_midnight_of_every_day`]
//! with the changed runs in the diff, old and new, whatever PostgreSQL does,
//! and the parity test carries the snapshot over to PostgreSQL. Accept a new
//! snapshot (`cargo insta review`) only in a PR that follows ADR 0041 §3 for
//! the days it moves. CI sets `INSTA_UPDATE=no`, so there a mismatch only
//! fails.

use std::fmt::Write as _;

use backend3::domain::{
    budapest::{self, BudapestTimeError},
    date_text,
};
use chrono::{DateTime, NaiveDate, NaiveTime, TimeDelta, Utc};
use sea_orm::{ConnectionTrait, DatabaseConnection, DbBackend, Statement, Value};
use serde_json::json;

use crate::support::with_app;

/// How long before 00:00 UTC of a day the first offset is read.
const PROBE_BEFORE: TimeDelta = TimeDelta::hours(3);

/// How long after 00:00 UTC of a day the second offset is read.
const PROBE_AFTER: TimeDelta = TimeDelta::hours(1);

/// The day that the day numbers of the query count from.
const EPOCH: NaiveDate = NaiveDate::from_ymd_opt(1970, 1, 1).expect("1970-01-01");

/// The most differing days a failure lists; the count covers all of them.
const LISTED_DIFFERENCES: i64 = 20;

/// The first day compared: the first day that the wire forms carry.
fn first_day() -> NaiveDate {
    NaiveDate::from_ymd_opt(date_text::FIRST_YEAR, 1, 1).expect("the first day of the wire forms")
}

/// The last day compared: the end of the last year whose Budapest clock
/// changes chrono-tz lists.
fn last_day() -> NaiveDate {
    NaiveDate::from_ymd_opt(budapest::CHRONO_TZ_LAST_CHANGE_YEAR, 12, 31)
        .expect("the end of chrono-tz's table")
}

/// The Budapest midnights of one day, each as the UTC offset in force at it
/// in seconds, in time order. A day has none, one or two; of two, the earlier
/// has the larger offset.
type Midnights = Vec<i32>;

/// Consecutive days that have the same midnights, as offsets: the days from
/// `first_day` to `last_day`, both included.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Run {
    first_day: NaiveDate,
    last_day: NaiveDate,
    midnights: Midnights,
}

/// 00:00 UTC of `day`, the instant from which the probes and the candidate
/// midnights are counted.
fn utc_midnight(day: NaiveDate) -> DateTime<Utc> {
    day.and_time(NaiveTime::MIN).and_utc()
}

/// The instant of a midnight of `day` that is `offset` seconds east of UTC.
fn instant_of(day: NaiveDate, offset: i32) -> DateTime<Utc> {
    utc_midnight(day) - TimeDelta::seconds(i64::from(offset))
}

/// backend3's UTC offset of Budapest at `instant`, in seconds, read through
/// the conversion module.
fn backend3_offset(instant: DateTime<Utc>) -> i32 {
    let wall = budapest::instant_to_wall(instant)
        .unwrap_or_else(|error| panic!("the Budapest wall time of {instant}: {error}"));
    let seconds = (wall - instant.naive_utc()).num_seconds();
    i32::try_from(seconds).expect("an offset of less than a day")
}

/// backend3's Budapest midnights of `day`.
///
/// # Panics
///
/// When the first midnight found is not the one that
/// `budapest::date_to_midnight` writes, or the module refuses a day that has
/// a midnight: the probes would then not speak for the module.
fn backend3_midnights(day: NaiveDate) -> Midnights {
    let at = utc_midnight(day);
    let [before, after] = [at - PROBE_BEFORE, at + PROBE_AFTER].map(backend3_offset);
    // The larger offset gives the earlier instant, so this is time order.
    let candidates = if before == after {
        vec![before]
    } else {
        vec![before.max(after), before.min(after)]
    };
    let midnights: Midnights = candidates
        .into_iter()
        .filter(|&offset| budapest::midnight_to_date(instant_of(day, offset)) == Ok(day))
        .collect();
    let written = budapest::date_to_midnight(day);
    match (written, midnights.first()) {
        (Ok(instant), Some(&offset)) if instant == instant_of(day, offset) => {}
        (Err(BudapestTimeError::NoMidnight { .. }), None) => {}
        (written, _) => panic!(
            "{day}: date_to_midnight gives {written:?}, the probes find {midnights:?}; widen \
             PROBE_BEFORE and PROBE_AFTER if Budapest has an offset outside +01:00 to +02:00"
        ),
    }
    midnights
}

/// backend3's midnights of every day from `first` to `last`, as runs.
fn backend3_runs(first: NaiveDate, last: NaiveDate) -> Vec<Run> {
    let mut runs: Vec<Run> = Vec::new();
    for day in first.iter_days().take_while(|&day| day <= last) {
        let midnights = backend3_midnights(day);
        match runs.last_mut() {
            Some(run) if run.midnights == midnights => run.last_day = day,
            _ => runs.push(Run {
                first_day: day,
                last_day: day,
                midnights,
            }),
        }
    }
    runs
}

/// The day number of `day` in the query: days since 1970-01-01.
fn day_number(day: NaiveDate) -> i32 {
    i32::try_from((day - EPOCH).num_days()).expect("a day number that fits an integer")
}

/// PostgreSQL's midnights of every day of the runs in `$1` (a JSON array of
/// `{"first_day", "last_day", "midnights"}`, day numbers counted from
/// 1970-01-01), compared with the runs' own. It returns the first
/// [`LISTED_DIFFERENCES`] days that differ, with both sides' midnights and
/// the number of all such days.
///
/// It reads PostgreSQL's offset as the wall time minus the instant, both in
/// seconds since the epoch: `to_timestamp` and `date_part` on a `timestamp`
/// do not depend on the session's `TimeZone`, so neither does the result.
/// The midnight test is the check of ADR 0010 point 2 and the day that
/// `(date AT TIME ZONE 'Europe/Budapest')::date` derives. The CTEs are
/// `MATERIALIZED` because PostgreSQL would otherwise inline them and work
/// out each zone conversion again at every reference (`days` holds none).
const POSTGRESQL_DIFFERENCES: &str = "
WITH runs AS (
    SELECT * FROM jsonb_to_recordset($1::jsonb) AS run(first_day int, last_day int, midnights int[])
), days AS (
    SELECT day, midnights AS backend3, day::bigint * 86400 AS utc_midnight
    FROM runs, generate_series(first_day, last_day) AS day
), probes AS MATERIALIZED (
    SELECT day, backend3, utc_midnight,
        (date_part('epoch', to_timestamp(utc_midnight - $2) AT TIME ZONE 'Europe/Budapest')
            - (utc_midnight - $2))::int AS before,
        (date_part('epoch', to_timestamp(utc_midnight + $3) AT TIME ZONE 'Europe/Budapest')
            - (utc_midnight + $3))::int AS after
    FROM days
), walls AS MATERIALIZED (
    SELECT day, backend3, before, after,
        to_timestamp(utc_midnight - before) AT TIME ZONE 'Europe/Budapest' AS wall_before,
        CASE WHEN after <> before
            THEN to_timestamp(utc_midnight - after) AT TIME ZONE 'Europe/Budapest' END AS wall_after
    FROM probes
), checked AS (
    SELECT day, backend3, before, after,
        wall_before::time = time '00:00' AND wall_before::date = date '1970-01-01' + day
            AS before_is_midnight,
        coalesce(wall_after::time = time '00:00' AND wall_after::date = date '1970-01-01' + day,
            false) AS after_is_midnight
    FROM walls
), compared AS (
    SELECT day, backend3,
        CASE
            WHEN before_is_midnight AND after_is_midnight
                THEN ARRAY[greatest(before, after), least(before, after)]
            WHEN before_is_midnight THEN ARRAY[before]
            WHEN after_is_midnight THEN ARRAY[after]
            ELSE '{}'::int[]
        END AS postgresql
    FROM checked
)
SELECT date '1970-01-01' + day AS day, postgresql, backend3, count(*) OVER () AS total
FROM compared
WHERE postgresql IS DISTINCT FROM backend3
ORDER BY day
LIMIT $4";

/// A day on which PostgreSQL's midnights differ from backend3's.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Difference {
    day: NaiveDate,
    postgresql: Midnights,
    backend3: Midnights,
}

/// Runs [`POSTGRESQL_DIFFERENCES`] over `runs` and returns the first listed
/// differing days with the number of all of them.
async fn postgresql_differences(db: &DatabaseConnection, runs: &[Run]) -> (i64, Vec<Difference>) {
    let runs = json!(
        runs.iter()
            .map(|run| json!({
                "first_day": day_number(run.first_day),
                "last_day": day_number(run.last_day),
                "midnights": run.midnights,
            }))
            .collect::<Vec<_>>()
    );
    let values: Vec<Value> = vec![
        runs.to_string().into(),
        PROBE_BEFORE.num_seconds().into(),
        PROBE_AFTER.num_seconds().into(),
        LISTED_DIFFERENCES.into(),
    ];
    let statement =
        Statement::from_sql_and_values(DbBackend::Postgres, POSTGRESQL_DIFFERENCES, values);
    let rows = db
        .query_all_raw(statement)
        .await
        .unwrap_or_else(|error| panic!("PostgreSQL's Budapest midnights: {error}"));
    let total = rows.first().map_or(0, |row| {
        row.try_get_by::<i64, _>("total")
            .expect("the count of differing days")
    });
    let listed = rows
        .iter()
        .map(|row| Difference {
            day: row.try_get_by("day").expect("the differing day"),
            postgresql: row
                .try_get_by("postgresql")
                .expect("PostgreSQL's midnights"),
            backend3: row.try_get_by("backend3").expect("backend3's midnights"),
        })
        .collect();
    (total, listed)
}

/// The midnights of `day` as instants, for a failure message.
fn instants(day: NaiveDate, midnights: &[i32]) -> String {
    match midnights {
        [] => "no midnight".to_owned(),
        _ => midnights
            .iter()
            .map(|&offset| instant_of(day, offset).to_rfc3339())
            .collect::<Vec<_>>()
            .join(" and "),
    }
}

/// The version of PostgreSQL's tz data, such as `2026c`, for a failure
/// message, or why it is unknown.
///
/// The official image builds PostgreSQL with `--with-system-tzdata`, so it
/// reads the image's `/usr/share/zoneinfo`, whose `tzdata.zi` starts with the
/// version line. `pg_read_file` on an absolute path needs the superuser role,
/// which compose's `loco` has as the image's bootstrap user, or membership in
/// `pg_read_server_files`; without either, the message says "unknown" with
/// PostgreSQL's reason, and [`the_comparison_reports_each_day_that_differs`]
/// fails, because it checks that the version can be read.
async fn postgresql_tz_data(db: &DatabaseConnection) -> String {
    let sql = "SELECT coalesce(substring(pg_read_file('/usr/share/zoneinfo/tzdata.zi', 0, 64) \
               FROM '^# version (\\S+)'), 'unknown (no version line in tzdata.zi)')";
    match db
        .query_one_raw(Statement::from_string(DbBackend::Postgres, sql))
        .await
    {
        Ok(Some(row)) => row
            .try_get_by_index::<String>(0)
            .unwrap_or_else(|error| format!("unknown ({error})")),
        Ok(None) => "unknown (no row)".to_owned(),
        Err(error) => format!("unknown ({error})"),
    }
}

#[tokio::test]
async fn postgresql_and_backend3_agree_on_every_budapest_midnight() {
    let runs = backend3_runs(first_day(), last_day());
    with_app(|_router, ctx| async move {
        let (total, listed) = postgresql_differences(&ctx.db, &runs).await;
        if total == 0 {
            return;
        }
        let days: Vec<String> = listed
            .iter()
            .map(|difference| {
                let Difference {
                    day,
                    postgresql,
                    backend3,
                } = difference;
                format!(
                    "  {day}: PostgreSQL {}; backend3 {}",
                    instants(*day, postgresql),
                    instants(*day, backend3)
                )
            })
            .collect();
        panic!(
            "PostgreSQL's and backend3's Budapest midnights differ on {total} days from {} to {}; \
             the first {}:\n{}\nPostgreSQL's tz data: {}; backend3's (chrono-tz): {}. Follow the \
             tz-data procedure of ADR 0041 §3: the change that brought one side's tz data waits \
             until both sides agree.",
            first_day(),
            last_day(),
            listed.len(),
            days.join("\n"),
            postgresql_tz_data(&ctx.db).await,
            budapest::TZ_DATA_VERSION,
        );
    })
    .await;
}

#[tokio::test]
async fn the_comparison_reports_each_day_that_differs() {
    // backend3's runs of the summer of 2026, with two days changed: 2026-07-01
    // claims winter time and 2026-07-02 claims no midnight.
    let summer = backend3_runs(date(2026, 6, 30), date(2026, 7, 3));
    assert_eq!(summer, [run((2026, 6, 30), (2026, 7, 3), &[7_200])]);
    let changed = [
        run((2026, 6, 30), (2026, 6, 30), &[7_200]),
        run((2026, 7, 1), (2026, 7, 1), &[3_600]),
        run((2026, 7, 2), (2026, 7, 2), &[]),
        run((2026, 7, 3), (2026, 7, 3), &[7_200]),
    ];
    with_app(|_router, ctx| async move {
        assert_eq!(postgresql_differences(&ctx.db, &summer).await, (0, vec![]));
        let expected = vec![
            Difference {
                day: date(2026, 7, 1),
                postgresql: vec![7_200],
                backend3: vec![3_600],
            },
            Difference {
                day: date(2026, 7, 2),
                postgresql: vec![7_200],
                backend3: vec![],
            },
        ];
        assert_eq!(
            postgresql_differences(&ctx.db, &changed).await,
            (2, expected)
        );
        // More differing days than a failure lists: the count still covers
        // all of them (it is taken before the LIMIT), and the listed ones are
        // the earliest. All of July 2026 claims winter time.
        let july = [run((2026, 7, 1), (2026, 7, 31), &[3_600])];
        let (total, listed) = postgresql_differences(&ctx.db, &july).await;
        assert_eq!(total, 31);
        let listed_days: Vec<NaiveDate> = listed.iter().map(|difference| difference.day).collect();
        let first_days: Vec<NaiveDate> = date(2026, 7, 1)
            .iter_days()
            .take(usize::try_from(LISTED_DIFFERENCES).expect("a small count"))
            .collect();
        assert_eq!(listed_days, first_days);
        assert!(
            listed.iter().all(
                |difference| difference.postgresql == [7_200] && difference.backend3 == [3_600]
            ),
            "{listed:?}"
        );
        // The failure message names PostgreSQL's tz data version, such as 2026c.
        let version = postgresql_tz_data(&ctx.db).await;
        let year = version
            .get(..4)
            .filter(|year| year.bytes().all(|b| b.is_ascii_digit()));
        let letters = version.get(4..).filter(|rest| !rest.is_empty());
        let letters = letters.filter(|rest| rest.bytes().all(|b| b.is_ascii_lowercase()));
        assert!(
            year.is_some() && letters.is_some(),
            "PostgreSQL's tz data version is {version}: reading the image's tzdata.zi needs a \
             superuser or a member of pg_read_server_files, as compose.yaml's loco is"
        );
    })
    .await;
}

#[tokio::test]
async fn postgresql_finds_no_midnight_and_two_midnights_as_backend3_does() {
    // 1981-03-29 has no Budapest midnight, 1981-09-27 two (ADR 0010 point 2),
    // and the first day of the wire forms is in local mean time.
    let cases = [
        (date(1981, 3, 28), date(1981, 3, 30)),
        (date(1981, 9, 26), date(1981, 9, 28)),
        (first_day(), first_day()),
    ];
    let runs: Vec<Run> = cases
        .iter()
        .flat_map(|&(first, last)| backend3_runs(first, last))
        .collect();
    assert_eq!(
        runs,
        [
            run((1981, 3, 28), (1981, 3, 28), &[3_600]),
            run((1981, 3, 29), (1981, 3, 29), &[]),
            run((1981, 3, 30), (1981, 3, 30), &[7_200]),
            run((1981, 9, 26), (1981, 9, 26), &[7_200]),
            run((1981, 9, 27), (1981, 9, 27), &[7_200, 3_600]),
            run((1981, 9, 28), (1981, 9, 28), &[3_600]),
            run((1, 1, 1), (1, 1, 1), &[4_580]),
        ]
    );
    with_app(|_router, ctx| async move {
        assert_eq!(postgresql_differences(&ctx.db, &runs).await, (0, vec![]));
    })
    .await;
}

/// The header of the committed snapshot, for whoever meets it in a diff.
const SNAPSHOT_DESCRIPTION: &str = "backend3's Budapest midnights (src/domain/budapest.rs, with \
     chrono-tz's tz data) of every day from 0001-01-01 to the end of \
     budapest::CHRONO_TZ_LAST_CHANGE_YEAR: runs of consecutive days, each with the UTC offset of \
     every midnight of its days in time order, or none (ADR 0041 §2). A change here moves stored \
     calendar days: accept it only in a PR that follows ADR 0041 §3.";

/// A UTC offset in seconds as `+HH:MM`, or as `+HH:MM:SS` when it has
/// seconds, as local mean time does.
fn offset_text(offset: i32) -> String {
    let sign = if offset < 0 { '-' } else { '+' };
    let total = offset.unsigned_abs();
    let (hours, minutes, seconds) = (total / 3_600, total / 60 % 60, total % 60);
    if seconds == 0 {
        format!("{sign}{hours:02}:{minutes:02}")
    } else {
        format!("{sign}{hours:02}:{minutes:02}:{seconds:02}")
    }
}

/// The snapshot text of `runs`: one line per run, its day (or its first and
/// last day), then the offset of each midnight, or `none`.
fn snapshot_text(runs: &[Run]) -> String {
    let mut text = String::new();
    for run in runs {
        let days = if run.first_day == run.last_day {
            run.first_day.to_string()
        } else {
            format!("{} to {}", run.first_day, run.last_day)
        };
        let midnights = if run.midnights.is_empty() {
            "none".to_owned()
        } else {
            run.midnights
                .iter()
                .map(|&offset| offset_text(offset))
                .collect::<Vec<_>>()
                .join(" and ")
        };
        writeln!(text, "{days}: {midnights}").expect("writing to a String");
    }
    text
}

#[test]
fn the_snapshot_text_names_each_run_and_its_offsets() {
    assert_eq!(offset_text(7_200), "+02:00");
    assert_eq!(offset_text(4_580), "+01:16:20");
    assert_eq!(offset_text(-1_800), "-00:30");
    let runs = [
        run((1981, 3, 28), (1981, 3, 28), &[3_600]),
        run((1981, 3, 29), (1981, 3, 29), &[]),
        run((1981, 3, 30), (1981, 9, 26), &[7_200]),
        run((1981, 9, 27), (1981, 9, 27), &[7_200, 3_600]),
    ];
    assert_eq!(
        snapshot_text(&runs),
        "1981-03-28: +01:00\n1981-03-29: none\n1981-03-30 to 1981-09-26: +02:00\n\
         1981-09-27: +02:00 and +01:00\n"
    );
}

#[test]
fn backend3_keeps_the_recorded_budapest_midnight_of_every_day() {
    // One pass over the range serves every assertion here: it is the
    // costliest part of the module, and nextest runs each test in its own
    // process, so a second test could not share it.
    let runs = backend3_runs(first_day(), last_day());
    assert!(
        runs.windows(2)
            .all(|pair| pair[0].last_day.succ_opt() == Some(pair[1].first_day)),
        "the runs cover the range without a gap"
    );
    assert_eq!(runs.first().map(|run| run.first_day), Some(first_day()));
    assert_eq!(runs.last().map(|run| run.last_day), Some(last_day()));
    // A midnight that moves, on any day, changes this text. The file name is
    // plain: insta would otherwise prefix it with the module path of the
    // `mod` test crate.
    insta::with_settings!({
        prepend_module_to_snapshot => false,
        description => SNAPSHOT_DESCRIPTION,
        omit_expression => true,
    }, {
        insta::assert_snapshot!("budapest_midnights", snapshot_text(&runs));
    });
    // ADR 0010 point 2: since 1984 the clocks change at 02:00 and 03:00, so
    // every day from 1984 on has exactly one midnight, and the days with none
    // or two are these eleven of 1916 to 1983. Pinned on their own as well,
    // so that a snapshot accepted in a hurry still cannot lose or add one.
    let exceptions: Vec<&Run> = runs.iter().filter(|run| run.midnights.len() != 1).collect();
    assert_eq!(
        exceptions,
        [
            &run((1916, 10, 1), (1916, 10, 1), &[7_200, 3_600]),
            &run((1945, 11, 1), (1945, 11, 1), &[7_200, 3_600]),
            &run((1954, 5, 23), (1954, 5, 23), &[]),
            &run((1980, 4, 6), (1980, 4, 6), &[]),
            &run((1980, 9, 28), (1980, 9, 28), &[7_200, 3_600]),
            &run((1981, 3, 29), (1981, 3, 29), &[]),
            &run((1981, 9, 27), (1981, 9, 27), &[7_200, 3_600]),
            &run((1982, 3, 28), (1982, 3, 28), &[]),
            &run((1982, 9, 26), (1982, 9, 26), &[7_200, 3_600]),
            &run((1983, 3, 27), (1983, 3, 27), &[]),
            &run((1983, 9, 25), (1983, 9, 25), &[7_200, 3_600]),
        ],
        "the days with no Budapest midnight or two are no longer those of ADR 0010 point 2: \
         follow the tz-data procedure of ADR 0041 §3"
    );
}

/// A calendar day of the tests.
fn date(year: i32, month: u32, day: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(year, month, day).expect("a test day")
}

/// A run of the tests, from `(year, month, day)` to `(year, month, day)`.
fn run(first: (i32, u32, u32), last: (i32, u32, u32), midnights: &[i32]) -> Run {
    Run {
        first_day: date(first.0, first.1, first.2),
        last_day: date(last.0, last.1, last.2),
        midnights: midnights.to_vec(),
    }
}
