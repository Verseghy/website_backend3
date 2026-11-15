//! The pure parts of the import's source session (docs/design/data-migration.md
//! §3.3, §4.3): the statements it sends, which legacy columns hold the
//! instants that `snapshot_at` must not precede, and the checks over what it
//! reads.
//!
//! The session itself, in `tasks::import_mysql::session`, runs on one
//! connection to the legacy MySQL source:
//!
//! 1. [`SESSION_SETTINGS`]: `time_zone` `+00:00`, so `TIMESTAMP` columns come
//!    back in UTC (§5.2); `NAMES utf8mb4`, so every string arrives as UTF-8
//!    whatever the URL's `charset` says; and `information_schema_stats_expiry`
//!    0, so `information_schema.TABLES.AUTO_INCREMENT` is current, not a
//!    cached value up to a day old.
//! 2. The schema fingerprint ([`fingerprint_statement`], compared by
//!    `domain::import_mysql::fingerprint`), once before the lock, so that a
//!    missing table is named instead of failing `LOCK TABLES` with an error
//!    number, and once under it, where it is authoritative.
//! 3. [`lock_statement`]: `LOCK TABLES <table> READ, ...` over the tables
//!    of the source allow-list, so that even a writable source cannot change
//!    while the session reads it.
//! 4. Every table's `AUTO_INCREMENT` ([`auto_increment_statement`]), which
//!    the steps use to set the identity sequences so that no id MySQL issued
//!    is issued again (§5.1).
//! 5. The latest write instant of every table that records them
//!    ([`SOURCE_TIMESTAMPS`], [`latest_statement`], each answer read by
//!    [`timestamps_of`]), and the refusal of a `snapshot_at` earlier than
//!    the latest one ([`check_snapshot_at`], §4.1), which
//!    [`SourceFacts::checked`] runs: the session's facts exist only once it
//!    has passed.
//! 6. The import steps' reads (`tasks::import_mysql::source::select_rows`).
//! 7. [`UNLOCK`].
//!
//! Every statement that names a legacy table names it from
//! `source_tables::SOURCE_TABLES` (the lock, the fingerprint and the
//! `AUTO_INCREMENT` read) or checks it against that list
//! ([`latest_statement`]), so `newsletter`, `password_resets` and
//! `migrations` are never locked, described or read (NWS-01, §9.1). A
//! `LOCK TABLES` session can read only the tables it locked, so the lock
//! itself also keeps every other table unread until [`UNLOCK`];
//! `information_schema` is the one exception MySQL makes, and the
//! fingerprint and `AUTO_INCREMENT` statements ask it only about the
//! allow-listed tables.

use std::fmt::Write as _;

use chrono::{DateTime, NaiveDateTime, TimeDelta, Utc};

use super::{
    options::{CanteenAuditLocalBefore, format_instant},
    source_tables::{SOURCE_TABLES, SourceTable, TableRefusal},
};
use crate::domain::budapest;

/// The session settings of data-migration.md §3.3, sent first on the
/// session's connection. They override what the driver set at connect from
/// the URL's `timezone` and `charset` parameters.
pub const SESSION_SETTINGS: &str =
    "SET time_zone = '+00:00', NAMES utf8mb4, information_schema_stats_expiry = 0";

/// Ends the session's table locks.
pub const UNLOCK: &str = "UNLOCK TABLES";

/// The allow-listed table names as a SQL list of string literals:
/// `'posts_data', 'posts_labels', ...`. Every name is a plain identifier of
/// letters and underscores, so none needs escaping inside quotes.
fn quoted_table_list() -> String {
    SOURCE_TABLES
        .iter()
        .map(|table| format!("'{}'", table.name()))
        .collect::<Vec<_>>()
        .join(", ")
}

/// `LOCK TABLES` over every table of the source allow-list, for reading.
///
/// One statement takes every lock at once, as MySQL requires: a second
/// `LOCK TABLES` would first release the locks of the first.
#[must_use]
pub fn lock_statement() -> String {
    let mut sql = String::from("LOCK TABLES ");
    for (index, table) in SOURCE_TABLES.iter().enumerate() {
        if index > 0 {
            sql.push_str(", ");
        }
        let _ = write!(sql, "`{}` READ", table.name());
    }
    sql
}

/// The schema fingerprint's query: every column of the allow-listed tables
/// in the session's database, with its table, name, `COLUMN_TYPE` and
/// `IS_NULLABLE`, ordered by table and position.
///
/// The text columns of `information_schema` are cast to `CHAR`, because
/// MySQL 8.4 reports some of them with a binary collation that the driver
/// would not read as text.
#[must_use]
pub fn fingerprint_statement() -> String {
    format!(
        "SELECT CAST(TABLE_NAME AS CHAR), CAST(COLUMN_NAME AS CHAR), CAST(COLUMN_TYPE AS CHAR), \
         CAST(IS_NULLABLE AS CHAR) FROM information_schema.COLUMNS WHERE TABLE_SCHEMA = \
         DATABASE() AND TABLE_NAME IN ({}) ORDER BY TABLE_NAME, ORDINAL_POSITION",
        quoted_table_list()
    )
}

