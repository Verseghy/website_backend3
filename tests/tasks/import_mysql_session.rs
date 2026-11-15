//! The import's source session (`tasks::import_mysql::session`,
//! data-migration.md §3.3) against the MySQL source of compose.yaml (profile
//! `import`; the CI Test job starts it), whose schema is
//! `tests/fixtures/legacy/schema.sql` and which holds no rows. Its account
//! has the cut-over account's grants, `SELECT` and `LOCK TABLES`, so these
//! tests lock and read with no more rights than production's import has.
//!
//! The session's `LOCK TABLES ... READ` takes shared locks, so the tests
//! run in parallel on the one source. The checks over what the session
//! reads are pure and have their own unit tests in
//! `src/domain/import_mysql/` (the fingerprint comparison, the refusal of
//! an early `snapshot_at` and its message); here the session runs them on
//! the real server. The source holds no rows, so its latest timestamp is
//! none; the synthetic legacy rows of the steps that follow give it some
//! (data-migration.md §12).

use std::collections::BTreeSet;

use backend3::{
    domain::import_mysql::{
        fingerprint::{ExpectedColumn, ExpectedTable},
        options::{CanteenAuditLocalBefore, DEFAULT_RESOLUTIONS, ImportOptions, Mode},
        source_session::SESSION_SETTINGS,
        source_tables::SOURCE_TABLES,
    },
    tasks::import_mysql::{
        expected_schema::EXPECTED_SCHEMA,
        session::{SourceSession, decode_maxima, read},
        source::select_rows,
    },
};
use chrono::{DateTime, Datelike, NaiveDateTime, Utc};
use sqlx::{Column, MySqlPool, Row, TypeInfo, mysql::MySqlDatabaseError};

use super::import_mysql_source::{CapturedLog, compose_source};
use crate::support::repository_root;

/// The legacy tables that the import never reads (data-migration.md §9.1).
const NEVER_READ: [&str; 3] = ["newsletter", "password_resets", "migrations"];

/// The options of a run that the session reads `snapshot_at` and the
/// canteen cut-off from.
fn options(snapshot_at: &str) -> ImportOptions {
    ImportOptions {
        mode: Mode::DryRun,
        snapshot_at: DateTime::parse_from_rfc3339(snapshot_at)
            .expect("RFC 3339")
            .with_timezone(&Utc),
        resolutions: DEFAULT_RESOLUTIONS.into(),
        storage_root: "/storage".into(),
        report: "/reports/run-1".into(),
        canteen_audit_local_before: CanteenAuditLocalBefore::default_cutoff(),
    }
}

/// The MySQL error number of a refused statement.
fn error_number(error: &sqlx::Error) -> u16 {
    match error {
        sqlx::Error::Database(database) => database
            .try_downcast_ref::<MySqlDatabaseError>()
            .expect("a MySQL error")
            .number(),
        other => panic!("not a server error: {other}"),
    }
}

/// The id of the MySQL connection that `executor` runs its statement on.
async fn connection_id(executor: impl sqlx::MySqlExecutor<'_>) -> u64 {
    sqlx::query_scalar("SELECT CONNECTION_ID()")
        .fetch_one(executor)
        .await
        .expect("the connection id")
}

/// Compose source: the session settings of §3.3 hold on the session's
/// connection, the allow-listed tables can be read there, and the tables
/// the import never reads cannot: a `LOCK TABLES` session reads only what
/// it locked, so MySQL refuses them with error 1100 ("was not locked with
/// LOCK TABLES"). After `close`, the same connection reads them again: the
/// session unlocked before it handed the connection back.
#[tokio::test]
async fn the_session_locks_exactly_the_allow_list_and_unlocks_at_the_end() {
    let source: MySqlPool = compose_source().await;
    let mut session = SourceSession::open(&source, &EXPECTED_SCHEMA)
        .await
        .expect("the session opens");
    let locked_connection = connection_id(session.connection()).await;

    let row = sqlx::query(
        "SELECT @@session.time_zone, @@session.character_set_client, \
         @@session.character_set_results, @@session.character_set_connection, \
         @@session.information_schema_stats_expiry",
    )
    .fetch_one(session.connection())
    .await
    .expect("the session variables");
    assert_eq!(row.get::<String, _>(0), "+00:00");
    for index in 1..=3 {
        assert_eq!(row.get::<String, _>(index), "utf8mb4", "{index}");
    }
    assert_eq!(row.get::<u64, _>(4), 0);

    // Every allow-listed table reads under the lock, with every expected
    // column; the fixture holds no rows.
    for table in EXPECTED_SCHEMA {
        let columns: Vec<&str> = table.columns.iter().map(|column| column.name).collect();
        let rows = select_rows(session.connection(), table.table, &columns, &columns[..1])
            .await
            .unwrap_or_else(|error| panic!("{}: {error}", table.table));
        assert!(rows.is_empty(), "{}", table.table);
    }

    for (table, statement) in [
        ("newsletter", "SELECT COUNT(*) FROM `newsletter`"),
        ("password_resets", "SELECT COUNT(*) FROM `password_resets`"),
        ("migrations", "SELECT COUNT(*) FROM `migrations`"),
    ] {
        let error = sqlx::query(statement)
            .fetch_one(session.connection())
            .await
            .expect_err(table);
        assert_eq!(error_number(&error), 1100, "{table}: {error}");
    }

    session.close().await.expect("the session unlocks");
    // The pool holds one connection, so the next read uses the session's.
    let mut connection = source.acquire().await.expect("the connection");
    assert_eq!(connection_id(&mut *connection).await, locked_connection);
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM `newsletter`")
        .fetch_one(&mut *connection)
        .await
        .expect("unlocked: the account may read every table again");
    assert_eq!(count, 0);
}

