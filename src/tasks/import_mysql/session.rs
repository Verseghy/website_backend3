//! The import's source session (docs/design/data-migration.md §3.3, §4.3):
//! the one locked session in which the import reads the legacy MySQL
//! source.
//!
//! [`read`] runs it from start to end. [`SourceSession::open`] takes the
//! source pool's one connection and, on it:
//!
//! 1. sends the session settings (`time_zone` `+00:00`, `NAMES utf8mb4`,
//!    `information_schema_stats_expiry` 0);
//! 2. compares the source's columns with the expected legacy DDL
//!    ([`super::expected_schema::EXPECTED_SCHEMA`]), naming every mismatch;
//! 3. takes `LOCK TABLES <every allow-listed table> READ`, so that even a
//!    writable source cannot change while the session reads it;
//! 4. compares the columns again, now that no statement can change them
//!    until the session ends.
//!
//! The first comparison runs before the lock because `LOCK TABLES` on a
//! table that does not exist fails with MySQL error 1146, and the source
//! layer passes on only the error number (data-migration.md §4.1): the
//! fingerprint names the missing table instead. The second is the one the
//! rows are read under.
//!
//! [`read`] then reads every table's `AUTO_INCREMENT`
//! ([`SourceSession::auto_increments`]) and the latest write instant in the
//! source ([`SourceSession::latest_timestamp`]), refuses a `snapshot_at`
//! earlier than it, and ends the session with `UNLOCK TABLES`
//! ([`SourceSession::close`]). The refusal is part of
//! `SourceFacts::checked`, the only way to build the session's facts, so
//! no run gets past the session without it. The import steps read their
//! rows in the session, with `source::select_rows` on
//! [`SourceSession::connection`]. Only the decoding of the answers stays
//! here ([`decode_maxima`]); what a value means is decided in
//! `domain::import_mysql::source_session`.
//!
//! This file and `source.rs` are the only files of the import task that
//! send statements to the source (`SOURCE_STATEMENT_FILES` in
//! `tests/architecture.rs`). Every statement here comes from
//! `domain::import_mysql::source_session`, which names its tables from the
//! source allow-list, so `newsletter`, `password_resets` and `migrations`
//! are never locked, described or read (NWS-01). The texts are built from
//! fixed identifiers and contain no value, so each is passed to sqlx with
//! `AssertSqlSafe`. Every driver error goes through `source::source_error`,
//! so no message shows any part of the source URL.

use chrono::{DateTime, NaiveDateTime, Utc};
use loco_rs::{Error, Result};
use sqlx::{
    AssertSqlSafe, MySql, MySqlConnection, MySqlPool, Row, mysql::MySqlRow, pool::PoolConnection,
};

use super::{expected_schema::EXPECTED_SCHEMA, source::source_error};
use crate::domain::import_mysql::{
    fingerprint::{self, ExpectedTable, FoundColumn},
    options::{CanteenAuditLocalBefore, ImportOptions},
    source_session::{
        self, AutoIncrement, SESSION_SETTINGS, SOURCE_TIMESTAMPS, SourceFacts, SourceTimestamp,
        UNLOCK,
    },
};

/// The locked source session: the source pool's one connection, with the
/// session settings sent, the fingerprint checked and every allow-listed
/// table locked for reading.
///
/// End it with [`SourceSession::close`], which unlocks the tables and hands
/// the connection back to the pool. A session that is dropped instead (after
/// an error) closes its connection, and MySQL releases the locks of a
/// closed connection, so no locked connection ever goes back to the pool.
pub struct SourceSession {
    connection: PoolConnection<MySql>,
    /// Whether `UNLOCK TABLES` has succeeded. Until then the connection may
    /// hold the session's locks or the state an error left behind.
    ended: bool,
}

impl Drop for SourceSession {
    fn drop(&mut self) {
        if !self.ended {
            self.connection.close_on_drop();
        }
    }
}