/// The query of every allow-listed table's `AUTO_INCREMENT`, the next id
/// MySQL would issue (`NULL` for a table without an auto-increment
/// column, as `InnoDB` reports it), ordered by table.
#[must_use]
pub fn auto_increment_statement() -> String {
    format!(
        "SELECT CAST(TABLE_NAME AS CHAR), AUTO_INCREMENT FROM information_schema.TABLES WHERE \
         TABLE_SCHEMA = DATABASE() AND TABLE_NAME IN ({}) ORDER BY TABLE_NAME",
        quoted_table_list()
    )
}

/// How a legacy column's values become instants (data-migration.md §5.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TimestampKind {
    /// A `TIMESTAMP`, read in the `+00:00` session, or a `DATETIME` that
    /// Laravel wrote in UTC: the value is the UTC instant.
    Utc,
    /// A `canteen_data` audit `DATETIME`: Budapest wall time before
    /// `canteen_audit_local_before`, UTC from then on.
    CanteenAudit,
}

/// The write-instant columns of one legacy table.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TimestampColumns {
    /// The legacy table, one of the source allow-list.
    pub table: &'static str,
    /// Its columns that record when a row was written.
    pub columns: &'static [&'static str],
    /// How their values become instants.
    pub kind: TimestampKind,
}

/// The columns that hold the source's timestamps, which `snapshot_at` must
/// not precede (data-migration.md §4.1): the instants at which a row was
/// created, changed or deleted, in every table of the allow-list that has
/// them, in the order of `SOURCE_TABLES`.
///
/// Columns that are not write instants are left out: the calendar days
/// `posts_data.date` and `canteen_data.date` (`DATE`), and the event times
/// `events_data.date_from` and `date_to`, which hold when an event takes
/// place and lie in the future for every upcoming event. A unit test of
/// `tasks::import_mysql::expected_schema` checks that every other
/// `DATETIME` or `TIMESTAMP` column of the expected DDL is listed here.
pub const SOURCE_TIMESTAMPS: [TimestampColumns; 9] = [
    TimestampColumns {
        table: "posts_data",
        columns: &["created_at", "updated_at"],
        kind: TimestampKind::Utc,
    },
    TimestampColumns {
        table: "canteen_menus",
        columns: &["created_at", "updated_at"],
        kind: TimestampKind::Utc,
    },
    TimestampColumns {
        table: "canteen_data",
        columns: &["created_at", "updated_at"],
        kind: TimestampKind::CanteenAudit,
    },
    TimestampColumns {
        table: "events_data",
        columns: &["created_at", "updated_at"],
        kind: TimestampKind::Utc,
    },
    TimestampColumns {
        table: "pages",
        columns: &["created_at", "updated_at", "deleted_at"],
        kind: TimestampKind::Utc,
    },
    TimestampColumns {
        table: "menu_items",
        columns: &["created_at", "updated_at", "deleted_at"],
        kind: TimestampKind::Utc,
    },
    TimestampColumns {
        table: "users",
        columns: &["created_at", "updated_at"],
        kind: TimestampKind::Utc,
    },
    TimestampColumns {
        table: "roles",
        columns: &["created_at", "updated_at"],
        kind: TimestampKind::Utc,
    },
    TimestampColumns {
        table: "permissions",
        columns: &["created_at", "updated_at"],
        kind: TimestampKind::Utc,
    },
];

/// The query of the latest value of each of `columns.columns`: one
/// `SELECT` of the `MAX` of every column, from the table.
///
/// - `columns`: one entry of [`SOURCE_TIMESTAMPS`].
///
/// One statement per table, because a `LOCK TABLES` session may name a
/// locked table only once in a statement, which rules out a `UNION` over
/// one table's columns.
///
/// # Errors
///
/// The allow-list's refusal of a table that is not on it.
pub fn latest_statement(columns: &TimestampColumns) -> Result<String, TableRefusal> {
    let table = SourceTable::from_name(columns.table)?;
    let maxima = columns
        .columns
        .iter()
        .map(|column| format!("MAX(`{column}`)"))
        .collect::<Vec<_>>()
        .join(", ");
    Ok(format!("SELECT {maxima} FROM `{}`", table.name()))
}

/// The instant that a legacy write-instant value stands for.
///
/// - `value`: the value as the `+00:00` session returns it.
/// - `kind`: how the column's values become instants.
/// - `canteen_audit_local_before`: the run's cut-off for `canteen_data`
///   (§5.2); used only for [`TimestampKind::CanteenAudit`].
///
/// A canteen audit value counts as Budapest wall time when, read as UTC, it
/// lies before the cut-off (or for every value with `all`); the cut-off
/// falls in January 2022, between the last value that backend2's fix 03
/// shifted and the first that Laravel wrote in UTC, so the reading of a
/// value near it never decides between the two.
///
/// Such a value is the instant that the Budapest clock showed it at, through
/// the `budapest` module (ADR 0010 point 5). Where a clock change leaves two
/// readings, this check takes the later one, `+01:00`, so that it errs
/// towards refusing `snapshot_at`: a value in the repeated autumn hour
/// counts at its second instant, and one in the skipped spring hour, which
/// no instant shows, at the offset in force before the change. That is a
/// reading for this check only. The step that imports `canteen_data`
/// applies the §5.2 rule to every row, and `budapest::wall_to_instant`
/// gives the earlier instant of a repeated hour (ADR 0010 point 6).
#[must_use]
pub fn source_instant(
    value: NaiveDateTime,
    kind: TimestampKind,
    canteen_audit_local_before: CanteenAuditLocalBefore,
) -> DateTime<Utc> {
    let as_utc = value.and_utc();
    let local = match (kind, canteen_audit_local_before) {
        (TimestampKind::Utc, _) => false,
        (TimestampKind::CanteenAudit, CanteenAuditLocalBefore::All) => true,
        (TimestampKind::CanteenAudit, CanteenAuditLocalBefore::Before(cutoff)) => as_utc < cutoff,
    };
    if !local {
        return as_utc;
    }
    match budapest::wall_to_instant(value) {
        Ok(earliest) => {
            // In the repeated autumn hour the clock shows the value again
            // one hour later, at `+01:00`; elsewhere that instant shows
            // another wall time, and the one reading stands.
            let later = earliest + TimeDelta::hours(1);
            if budapest::instant_to_wall(later) == Ok(value) {
                later
            } else {
                earliest
            }
        }
        // The spring gap. MySQL's DATETIME holds no leap second, and its
        // range (years 1000 to 9999) lies far inside chrono's, so no other
        // error of the conversion can occur here.
        Err(_) => as_utc - TimeDelta::hours(1),
    }
}