/// Compose source: a session dropped without `close`, as an error drops
/// it, closes its connection instead of handing it back to the pool, and
/// MySQL ends the locks of a closed connection. The pool holds one
/// connection, so the next read gets a new one, on which every table the
/// account may read is readable again.
#[tokio::test]
async fn a_session_dropped_without_close_closes_its_connection_and_its_locks() {
    let source = compose_source().await;
    let mut session = SourceSession::open(&source, &EXPECTED_SCHEMA)
        .await
        .expect("the session opens");
    let locked_connection = connection_id(session.connection()).await;
    drop(session);

    let mut connection = source.acquire().await.expect("a connection");
    assert_ne!(
        connection_id(&mut *connection).await,
        locked_connection,
        "the locked connection went back to the pool"
    );
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM `newsletter`")
        .fetch_one(&mut *connection)
        .await
        .expect("no lock left on the new connection");
    assert_eq!(count, 0);
}

/// Compose source: the answer of a latest-timestamp statement decodes
/// through `decode_maxima` for both column types it can have, `DATETIME`
/// (the Laravel audit columns, and `MAX` over them) and `TIMESTAMP`
/// (`users` and `menu_items`), and for a column with no value. The source
/// holds no rows, so the statement takes the values from literals and from
/// `information_schema.TABLES.CREATE_TIME`, which MySQL 8.4 types as
/// `TIMESTAMP`; in the `+00:00` session it is a UTC wall time, so it lies
/// at or before the server's `UTC_TIMESTAMP()`.
#[tokio::test]
async fn a_latest_timestamp_answer_decodes_datetime_timestamp_and_null() {
    let source = compose_source().await;
    let mut connection = source.acquire().await.expect("a connection");
    sqlx::raw_sql(SESSION_SETTINGS)
        .execute(&mut *connection)
        .await
        .expect("the session settings");
    let row = sqlx::query(
        "SELECT CAST('2021-03-28 02:30:00' AS DATETIME), CREATE_TIME, CAST(NULL AS DATETIME), \
         UTC_TIMESTAMP() FROM information_schema.TABLES WHERE TABLE_SCHEMA = DATABASE() AND \
         TABLE_NAME = 'users'",
    )
    .fetch_one(&mut *connection)
    .await
    .expect("the row");
    let types: Vec<&str> = row
        .columns()
        .iter()
        .map(|column| column.type_info().name())
        .collect();
    assert_eq!(types[..3], ["DATETIME", "TIMESTAMP", "DATETIME"]);

    let maxima = decode_maxima(&row, 3).expect("both types and NULL decode");
    assert_eq!(
        maxima[0],
        Some(
            NaiveDateTime::parse_from_str("2021-03-28 02:30:00", "%Y-%m-%d %H:%M:%S")
                .expect("a wall time")
        )
    );
    let created = maxima[1].expect("the table's creation time");
    let server_now: NaiveDateTime = row.get(3);
    assert!(created <= server_now, "{created} after {server_now}");
    assert!(created.year() >= 2024, "{created}");
    assert_eq!(maxima[2], None);
    // The reason `decode_maxima` reads a UTC `DateTime`: sqlx does not
    // decode a `TIMESTAMP` column as a `NaiveDateTime`.
    assert!(row.try_get::<Option<NaiveDateTime>, _>(1).is_err());
    // A statement with fewer columns than asked for is an error, not a
    // shorter list.
    assert!(decode_maxima(&row, 5).is_err());
}

