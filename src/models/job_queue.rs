//! Loco's Postgres job queue table, `pg_loco_queue`, as `/metrics` reads it
//! (ADR 0016 §3, ADR 0023 §9).
//!
//! A model module without an entity (ADR 0008 rule 5): Loco creates and
//! owns the table (`loco_rs::bgworker::pg::initialize_database`), and
//! Sea-ORM migrations never touch it, so there is no entity to generate.
//! Raw queries stay out of controllers (rule 4), so the scrape's one query
//! lives here and takes its connection and its time limit as arguments
//! (rule 2). It is a single read-only aggregate: it locks no row, so it never
//! waits for the worker's `FOR UPDATE SKIP LOCKED` dequeue. Only a table lock
//! (a migration's `ALTER TABLE`, a `VACUUM FULL`) or an overloaded server
//! makes it wait, and the time limit covers those.

use std::time::Duration;

use sea_orm::{
    AccessMode, ConnectionTrait, DatabaseConnection, DbBackend, DbErr, FromQueryResult, Statement,
    TransactionTrait,
};

/// Counts the jobs per status and, per status, the age of the oldest job
/// that is due (`run_at` not in the future), measured with the database's
/// clock so that the application's clock cannot skew it.
///
/// `EXTRACT(EPOCH ...)` is `numeric` since PostgreSQL 14, hence the cast to
/// `float8`. The column is `NULL` when no job of that status is due.
const JOBS_BY_STATUS: &str = "SELECT status, COUNT(*) AS jobs, \
     EXTRACT(EPOCH FROM (now() - MIN(run_at) FILTER (WHERE run_at <= now())))::float8 \
     AS oldest_due_age_seconds \
     FROM pg_loco_queue GROUP BY status";

/// The largest `statement_timeout` PostgreSQL accepts, in milliseconds
/// (`INT_MAX`).
const MAX_STATEMENT_TIMEOUT_MS: u128 = i32::MAX as u128;

/// The jobs of one status in `pg_loco_queue`.
#[derive(Clone, Debug, PartialEq, FromQueryResult)]
pub struct JobsByStatus {
    /// The `status` column, for example `queued`.
    pub status: String,
    /// How many jobs have that status.
    pub jobs: i64,
    /// Seconds since the oldest due job of that status became due, or
    /// `None` when none of them is due.
    pub oldest_due_age_seconds: Option<f64>,
}

/// Reads the per-status summary of the job queue, one row per status that
/// has jobs.
///
/// The query runs in a read-only transaction with `SET LOCAL
/// statement_timeout`, so PostgreSQL itself cancels it once it has run for
/// `statement_timeout`. A timeout on the caller's side alone is not enough:
/// when the caller drops the query, sqlx keeps its connection checked out
/// until the server has answered (sqlx-core 0.9,
/// `PoolConnection::return_to_pool` pings the connection first), so scrapes
/// that keep waiting for a locked table would use up the pool.
///
/// - `db`: a connection to the application database, which holds the queue
///   table (`queue.uri` is the same `DATABASE_URL`, ADR 0016 §3).
/// - `statement_timeout`: how long PostgreSQL lets the query run, in whole
///   milliseconds; between 1 ms and `i32::MAX` ms, because 0 would turn the
///   limit off.
///
/// # Errors
///
/// Returns the database error, for example when the table does not exist
/// because no Postgres queue was ever started on this database, or when the
/// server cancelled the query at `statement_timeout`. Returns
/// [`DbErr::Custom`] for a `statement_timeout` outside the range above,
/// before it touches the database.
pub async fn jobs_by_status(
    db: &DatabaseConnection,
    statement_timeout: Duration,
) -> Result<Vec<JobsByStatus>, DbErr> {
    let millis = statement_timeout.as_millis();
    if !(1..=MAX_STATEMENT_TIMEOUT_MS).contains(&millis) {
        return Err(DbErr::Custom(format!(
            "the job-queue statement_timeout must be between 1 ms and {MAX_STATEMENT_TIMEOUT_MS} \
             ms, got {statement_timeout:?}"
        )));
    }
    let transaction = db
        .begin_with_config(None, Some(AccessMode::ReadOnly))
        .await?;
    // `SET` takes no bind parameters. The value is an integer formatted
    // here, so nothing from outside reaches the statement text.
    transaction
        .execute_unprepared(&format!("SET LOCAL statement_timeout = {millis}"))
        .await?;
    // On an error the transaction is dropped, which queues its rollback;
    // sqlx sends it before the connection goes back to the pool.
    let rows = JobsByStatus::find_by_statement(Statement::from_string(
        DbBackend::Postgres,
        JOBS_BY_STATUS,
    ))
    .all(&transaction)
    .await?;
    transaction.commit().await?;
    Ok(rows)
}