/// A write instant of the source: the latest value of one column.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SourceTimestamp {
    /// The legacy table.
    pub table: &'static str,
    /// The column.
    pub column: &'static str,
    /// The latest value, as an instant ([`source_instant`]).
    pub at: DateTime<Utc>,
}

/// Why the latest values of one table's write-instant columns could not be
/// read: the answer holds another number of values than the statement of
/// [`latest_statement`] asked for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error(
    "reading the latest timestamps of `{table}` failed: the source answered {found} values for \
     its {expected} timestamp columns"
)]
pub struct MaximaCountMismatch {
    /// The legacy table.
    pub table: &'static str,
    /// How many columns the statement asked for.
    pub expected: usize,
    /// How many values the answer held.
    pub found: usize,
}

/// The write instants of one legacy table: each value of `maxima` that is
/// not `NULL`, as the instant it stands for ([`source_instant`], with the
/// table's [`TimestampKind`]), named by its column.
///
/// - `columns`: one entry of [`SOURCE_TIMESTAMPS`].
/// - `maxima`: the answer of [`latest_statement`] for `columns`: the latest
///   value of each column, in the order of `columns.columns`, `None` for a
///   column with no value.
/// - `canteen_audit_local_before`: the run's cut-off for the `canteen_data`
///   audit values (§5.2).
///
/// The session passes the whole entry, so each value is read with the kind
/// of its own table.
///
/// # Errors
///
/// [`MaximaCountMismatch`] when `maxima` does not hold one value per column.
pub fn timestamps_of(
    columns: &TimestampColumns,
    maxima: &[Option<NaiveDateTime>],
    canteen_audit_local_before: CanteenAuditLocalBefore,
) -> Result<Vec<SourceTimestamp>, MaximaCountMismatch> {
    if maxima.len() != columns.columns.len() {
        return Err(MaximaCountMismatch {
            table: columns.table,
            expected: columns.columns.len(),
            found: maxima.len(),
        });
    }
    Ok(columns
        .columns
        .iter()
        .zip(maxima)
        .filter_map(|(column, value)| {
            value.map(|value| SourceTimestamp {
                table: columns.table,
                column,
                at: source_instant(value, columns.kind, canteen_audit_local_before),
            })
        })
        .collect())
}

/// The latest of `timestamps`; for equal instants, the first given.
///
/// - `timestamps`: the latest value of each write-instant column that holds
///   one, in the order of [`SOURCE_TIMESTAMPS`].
#[must_use]
pub fn latest(timestamps: &[SourceTimestamp]) -> Option<SourceTimestamp> {
    timestamps
        .iter()
        .copied()
        .reduce(|latest, next| if next.at > latest.at { next } else { latest })
}

/// Why `snapshot_at` was refused: it lies before a write recorded in the
/// source, so it cannot be the moment the source was frozen or dumped.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error(
    "refusing to import: snapshot_at is {snapshot}, earlier than the latest timestamp in the \
     source, `{table}`.`{column}` at {latest} (data-migration.md §4.1). Pass the moment the \
     source was frozen or dumped, which is never before a write it holds",
    snapshot = format_instant(*snapshot_at),
    table = latest.table,
    column = latest.column,
    latest = format_instant(latest.at)
)]
pub struct SnapshotTooEarly {
    /// The run's `snapshot_at`.
    pub snapshot_at: DateTime<Utc>,
    /// The source's latest timestamp.
    pub latest: SourceTimestamp,
}

/// Refuses a `snapshot_at` earlier than the source's latest timestamp.
///
/// - `snapshot_at`: the run's argument.
/// - `latest`: the source's latest timestamp, or `None` when no
///   write-instant column holds a value (an empty source).
///
/// # Errors
///
/// [`SnapshotTooEarly`], naming both instants and the column.
pub fn check_snapshot_at(
    snapshot_at: DateTime<Utc>,
    latest: Option<SourceTimestamp>,
) -> Result<(), SnapshotTooEarly> {
    match latest {
        Some(latest) if snapshot_at < latest.at => Err(SnapshotTooEarly {
            snapshot_at,
            latest,
        }),
        _ => Ok(()),
    }
}