/// Compose source: every allow-listed table's `AUTO_INCREMENT` is read, in
/// the allow-list's order. The fixture's tables are new and empty, so each
/// table with an auto-increment `id` reports 1, the first id MySQL would
/// issue.
#[tokio::test]
async fn the_session_reads_every_table_s_auto_increment() {
    let source = compose_source().await;
    let mut session = SourceSession::open(&source, &EXPECTED_SCHEMA)
        .await
        .expect("the session opens");
    let auto_increments = session.auto_increments().await.expect("AUTO_INCREMENT");
    session.close().await.expect("the session unlocks");

    assert_eq!(auto_increments.len(), SOURCE_TABLES.len());
    for (entry, (table, expected)) in auto_increments
        .iter()
        .zip(SOURCE_TABLES.iter().zip(EXPECTED_SCHEMA))
    {
        assert_eq!(entry.table, *table);
        if expected.columns.iter().any(|column| column.name == "id") {
            assert_eq!(entry.next_id, Some(1), "{}", table.name());
        }
    }
}

/// Compose source: `read` runs the whole session. The source holds no
/// rows, so no timestamp bounds `snapshot_at`, and any instant passes.
#[tokio::test]
async fn reading_the_empty_source_finds_no_timestamp() {
    let source = compose_source().await;
    let facts = read(&source, &options("1970-01-01T00:00:00Z"))
        .await
        .expect("the session");
    assert_eq!(facts.latest_timestamp(), None);
    assert_eq!(facts.auto_increments().len(), SOURCE_TABLES.len());

    let mut session = SourceSession::open(&source, &EXPECTED_SCHEMA)
        .await
        .expect("the session opens");
    assert_eq!(
        session
            .latest_timestamp(CanteenAuditLocalBefore::All)
            .await
            .expect("the timestamps"),
        None
    );
    session.close().await.expect("the session unlocks");
}

/// `schema` with `columns` in place of `table`'s columns.
fn with_columns(
    schema: &mut [ExpectedTable],
    table: &str,
    columns: impl FnOnce(&[ExpectedColumn]) -> Vec<ExpectedColumn>,
) {
    let entry = schema
        .iter_mut()
        .find(|entry| entry.table == table)
        .expect("an expected table");
    // Leaked, because `ExpectedTable` holds `'static` slices; a test may.
    entry.columns = Box::leak(columns(entry.columns).into_boxed_slice());
}

/// Compose source: an expected DDL that differs from the source stops the
/// session before it locks anything, with a message that names every
/// table and column that differs.
#[tokio::test]
async fn a_fingerprint_mismatch_names_every_difference() {
    let source = compose_source().await;
    let mut expected = EXPECTED_SCHEMA.to_vec();
    with_columns(&mut expected, "posts_data", |columns| {
        columns
            .iter()
            .filter(|column| column.name != "previewToken")
            .map(|column| {
                if column.name == "id" {
                    ExpectedColumn {
                        column_type: "int",
                        ..*column
                    }
                } else {
                    *column
                }
            })
            .chain([ExpectedColumn {
                name: "type",
                column_type: "smallint unsigned",
                nullable: false,
            }])
            .collect()
    });
    with_columns(&mut expected, "users", |columns| {
        columns
            .iter()
            .map(|column| ExpectedColumn {
                nullable: column.nullable || column.name == "email",
                ..*column
            })
            .collect()
    });
    expected.push(ExpectedTable {
        table: "posts_comments",
        columns: &[ExpectedColumn {
            name: "id",
            column_type: "int unsigned",
            nullable: false,
        }],
    });

    let message = SourceSession::open(&source, &expected)
        .await
        .err()
        .expect("the source differs")
        .to_string();
    assert!(
        message.starts_with(
            "refusing to import: the source schema differs from the expected legacy DDL \
             (data-migration.md §3.3) in 5 places: "
        ),
        "{message}"
    );
    for part in [
        "`posts_data`.`id` is int unsigned, expected int",
        "`posts_data`.`type` is missing (expected smallint unsigned)",
        "`posts_data`.`previewToken` (varchar(191)) is not in the expected DDL",
        "`users`.`email` is NOT NULL, expected NULL",
        "the table `posts_comments` is missing",
    ] {
        assert!(message.contains(part), "{part} in {message}");
    }
    assert!(!message.contains("loco"), "{message}");
}

