//! The advisory lock every backend3 task runs under (ADR 0016 §4).
//!
//! A `CronJob` runs each recurring task, and an operator may run the same
//! task by hand. `concurrencyPolicy: Forbid` keeps two scheduled runs apart,
//! but not a scheduled run and a manual one, so every task wraps its work in
//! [`with_task_lock`].
//!
//! - **A transaction-level lock on a transaction of its own.** The helper
//!   takes `pg_try_advisory_xact_lock(key)` in a *holder* transaction on one
//!   pooled connection and keeps it open, idle, until the work has finished;
//!   the lock ends with it. A session-level lock is never used: it would stay
//!   with the pooled connection after the pool took it back.
//! - **The work runs outside the holder**, in transactions of its own: the
//!   import, for example, writes everything in one transaction and rolls it
//!   back on a dry run (ADR 0017), which must not end the lock.
//! - **A lost lock is reported.** If the holder's session ends during the
//!   run (a restarted server, a terminated backend), the helper logs a
//!   warning that another run may have overlapped this one, and returns the
//!   work's own result: tasks are idempotent and the `CronJob` retries a
//!   failed run (ADR 0016 §4).
//!
//! It is a model module without an entity (ADR 0008 rule 5): database code
//! that every task shares and no Sea-ORM entity owns.

use std::future::Future;

use loco_rs::{Error, Result};
use sea_orm::{
    ConnectionTrait, DatabaseConnection, DatabaseConnectionType, DatabaseTransaction, DbBackend,
    Statement, TransactionTrait,
};

use crate::domain::task_lock_key::task_lock_key;

/// The smallest database pool [`with_task_lock`] accepts: the holder keeps
/// one connection for the whole run, and the work needs at least one more.
pub const MIN_POOL_CONNECTIONS: u32 = 2;

/// What [`with_task_lock`] did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TaskLockOutcome<T> {
    /// The lock was free: the work ran, and this is what it returned.
    Ran(T),
    /// Another run of the task holds the lock: the work did not run.
    AlreadyRunning,
}

/// Runs `work` while this process holds the task's advisory lock, or skips
/// it and logs that when another run holds the lock (ADR 0016 §4).
///
/// - `db`: the application's pool, `ctx.db` in a task. Not a transaction,
///   which would share one session with the work, so the lock would exclude
///   nothing within it.
/// - `task_name`: the name the task registers in its `TaskInfo`; its key
///   comes from [`task_lock_key`].
/// - `work`: the task's work. It runs at most once, after the lock is taken,
///   and the lock is held until it has finished.
///
/// A task's `run` does all of its database work inside
/// `with_task_lock(&ctx.db, &self.task().name, || async { ... })` and returns
/// `Ok(())` for both outcomes, so the CLI exits 0 when the lock is held;
/// `ProbeTask` in `tests/models/task_lock.rs` is a complete example.
///
/// # Errors
///
/// - the task name is empty or not snake case;
/// - `db` is not a PostgreSQL pool, or a pool of fewer than
///   [`MIN_POOL_CONNECTIONS`] connections;
/// - the holder transaction cannot be opened or the lock cannot be queried;
/// - `work` fails: its error is returned unchanged, after the lock is
///   released. This is the only error after which `work` has run.
pub async fn with_task_lock<T, F, Fut>(
    db: &DatabaseConnection,
    task_name: &str,
    work: F,
) -> Result<TaskLockOutcome<T>>
where
    F: FnOnce() -> Fut,
    Fut: Future<Output = Result<T>>,
{
    let key = task_lock_key(task_name).map_err(Error::msg)?;
    check_pool(db, task_name)?;

    let holder = db.begin().await?;
    if !try_lock(&holder, key).await? {
        tracing::info!(
            task = task_name,
            lock_key = key,
            "another run of this task holds its lock; skipping this run (ADR 0016 §4)"
        );
        holder.rollback().await?;
        return Ok(TaskLockOutcome::AlreadyRunning);
    }

    let outcome = work().await;
    // A failed rollback means the holder's session ended before the run did:
    // logged, not returned (see the module documentation).
    if let Err(error) = holder.rollback().await {
        tracing::warn!(
            task = task_name,
            lock_key = key,
            %error,
            "the task lock's session ended during the run, so the lock was released early \
             and another run may have overlapped this one"
        );
    }
    outcome.map(TaskLockOutcome::Ran)
}

/// Refuses a connection that is not a PostgreSQL pool of at least
/// [`MIN_POOL_CONNECTIONS`] connections. With one connection, the holder
/// would keep it and the work's first query would wait for the pool until it
/// timed out.
fn check_pool(db: &DatabaseConnection, task_name: &str) -> Result<()> {
    // `get_postgres_connection_pool` panics on any other kind of connection.
    if !matches!(
        db.inner,
        DatabaseConnectionType::SqlxPostgresPoolConnection(_)
    ) {
        return Err(Error::Message(format!(
            "task {task_name}: the task lock needs a PostgreSQL connection pool (ADR 0016 §4)"
        )));
    }
    let pool = db.get_postgres_connection_pool();
    let max_connections = pool.options().get_max_connections();
    if max_connections < MIN_POOL_CONNECTIONS {
        return Err(Error::Message(format!(
            "task {task_name}: the task lock needs a database pool of at least \
             {MIN_POOL_CONNECTIONS} connections, one to hold the lock and one for the work, \
             but database.max_connections is {max_connections}; raise it (ADR 0016 §4)"
        )));
    }
    Ok(())
}

/// Tries to take the lock for `key` in `holder`, without waiting.
async fn try_lock(holder: &DatabaseTransaction, key: i64) -> Result<bool> {
    let sql = "SELECT pg_try_advisory_xact_lock($1)";
    let statement = Statement::from_sql_and_values(DbBackend::Postgres, sql, [key.into()]);
    let row = holder.query_one_raw(statement).await?;
    let row = row.ok_or_else(|| Error::string("pg_try_advisory_xact_lock returned no row"))?;
    Ok(row.try_get_by_index::<bool>(0)?)
}