/// One allow-listed table's `AUTO_INCREMENT`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AutoIncrement {
    /// The legacy table.
    pub table: SourceTable,
    /// The next id MySQL would issue, or `None` when the table has no
    /// auto-increment column.
    pub next_id: Option<u64>,
}

/// Why the `AUTO_INCREMENT` answer was refused.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum AutoIncrementError {
    /// An allow-listed table is missing from the answer.
    #[error(
        "refusing to import: information_schema.TABLES lists no `{table}` in the source, so its \
         AUTO_INCREMENT cannot be read (data-migration.md §3.3)"
    )]
    Missing {
        /// The table's name.
        table: &'static str,
    },
    /// The answer names a table that was not asked for, or one twice.
    #[error(
        "refusing to import: information_schema.TABLES answered {table:?}, which the \
         AUTO_INCREMENT query did not ask for or listed twice"
    )]
    Unexpected {
        /// The name as answered.
        table: String,
    },
}

/// Every allow-listed table's `AUTO_INCREMENT`, in the order of
/// `SOURCE_TABLES`.
///
/// - `answer`: the rows of [`auto_increment_statement`]: table name and
///   value.
///
/// # Errors
///
/// [`AutoIncrementError`] for a table that is missing, unexpected or
/// listed twice.
pub fn auto_increments(
    answer: &[(String, Option<u64>)],
) -> Result<Vec<AutoIncrement>, AutoIncrementError> {
    for (index, (name, _)) in answer.iter().enumerate() {
        let listed = SOURCE_TABLES.iter().any(|table| table.name() == name);
        let repeated = answer[..index].iter().any(|(earlier, _)| earlier == name);
        if !listed || repeated {
            return Err(AutoIncrementError::Unexpected {
                table: name.clone(),
            });
        }
    }
    SOURCE_TABLES
        .iter()
        .map(|table| {
            answer
                .iter()
                .find(|(name, _)| name == table.name())
                .map(|(_, next_id)| AutoIncrement {
                    table: *table,
                    next_id: *next_id,
                })
                .ok_or(AutoIncrementError::Missing {
                    table: table.name(),
                })
        })
        .collect()
}

/// What the source session established, for the import steps and the run
/// report (data-migration.md §11.3): the fingerprint matched (a session
/// that ends has checked it), every table's `AUTO_INCREMENT`, and the
/// latest timestamp in the source, which `snapshot_at` does not precede.
///
/// The fields are private and [`SourceFacts::checked`] is the only way to
/// build one, so no code can hold the facts of a session without the check
/// of `snapshot_at` having passed (data-migration.md §4.1).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceFacts {
    auto_increments: Vec<AutoIncrement>,
    latest_timestamp: Option<SourceTimestamp>,
}

impl SourceFacts {
    /// The facts of a session, once `snapshot_at` is checked against the
    /// latest timestamp ([`check_snapshot_at`]).
    ///
    /// - `auto_increments`: every allow-listed table's `AUTO_INCREMENT`, in
    ///   the order of `SOURCE_TABLES` ([`auto_increments`]).
    /// - `latest_timestamp`: the latest timestamp in the source ([`latest`]),
    ///   or `None` when no write-instant column holds a value.
    /// - `snapshot_at`: the run's argument.
    ///
    /// # Errors
    ///
    /// [`SnapshotTooEarly`] when `snapshot_at` lies before
    /// `latest_timestamp`, naming both instants and the column.
    pub fn checked(
        auto_increments: Vec<AutoIncrement>,
        latest_timestamp: Option<SourceTimestamp>,
        snapshot_at: DateTime<Utc>,
    ) -> Result<Self, SnapshotTooEarly> {
        check_snapshot_at(snapshot_at, latest_timestamp)?;
        Ok(Self {
            auto_increments,
            latest_timestamp,
        })
    }

    /// Every allow-listed table's `AUTO_INCREMENT`, in the order of
    /// `SOURCE_TABLES`.
    #[must_use]
    pub fn auto_increments(&self) -> &[AutoIncrement] {
        &self.auto_increments
    }