impl SourceSession {
    /// Opens the session on `source_db`'s connection: the session settings,
    /// the fingerprint, `LOCK TABLES`, and the fingerprint again.
    ///
    /// - `source_db`: the source pool (`source::pool`), whose one connection
    ///   the session holds until it ends.
    /// - `expected`: the expected legacy DDL; the import passes
    ///   [`EXPECTED_SCHEMA`].
    ///
    /// # Errors
    ///
    /// A failed exchange with the source (`source::source_error`), or the
    /// fingerprint's refusal, which names every table and column that
    /// differs (data-migration.md §4.2).
    pub async fn open(source_db: &MySqlPool, expected: &[ExpectedTable]) -> Result<Self> {
        let connection = source_db
            .acquire()
            .await
            .map_err(|error| source_error("opening the source session", &error))?;
        // From here on, dropping the session (on an error below) closes the
        // connection instead of handing it back with the session state that
        // the error left behind.
        let mut session = Self {
            connection,
            ended: false,
        };
        execute(
            &mut session.connection,
            SESSION_SETTINGS,
            "setting the source session's time zone, character set and statistics expiry",
        )
        .await?;
        check_fingerprint(&mut session.connection, expected).await?;
        execute(
            &mut session.connection,
            &source_session::lock_statement(),
            "locking the legacy tables for reading",
        )
        .await?;
        check_fingerprint(&mut session.connection, expected).await?;
        Ok(session)
    }

    /// The session's connection, for the import steps' reads with
    /// `source::select_rows`. Only the locked tables can be read on it.
    pub fn connection(&mut self) -> &mut MySqlConnection {
        &mut self.connection
    }

    /// Every allow-listed table's `AUTO_INCREMENT`, the next id MySQL would
    /// issue, in the order of `SOURCE_TABLES` (data-migration.md §5.1).
    ///
    /// # Errors
    ///
    /// A failed read, or an answer that misses a table or names another.
    pub async fn auto_increments(&mut self) -> Result<Vec<AutoIncrement>> {
        let doing = "reading the legacy tables' AUTO_INCREMENT";
        let rows = sqlx::query(AssertSqlSafe(source_session::auto_increment_statement()))
            .fetch_all(&mut *self.connection)
            .await
            .map_err(|error| source_error(doing, &error))?;
        let answer = rows
            .iter()
            .map(|row| {
                Ok((
                    row.try_get::<String, _>(0)?,
                    row.try_get::<Option<u64>, _>(1)?,
                ))
            })
            .collect::<std::result::Result<Vec<_>, sqlx::Error>>()
            .map_err(|error| source_error(doing, &error))?;
        source_session::auto_increments(&answer).map_err(Error::msg)
    }

    /// The latest write instant in the source: the greatest value of every
    /// column of `SOURCE_TIMESTAMPS`, each read as the instant it stands
    /// for (`source_session::timestamps_of`, data-migration.md §4.1, §5.2),
    /// or `None` when none holds a value.
    ///
    /// - `canteen_audit_local_before`: the run's cut-off for the
    ///   `canteen_data` audit values.
    ///
    /// # Errors
    ///
    /// A failed read, or an answer with another number of values than the
    /// statement asked for.
    pub async fn latest_timestamp(
        &mut self,
        canteen_audit_local_before: CanteenAuditLocalBefore,
    ) -> Result<Option<SourceTimestamp>> {
        let mut found = Vec::new();
        for columns in &SOURCE_TIMESTAMPS {
            let statement = source_session::latest_statement(columns).map_err(Error::msg)?;
            let doing = format!("reading the latest timestamps of `{}`", columns.table);
            let row = sqlx::query(AssertSqlSafe(statement))
                .fetch_one(&mut *self.connection)
                .await
                .map_err(|error| source_error(&doing, &error))?;
            let maxima = decode_maxima(&row, columns.columns.len())
                .map_err(|error| source_error(&doing, &error))?;
            found.extend(
                source_session::timestamps_of(columns, &maxima, canteen_audit_local_before)
                    .map_err(Error::msg)?,
            );
        }
        Ok(source_session::latest(&found))
    }

    /// Ends the session: `UNLOCK TABLES`, then hands the connection back to
    /// the pool.
    ///
    /// # Errors
    ///
    /// A failed `UNLOCK TABLES`; the connection is then closed instead, and
    /// MySQL releases the locks of a closed connection.
    pub async fn close(mut self) -> Result<()> {
        execute(&mut self.connection, UNLOCK, "unlocking the legacy tables").await?;
        self.ended = true;
        Ok(())
    }
}