/// Compose source: no statement of the session names a table that the
/// import never reads (NWS-01). The driver logs every statement it sends,
/// so the test reads them from the log of one whole session.
#[tokio::test]
async fn no_statement_of_the_session_names_a_table_the_import_never_reads() {
    let log = CapturedLog::default();
    let guard = tracing::subscriber::set_default(log.clone());
    let source = compose_source().await;
    read(&source, &options("2026-10-23T17:01:02Z"))
        .await
        .expect("the session");
    source.close().await;
    drop(guard);

    let text = log.text();
    // The driver did log the session's statements, so the check below has
    // something to look at.
    for statement in [
        "SET time_zone = '+00:00', NAMES utf8mb4, information_schema_stats_expiry = 0",
        "LOCK TABLES `posts_data` READ,",
        "FROM information_schema.COLUMNS",
        "FROM information_schema.TABLES",
        "SELECT MAX(`created_at`), MAX(`updated_at`) FROM `posts_data`",
        "UNLOCK TABLES",
    ] {
        assert!(text.contains(statement), "{statement} in the log:\n{text}");
    }
    for table in NEVER_READ {
        assert!(!text.contains(table), "{table} in the log:\n{text}");
    }

    // The order of §3.3: the settings, the fingerprint, the lock, the
    // fingerprint again under the lock, the reads, then the unlock.
    let position = |statement: &str| {
        text.find(statement)
            .unwrap_or_else(|| panic!("{statement} in the log:\n{text}"))
    };
    let fingerprints: Vec<usize> = text
        .match_indices("FROM information_schema.COLUMNS")
        .map(|(index, _)| index)
        .collect();
    let lock = position("LOCK TABLES `posts_data` READ,");
    let unlock = position("UNLOCK TABLES");
    assert_eq!(
        fingerprints.len(),
        2,
        "two fingerprints in the log:\n{text}"
    );
    assert!(
        position("SET time_zone = '+00:00'") < fingerprints[0],
        "settings first:\n{text}"
    );
    assert!(
        fingerprints[0] < lock,
        "a fingerprint before the lock:\n{text}"
    );
    assert!(
        lock < fingerprints[1],
        "a fingerprint under the lock:\n{text}"
    );
    for read in [
        "FROM information_schema.TABLES",
        "SELECT MAX(`created_at`), MAX(`updated_at`) FROM `posts_data`",
    ] {
        let at = position(read);
        assert!(
            fingerprints[1] < at && at < unlock,
            "{read} under the lock:\n{text}"
        );
    }
}

/// A table of the legacy fixture: its name, and each column's name,
/// `COLUMN_TYPE` and nullability.
type FixtureTable = (String, Vec<(String, String, bool)>);

/// The columns of each `CREATE TABLE` of the legacy fixture, as
/// `information_schema.COLUMNS` reports them: name, `COLUMN_TYPE` (the
/// type word, with `unsigned` when it follows) and whether it admits NULL.
fn columns_of_the_fixture() -> Vec<FixtureTable> {
    let fixture =
        std::fs::read_to_string(repository_root().join("tests/fixtures/legacy/schema.sql"))
            .expect("the legacy fixture");
    let mut tables: Vec<FixtureTable> = Vec::new();
    for line in fixture.lines() {
        if let Some(rest) = line.strip_prefix("CREATE TABLE `") {
            let name = rest.split_once('`').expect("a closing backtick").0;
            tables.push((name.to_owned(), Vec::new()));
        } else if let Some(rest) = line.strip_prefix("  `") {
            let (name, definition) = rest.split_once("` ").expect("a column definition");
            let mut words = definition.split(' ');
            let mut column_type = words.next().expect("a type").to_owned();
            if words.next() == Some("unsigned") {
                column_type.push_str(" unsigned");
            }
            let nullable = !definition.contains("NOT NULL");
            tables.last_mut().expect("inside a CREATE TABLE").1.push((
                name.to_owned(),
                column_type,
                nullable,
            ));
        }
    }
    tables
}

#[test]
fn the_expected_schema_is_the_ddl_of_the_legacy_fixture() {
    let fixture = columns_of_the_fixture();
    let read: BTreeSet<&str> = EXPECTED_SCHEMA.iter().map(|table| table.table).collect();
    let fixture_tables: BTreeSet<&str> = fixture.iter().map(|(name, _)| name.as_str()).collect();
    let never_read: BTreeSet<&str> = fixture_tables.difference(&read).copied().collect();
    assert_eq!(never_read, NEVER_READ.into_iter().collect());
    for table in EXPECTED_SCHEMA {
        let columns = &fixture
            .iter()
            .find(|(name, _)| name == table.table)
            .unwrap_or_else(|| panic!("{} is not in the fixture", table.table))
            .1;
        let expected: Vec<(String, String, bool)> = table
            .columns
            .iter()
            .map(|column| {
                (
                    column.name.to_owned(),
                    column.column_type.to_owned(),
                    column.nullable,
                )
            })
            .collect();
        assert_eq!(*columns, expected, "{}", table.table);
    }
}