    /// The latest timestamp in the source, or `None` when no write-instant
    /// column holds a value.
    #[must_use]
    pub fn latest_timestamp(&self) -> Option<SourceTimestamp> {
        self.latest_timestamp
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::import_mysql::not_migrated::NOT_MIGRATED_TABLES;
    use proptest::prelude::*;

    /// The tables of §9.1 that the import never reads.
    const LEFT_BEHIND: [&str; 3] = ["newsletter", "password_resets", "migrations"];

    fn instant(rfc3339: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(rfc3339)
            .expect("RFC 3339")
            .with_timezone(&Utc)
    }

    fn wall(text: &str) -> NaiveDateTime {
        NaiveDateTime::parse_from_str(text, "%Y-%m-%d %H:%M:%S").expect("a wall time")
    }

    /// Every statement the session sends that names a table.
    fn statements() -> Vec<String> {
        let mut statements = vec![
            SESSION_SETTINGS.to_owned(),
            lock_statement(),
            fingerprint_statement(),
            auto_increment_statement(),
            UNLOCK.to_owned(),
        ];
        statements.extend(
            SOURCE_TIMESTAMPS
                .iter()
                .map(|columns| latest_statement(columns).expect("allow-listed")),
        );
        statements
    }

    #[test]
    fn the_lock_covers_exactly_the_allow_list_for_reading() {
        let lock = lock_statement();
        assert!(lock.starts_with("LOCK TABLES `posts_data` READ, `posts_labels` READ, "));
        assert!(lock.ends_with(", `role_has_permissions` READ"), "{lock}");
        assert_eq!(lock.matches(" READ").count(), SOURCE_TABLES.len());
        for table in SOURCE_TABLES {
            assert!(lock.contains(&format!("`{}` READ", table.name())), "{lock}");
        }
        assert!(!lock.contains("WRITE"));
    }

    #[test]
    fn no_statement_names_a_table_the_import_never_reads() {
        for statement in statements() {
            for table in LEFT_BEHIND {
                assert!(!statement.contains(table), "{table} in {statement}");
            }
        }
        // §9.1 lists them with the verified tables, which are read.
        for row in NOT_MIGRATED_TABLES {
            if LEFT_BEHIND.contains(&row.table) {
                continue;
            }
            assert!(lock_statement().contains(&format!("`{}`", row.table)));
        }
    }

    #[test]
    fn the_information_schema_queries_ask_only_about_the_allow_list() {
        let list = quoted_table_list();
        assert_eq!(list.matches('\'').count(), 2 * SOURCE_TABLES.len());
        assert!(list.starts_with("'posts_data', 'posts_labels', "), "{list}");
        for statement in [fingerprint_statement(), auto_increment_statement()] {
            assert!(
                statement.contains(&format!(
                    "WHERE TABLE_SCHEMA = DATABASE() AND TABLE_NAME IN ({list})"
                )),
                "{statement}"
            );
        }
        assert!(fingerprint_statement().starts_with(
            "SELECT CAST(TABLE_NAME AS CHAR), CAST(COLUMN_NAME AS CHAR), CAST(COLUMN_TYPE AS \
             CHAR), CAST(IS_NULLABLE AS CHAR) FROM information_schema.COLUMNS "
        ));
        assert!(auto_increment_statement().ends_with(") ORDER BY TABLE_NAME"));
    }

    #[test]
    fn the_session_settings_are_those_of_section_3_3() {
        assert_eq!(
            SESSION_SETTINGS,
            "SET time_zone = '+00:00', NAMES utf8mb4, information_schema_stats_expiry = 0"
        );
        assert_eq!(UNLOCK, "UNLOCK TABLES");
    }

    #[test]
    fn the_timestamp_columns_are_on_the_allow_list_in_its_order() {
        let positions: Vec<usize> = SOURCE_TIMESTAMPS
            .iter()
            .map(|columns| {
                SOURCE_TABLES
                    .iter()
                    .position(|table| table.name() == columns.table)
                    .unwrap_or_else(|| panic!("{} is not allow-listed", columns.table))
            })
            .collect();
        assert!(positions.is_sorted(), "{positions:?}");
        assert_eq!(
            latest_statement(&SOURCE_TIMESTAMPS[4]).expect("pages"),
            "SELECT MAX(`created_at`), MAX(`updated_at`), MAX(`deleted_at`) FROM `pages`"
        );
        let canteen: Vec<&TimestampColumns> = SOURCE_TIMESTAMPS
            .iter()
            .filter(|columns| columns.kind == TimestampKind::CanteenAudit)
            .collect();
        assert_eq!(canteen.len(), 1);
        assert_eq!(canteen[0].table, "canteen_data");
    }

    #[test]
    fn a_latest_statement_of_a_table_off_the_list_is_refused() {
        for table in LEFT_BEHIND {
            let refusal = latest_statement(&TimestampColumns {
                table,
                columns: &["created_at"],
                kind: TimestampKind::Utc,
            })
            .expect_err(table);
            assert!(
                matches!(refusal, TableRefusal::LeftBehind { .. }),
                "{refusal:?}"
            );
        }
    }

    #[test]
    fn utc_values_are_their_own_instant() {
        let cutoff = CanteenAuditLocalBefore::default_cutoff();
        assert_eq!(
            source_instant(wall("2026-10-23 17:05:00"), TimestampKind::Utc, cutoff),
            instant("2026-10-23T17:05:00Z")
        );
        // A canteen value at or after the cut-off is UTC too.
        for value in ["2022-01-21 00:00:00", "2026-06-01 12:00:00"] {
            assert_eq!(
                source_instant(wall(value), TimestampKind::CanteenAudit, cutoff),
                wall(value).and_utc(),
                "{value}"
            );
        }
    }

    #[test]
    fn canteen_values_before_the_cutoff_are_budapest_wall_time() {
        let cutoff = CanteenAuditLocalBefore::default_cutoff();
        for (value, expected) in [
            // Winter (+01:00) and summer (+02:00).
            ("2021-12-01 08:00:00", "2021-12-01T07:00:00Z"),
            ("2021-06-01 08:00:00", "2021-06-01T06:00:00Z"),
            ("2022-01-20 23:59:59", "2022-01-20T22:59:59Z"),
            // Where a clock change leaves two readings, the later, +01:00:
            // the second instant of the repeated autumn hour, and the
            // offset before the change for the skipped spring hour.
            ("2021-10-31 02:00:00", "2021-10-31T01:00:00Z"),
            ("2021-10-31 02:30:00", "2021-10-31T01:30:00Z"),
            ("2021-10-31 02:59:59", "2021-10-31T01:59:59Z"),
            ("2021-03-28 02:30:00", "2021-03-28T01:30:00Z"),
            // Next to the changes, the one reading.
            ("2021-10-31 01:59:59", "2021-10-30T23:59:59Z"),
            ("2021-10-31 03:00:00", "2021-10-31T02:00:00Z"),
            ("2021-03-28 01:59:59", "2021-03-28T00:59:59Z"),
            ("2021-03-28 03:00:00", "2021-03-28T01:00:00Z"),
        ] {
            assert_eq!(
                source_instant(wall(value), TimestampKind::CanteenAudit, cutoff),
                instant(expected),
                "{value}"
            );
        }
        assert_eq!(
            source_instant(
                wall("2026-06-01 12:00:00"),
                TimestampKind::CanteenAudit,
                CanteenAuditLocalBefore::All
            ),
            instant("2026-06-01T10:00:00Z")
        );
    }

    fn timestamp(table: &'static str, column: &'static str, at: &str) -> SourceTimestamp {
        SourceTimestamp {
            table,
            column,
            at: instant(at),
        }
    }

    #[test]
    fn the_latest_timestamp_is_the_maximum_and_the_first_of_equals() {
        assert_eq!(latest(&[]), None);
        let first = timestamp("posts_data", "updated_at", "2026-10-23T17:00:00Z");
        let equal = timestamp("users", "updated_at", "2026-10-23T17:00:00Z");
        let earlier = timestamp("pages", "created_at", "2026-10-01T00:00:00Z");
        assert_eq!(latest(&[earlier, first, equal]), Some(first));
        assert_eq!(latest(&[earlier]), Some(earlier));
    }

    #[test]
    fn a_snapshot_before_the_latest_timestamp_is_refused_naming_the_column() {
        let latest = timestamp("posts_data", "updated_at", "2026-10-23T17:05:00Z");
        assert_eq!(check_snapshot_at(latest.at, Some(latest)), Ok(()));
        assert_eq!(
            check_snapshot_at(instant("2026-10-23T17:05:01Z"), Some(latest)),
            Ok(())
        );
        assert_eq!(
            check_snapshot_at(instant("2000-01-01T00:00:00Z"), None),
            Ok(())
        );
        let refusal = check_snapshot_at(instant("2026-10-23T17:01:02Z"), Some(latest))
            .expect_err("too early");
        assert_eq!(
            refusal.to_string(),
            "refusing to import: snapshot_at is 2026-10-23T17:01:02Z, earlier than the latest \
             timestamp in the source, `posts_data`.`updated_at` at 2026-10-23T17:05:00Z \
             (data-migration.md §4.1). Pass the moment the source was frozen or dumped, which \
             is never before a write it holds"
        );
    }

    /// The entry of [`SOURCE_TIMESTAMPS`] for `table`.
    fn timestamp_columns(table: &str) -> &'static TimestampColumns {
        SOURCE_TIMESTAMPS
            .iter()
            .find(|columns| columns.table == table)
            .expect("a table with timestamps")
    }