/// Reads what the import needs from the source in one locked session:
/// opens it ([`SourceSession::open`] with [`EXPECTED_SCHEMA`]), reads every
/// table's `AUTO_INCREMENT` and the latest timestamp, refuses a
/// `snapshot_at` earlier than that timestamp, and ends the session.
///
/// - `source_db`: the source pool (`source::pool`).
/// - `options`: the run's arguments; `snapshot_at` and
///   `canteen_audit_local_before` are read.
///
/// # Errors
///
/// Every refusal of [`SourceSession::open`], a failed read, and the
/// refusal of `snapshot_at` (data-migration.md §4.1), which names the
/// latest timestamp and its column.
pub async fn read(source_db: &MySqlPool, options: &ImportOptions) -> Result<SourceFacts> {
    let mut session = SourceSession::open(source_db, &EXPECTED_SCHEMA).await?;
    let facts = async {
        let auto_increments = session.auto_increments().await?;
        let latest_timestamp = session
            .latest_timestamp(options.canteen_audit_local_before)
            .await?;
        // The only way to the facts runs the check of `snapshot_at`.
        SourceFacts::checked(auto_increments, latest_timestamp, options.snapshot_at)
            .map_err(Error::msg)
    }
    .await;
    match facts {
        Ok(facts) => {
            session.close().await?;
            Ok(facts)
        }
        // Dropping the session closes its connection, which ends the locks.
        Err(error) => Err(error),
    }
}

/// The values of the first `count` columns of `row`, the answer of one
/// `source_session::latest_statement`: the latest value of each
/// write-instant column, `None` for a column with no value.
///
/// - `row`: the answer's one row.
/// - `count`: how many columns the statement asked for.
///
/// sqlx reads `DATETIME` and `TIMESTAMP` alike as a UTC `DateTime`, without
/// any zone conversion (`MAX` over a column keeps its type): the value is the
/// wall time as the `+00:00` session returns it, which
/// `source_session::timestamps_of` interprets. Decoding as `NaiveDateTime`
/// would refuse a `TIMESTAMP` column, which sqlx 0.9 does not count as
/// compatible with it. This function sends nothing.
///
/// # Errors
///
/// A column that is missing or holds another type.
pub fn decode_maxima(
    row: &MySqlRow,
    count: usize,
) -> std::result::Result<Vec<Option<NaiveDateTime>>, sqlx::Error> {
    (0..count)
        .map(|index| {
            row.try_get::<Option<DateTime<Utc>>, _>(index)
                .map(|value| value.map(|value| value.naive_utc()))
        })
        .collect()
}

/// Sends `statement`, which returns no rows, on `connection`.
///
/// - `connection`: the session's connection.
/// - `statement`: the statement, from `domain::import_mysql::source_session`.
/// - `doing`: what it does, for the error message.
async fn execute(connection: &mut MySqlConnection, statement: &str, doing: &str) -> Result<()> {
    sqlx::raw_sql(AssertSqlSafe(statement.to_owned()))
        .execute(connection)
        .await
        .map(|_| ())
        .map_err(|error| source_error(doing, &error))
}

/// Reads the columns of the allow-listed tables from
/// `information_schema.COLUMNS` and compares them with `expected`.
///
/// - `connection`: the session's connection.
/// - `expected`: the expected legacy DDL.
///
/// # Errors
///
/// A failed read, an `IS_NULLABLE` that is neither `YES` nor `NO`, or the
/// fingerprint's refusal, which names every mismatch.
async fn check_fingerprint(
    connection: &mut MySqlConnection,
    expected: &[ExpectedTable],
) -> Result<()> {
    let doing = "reading the source schema for its fingerprint";
    let rows = sqlx::query(AssertSqlSafe(source_session::fingerprint_statement()))
        .fetch_all(connection)
        .await
        .map_err(|error| source_error(doing, &error))?;
    let mut found = Vec::with_capacity(rows.len());
    for row in &rows {
        let read = |index: usize| {
            row.try_get::<String, _>(index)
                .map_err(|error| source_error(doing, &error))
        };
        let (table, name, column_type, is_nullable) = (read(0)?, read(1)?, read(2)?, read(3)?);
        let nullable = fingerprint::parse_is_nullable(&is_nullable).ok_or_else(|| {
            Error::Message(format!(
                "{doing} failed: information_schema.COLUMNS answered IS_NULLABLE {is_nullable:?} \
                 for `{table}`.`{name}`, which is neither YES nor NO"
            ))
        })?;
        found.push(FoundColumn {
            table,
            name,
            column_type,
            nullable,
        });
    }
    fingerprint::compare(expected, &found).map_err(Error::msg)
}