    #[test]
    fn each_value_of_an_answer_is_read_with_its_table_s_kind_and_named_by_its_column() {
        let cutoff = CanteenAuditLocalBefore::default_cutoff();
        let summer = wall("2021-06-01 08:00:00");
        // The same value: UTC in `posts_data`, Budapest wall time in
        // `canteen_data` before the cut-off.
        assert_eq!(
            timestamps_of(
                timestamp_columns("posts_data"),
                &[Some(summer), None],
                cutoff
            ),
            Ok(vec![timestamp(
                "posts_data",
                "created_at",
                "2021-06-01T08:00:00Z"
            )])
        );
        assert_eq!(
            timestamps_of(
                timestamp_columns("canteen_data"),
                &[None, Some(summer)],
                cutoff
            ),
            Ok(vec![timestamp(
                "canteen_data",
                "updated_at",
                "2021-06-01T06:00:00Z"
            )])
        );
        // A canteen value after the cut-off is UTC.
        assert_eq!(
            timestamps_of(
                timestamp_columns("canteen_data"),
                &[Some(wall("2026-06-01 08:00:00")), None],
                cutoff
            ),
            Ok(vec![timestamp(
                "canteen_data",
                "created_at",
                "2026-06-01T08:00:00Z"
            )])
        );
        // Every column of `pages`, in its order; an empty table has none.
        let pages = timestamp_columns("pages");
        let read = timestamps_of(
            pages,
            &[
                Some(wall("2020-01-01 00:00:00")),
                Some(wall("2021-01-01 00:00:00")),
                Some(wall("2022-01-01 00:00:00")),
            ],
            cutoff,
        )
        .expect("three values");
        let named: Vec<&str> = read.iter().map(|found| found.column).collect();
        assert_eq!(named, ["created_at", "updated_at", "deleted_at"]);
        assert_eq!(
            timestamps_of(pages, &[None, None, None], cutoff),
            Ok(vec![])
        );
    }

    #[test]
    fn an_answer_with_another_number_of_values_is_refused() {
        let cutoff = CanteenAuditLocalBefore::default_cutoff();
        let mismatch = timestamps_of(timestamp_columns("pages"), &[None, None], cutoff)
            .expect_err("two values for three columns");
        assert_eq!(
            mismatch,
            MaximaCountMismatch {
                table: "pages",
                expected: 3,
                found: 2
            }
        );
        assert_eq!(
            mismatch.to_string(),
            "reading the latest timestamps of `pages` failed: the source answered 2 values for \
             its 3 timestamp columns"
        );
    }

    #[test]
    fn the_facts_exist_only_for_a_snapshot_at_or_after_the_latest_timestamp() {
        let auto_increments = vec![AutoIncrement {
            table: SOURCE_TABLES[0],
            next_id: Some(577),
        }];
        let latest = timestamp("canteen_data", "updated_at", "2021-06-01T06:00:00Z");
        let facts = SourceFacts::checked(auto_increments.clone(), Some(latest), latest.at)
            .expect("at the latest timestamp");
        assert_eq!(facts.auto_increments(), auto_increments.as_slice());
        assert_eq!(facts.latest_timestamp(), Some(latest));
        let empty = SourceFacts::checked(vec![], None, instant("2000-01-01T00:00:00Z"))
            .expect("an empty source bounds nothing");
        assert_eq!(empty.latest_timestamp(), None);

        let refusal = SourceFacts::checked(
            auto_increments,
            Some(latest),
            instant("2021-06-01T05:59:59Z"),
        )
        .expect_err("one second too early");
        assert_eq!(
            refusal,
            SnapshotTooEarly {
                snapshot_at: instant("2021-06-01T05:59:59Z"),
                latest,
            }
        );
    }

    #[test]
    fn auto_increments_come_in_allow_list_order_with_every_table() {
        let mut answer: Vec<(String, Option<u64>)> = SOURCE_TABLES
            .iter()
            .enumerate()
            .map(|(index, table)| {
                let next = u64::try_from(index).expect("small") + 100;
                (table.name().to_owned(), (index % 3 != 0).then_some(next))
            })
            .collect();
        answer.reverse();
        let read = auto_increments(&answer).expect("every table");
        assert_eq!(read.len(), SOURCE_TABLES.len());
        for (index, entry) in read.iter().enumerate() {
            assert_eq!(entry.table, SOURCE_TABLES[index]);
            let next = u64::try_from(index).expect("small") + 100;
            assert_eq!(entry.next_id, (index % 3 != 0).then_some(next));
        }

        let missing = &answer[1..];
        assert_eq!(
            auto_increments(missing),
            Err(AutoIncrementError::Missing {
                table: "role_has_permissions"
            })
        );
        let mut extra = answer.clone();
        extra.push(("newsletter".to_owned(), Some(1)));
        let error = auto_increments(&extra).expect_err("not asked for");
        assert_eq!(
            error,
            AutoIncrementError::Unexpected {
                table: "newsletter".to_owned()
            }
        );
        assert!(error.to_string().contains("\"newsletter\""), "{error}");
        let mut twice = answer;
        twice.push(("users".to_owned(), Some(41)));
        assert!(matches!(
            auto_increments(&twice),
            Err(AutoIncrementError::Unexpected { table }) if table == "users"
        ));
    }

    fn any_wall() -> impl Strategy<Value = NaiveDateTime> {
        // MySQL's DATETIME range starts at year 1000; the legacy data is
        // from 2018 on.
        (0_i64..(9_000 * 366 * 86_400))
            .prop_map(|seconds| wall("1000-01-01 00:00:00") + TimeDelta::seconds(seconds))
    }

    proptest! {
        #[test]
        fn a_latest_statement_takes_the_maximum_of_each_column_of_an_allowed_table(
            table in prop_oneof![
                proptest::sample::select(SOURCE_TABLES.map(SourceTable::name).to_vec()),
                proptest::sample::select(LEFT_BEHIND.to_vec()),
            ],
            columns in proptest::collection::vec("[a-z_]{1,12}", 1..4),
        ) {
            let columns: &'static [&'static str] = Box::leak(
                columns
                    .into_iter()
                    .map(|column| &*Box::leak(column.into_boxed_str()))
                    .collect::<Vec<&'static str>>()
                    .into_boxed_slice(),
            );
            let entry = TimestampColumns { table, columns, kind: TimestampKind::Utc };
            match latest_statement(&entry) {
                Ok(statement) => {
                    let maxima: Vec<String> = columns.iter().map(|column| format!("MAX(`{column}`)")).collect();
                    prop_assert!(!LEFT_BEHIND.contains(&table));
                    prop_assert_eq!(statement, format!("SELECT {} FROM `{table}`", maxima.join(", ")));
                }
                Err(refusal) => {
                    prop_assert!(LEFT_BEHIND.contains(&table));
                    let is_left_behind = matches!(refusal, TableRefusal::LeftBehind { .. });
                    prop_assert!(is_left_behind);
                }
            }
        }

        #[test]
        fn a_local_canteen_value_lies_one_or_two_hours_before_its_utc_reading(value in any_wall()) {
            let local = source_instant(value, TimestampKind::CanteenAudit, CanteenAuditLocalBefore::All);
            let offset = value.and_utc() - local;
            prop_assert!(
                offset == TimeDelta::hours(1) || offset == TimeDelta::hours(2)
                    // Budapest's local mean time before 1890 (+01:16:20).
                    || (offset > TimeDelta::hours(1) && offset < TimeDelta::hours(2)),
                "{} -> {}", value, local
            );
            // A value that the clock shows reads as an instant at which it
            // shows it, and as the later one where it shows it twice.
            if budapest::wall_to_instant(value).is_ok() {
                prop_assert_eq!(budapest::instant_to_wall(local), Ok(value));
                prop_assert_ne!(budapest::instant_to_wall(local + TimeDelta::hours(1)), Ok(value));
            }
        }

        #[test]
        fn source_instants_never_go_backwards(a in any_wall(), b in any_wall(), all in any::<bool>()) {
            let rule = if all {
                CanteenAuditLocalBefore::All
            } else {
                CanteenAuditLocalBefore::default_cutoff()
            };
            let (early, late) = if a <= b { (a, b) } else { (b, a) };
            for kind in [TimestampKind::Utc, TimestampKind::CanteenAudit] {
                let early_at = source_instant(early, kind, rule);
                let late_at = source_instant(late, kind, rule);
                // A later wall time never stands for an instant more than an
                // hour earlier: the autumn hour repeats once.
                prop_assert!(late_at >= early_at - TimeDelta::hours(1), "{} {}", early, late);
                if kind == TimestampKind::Utc {
                    prop_assert!(late_at >= early_at);
                }
            }
        }

        #[test]
        fn the_snapshot_check_refuses_exactly_the_earlier_snapshots(
            snapshot in any_wall(),
            latest_value in proptest::option::of(any_wall()),
        ) {
            let latest = latest_value.map(|value| SourceTimestamp {
                table: "users",
                column: "updated_at",
                at: value.and_utc(),
            });
            let result = check_snapshot_at(snapshot.and_utc(), latest);
            let too_early = latest.is_some_and(|latest| snapshot.and_utc() < latest.at);
            prop_assert_eq!(result.is_err(), too_early);
            if let Err(refusal) = result {
                prop_assert!(refusal.to_string().contains("`users`.`updated_at`"));
            }
        }

        #[test]
        fn an_answer_gives_one_instant_per_value_read_with_the_table_s_kind(
            entry in proptest::sample::select(SOURCE_TIMESTAMPS.to_vec()),
            values in proptest::collection::vec(proptest::option::of(any_wall()), 0..4),
            all in any::<bool>(),
        ) {
            let rule = if all {
                CanteenAuditLocalBefore::All
            } else {
                CanteenAuditLocalBefore::default_cutoff()
            };
            match timestamps_of(&entry, &values, rule) {
                Ok(read) => {
                    prop_assert_eq!(values.len(), entry.columns.len());
                    let expected: Vec<SourceTimestamp> = entry
                        .columns
                        .iter()
                        .zip(&values)
                        .filter_map(|(column, value)| value.map(|value| SourceTimestamp {
                            table: entry.table,
                            column,
                            at: source_instant(value, entry.kind, rule),
                        }))
                        .collect();
                    prop_assert_eq!(&read, &expected);
                    // Only `canteen_data` reads a value as wall time.
                    for (found, value) in read.iter().zip(values.iter().flatten()) {
                        if entry.table != "canteen_data" {
                            prop_assert_eq!(found.at, value.and_utc());
                        }
                    }
                }
                Err(mismatch) => {
                    prop_assert_ne!(values.len(), entry.columns.len());
                    prop_assert_eq!(mismatch.found, values.len());
                    prop_assert_eq!(mismatch.expected, entry.columns.len());
                }
            }
        }

        #[test]
        fn the_facts_hold_exactly_for_a_snapshot_not_before_the_latest(
            snapshot in any_wall(),
            latest_value in proptest::option::of(any_wall()),
        ) {
            let latest = latest_value.map(|value| SourceTimestamp {
                table: "posts_data",
                column: "updated_at",
                at: value.and_utc(),
            });
            let checked = SourceFacts::checked(vec![], latest, snapshot.and_utc());
            prop_assert_eq!(
                checked.is_ok(),
                check_snapshot_at(snapshot.and_utc(), latest).is_ok()
            );
            if let Ok(facts) = checked {
                prop_assert_eq!(facts.latest_timestamp(), latest);
            }
        }

        #[test]
        fn the_latest_is_never_before_any_timestamp(
            values in proptest::collection::vec(any_wall(), 0..12),
        ) {
            let timestamps: Vec<SourceTimestamp> = values
                .iter()
                .map(|value| SourceTimestamp { table: "pages", column: "updated_at", at: value.and_utc() })
                .collect();
            match latest(&timestamps) {
                None => prop_assert!(timestamps.is_empty()),
                Some(found) => {
                    prop_assert!(timestamps.contains(&found));
                    prop_assert!(timestamps.iter().all(|timestamp| timestamp.at <= found.at));
                }
            }
        }

        #[test]
        fn an_answer_in_any_order_gives_every_table_once(
            values in proptest::collection::vec(proptest::option::of(any::<u64>()), 17),
            rotation in any::<prop::sample::Index>(),
        ) {
            let mut answer: Vec<(String, Option<u64>)> = SOURCE_TABLES
                .iter()
                .zip(&values)
                .map(|(table, value)| (table.name().to_owned(), *value))
                .collect();
            let by = rotation.index(answer.len());
            answer.rotate_left(by);
            let read = auto_increments(&answer).expect("every table once");
            for (entry, (table, value)) in read.iter().zip(SOURCE_TABLES.iter().zip(&values)) {
                prop_assert_eq!(entry.table, *table);
                prop_assert_eq!(entry.next_id, *value);
            }
        }
    }
}
