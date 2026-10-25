//! Tests of `backend3::models::task_lock::with_task_lock` (ADR 0016 §4).
//!
//! The lock is a PostgreSQL advisory lock, and only a real server shows how
//! it behaves across sessions, so the tests run on Loco's per-test
//! databases. Another run of a task is played by another session: a manual
//! run and a `CronJob` pod are two sessions too. The test pool has two
//! connections, the smallest the helper accepts.

use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::Duration,
};

use async_trait::async_trait;
use backend3::{
    domain::task_lock_key::task_lock_key,
    models::task_lock::{MIN_POOL_CONNECTIONS, TaskLockOutcome, with_task_lock},
};
use loco_rs::{
    Error, Result,
    app::AppContext,
    db,
    task::{Task, TaskInfo, Tasks, Vars},
};
use sea_orm::{ConnectionTrait, DatabaseConnection, DbBackend, Statement, TransactionTrait};
use tokio::{sync::Notify, time};

use crate::support::{hold_task_lock, with_app};

/// The task name the tests lock under.
const TASK: &str = "lock_probe";

/// Runs a query that returns one value of type `T`.
async fn query_one<T: sea_orm::TryGetable>(
    db: &impl ConnectionTrait,
    sql: &str,
    values: Vec<sea_orm::Value>,
) -> T {
    let statement = Statement::from_sql_and_values(DbBackend::Postgres, sql, values);
    let row = db.query_one_raw(statement).await;
    let row = row.unwrap_or_else(|error| panic!("{sql}: {error}"));
    row.expect("one row").try_get_by_index::<T>(0).expect(sql)
}

/// Computes `aggregate` over the sessions (`pid`) that hold an advisory lock
/// in the current database; only the helper and the probes take one there.
async fn over_lock_holders(db: &DatabaseConnection, aggregate: &str) -> i64 {
    let sql = format!(
        "SELECT {aggregate} FROM pg_locks AS holder WHERE locktype = 'advisory' AND granted \
         AND database = (SELECT oid FROM pg_database WHERE datname = current_database())"
    );
    query_one(db, &sql, vec![]).await
}

/// Whether another session could take the lock of [`TASK`] now. The probe's
/// transaction is rolled back at once, so it keeps nothing it took.
async fn lock_is_free(db: &DatabaseConnection) -> bool {
    let key = task_lock_key(TASK).expect("a snake_case name");
    let probe = db.begin().await.expect("open a probe transaction");
    let sql = "SELECT pg_try_advisory_xact_lock($1)";
    let taken = query_one::<bool>(&probe, sql, vec![key.into()]).await;
    probe.rollback().await.expect("end the probe transaction");
    taken
}

/// A task that uses the helper as every task does (recipe 6).
struct ProbeTask {
    /// Set when the task's work runs.
    ran: Arc<AtomicBool>,
}

#[async_trait]
impl Task for ProbeTask {
    fn task(&self) -> TaskInfo {
        TaskInfo {
            name: TASK.to_owned(),
            detail: "Records whether its work ran".to_owned(),
        }
    }

    async fn run(&self, ctx: &AppContext, _vars: &Vars) -> Result<()> {
        with_task_lock(&ctx.db, &self.task().name, || async {
            self.ran.store(true, Ordering::SeqCst);
            Ok(())
        })
        .await?;
        Ok(())
    }
}

/// The lock-held test every task copies (recipe 6): Loco's CLI exits 0 when
/// the task runner returns `Ok` (loco-rs 1.2.0 `cli.rs`). A real task runs
/// through `loco_rs::boot::run_task::<App>`; the probe is not registered in
/// `App`, so it runs through a registry of its own, on the same `Tasks::run`.
#[tokio::test]
async fn a_task_exits_successfully_without_its_work_while_another_run_holds_the_lock() {
    with_app(|_router, ctx| async move {
        let ran = Arc::new(AtomicBool::new(false));
        let mut tasks = Tasks::default();
        tasks.register(ProbeTask { ran: ran.clone() });

        let other_run = hold_task_lock(&ctx.db, TASK).await;
        let held = tasks.run(&ctx, TASK, &Vars::default()).await;
        held.expect("exit 0 while the lock is held");
        assert!(!ran.load(Ordering::SeqCst), "the work must not run");

        other_run.rollback().await.expect("release the other run");
        let free = tasks.run(&ctx, TASK, &Vars::default()).await;
        free.expect("a free lock runs the task");
        assert!(ran.load(Ordering::SeqCst), "the work ran when free");
    })
    .await;
}

/// One of two concurrent runs: its work counts itself, then waits until the
/// other run has returned, so the two calls overlap for certain. Had both
/// taken the lock, each would wait for the other, and the time limit fails
/// the test.
async fn concurrent_run(
    db: &DatabaseConnection,
    runs: &AtomicUsize,
    other_returned: &Notify,
    this_returned: &Notify,
) -> Result<TaskLockOutcome<()>> {
    let outcome = with_task_lock(db, TASK, || async {
        runs.fetch_add(1, Ordering::SeqCst);
        other_returned.notified().await;
        Ok(())
    })
    .await;
    // `notify_one` keeps a permit when nobody waits yet, so the signal is not
    // lost if the other work starts waiting later.
    this_returned.notify_one();
    outcome
}

#[tokio::test]
async fn two_concurrent_calls_run_the_work_once() {
    with_app(|_router, ctx| async move {
        let runs = AtomicUsize::new(0);
        let (first_returned, second_returned) = (Notify::new(), Notify::new());

        let both = time::timeout(Duration::from_secs(30), async {
            tokio::join!(
                concurrent_run(&ctx.db, &runs, &second_returned, &first_returned),
                concurrent_run(&ctx.db, &runs, &first_returned, &second_returned),
            )
        });
        let (first, second) = both.await.expect("a hang means both took the lock");

        let mut outcomes = [first.expect("first call"), second.expect("second call")];
        outcomes.sort_by_key(|outcome| matches!(outcome, TaskLockOutcome::AlreadyRunning));
        let expected = [TaskLockOutcome::Ran(()), TaskLockOutcome::AlreadyRunning];
        assert_eq!(outcomes, expected);
        assert_eq!(runs.load(Ordering::SeqCst), 1, "the work ran once");
    })
    .await;
}

#[tokio::test]
async fn the_lock_is_held_for_the_whole_run_and_released_after_it() {
    with_app(|_router, ctx| async move {
        let during = with_task_lock(&ctx.db, TASK, || async {
            let holders = over_lock_holders(&ctx.db, "count(*)").await;
            Ok((holders, lock_is_free(&ctx.db).await))
        })
        .await;

        let during = during.expect("the run succeeds");
        assert_eq!(during, TaskLockOutcome::Ran((1, false)), "one holder");
        assert!(lock_is_free(&ctx.db).await, "released after the run");
        // A session-level lock would stay with the holder's connection after
        // the pool took it back; a transaction-level lock leaves nothing.
        assert_eq!(over_lock_holders(&ctx.db, "count(*)").await, 0);
    })
    .await;
}

#[tokio::test]
async fn a_failing_work_returns_its_error_and_releases_the_lock() {
    with_app(|_router, ctx| async move {
        let failing = || async { Err::<(), _>(Error::string("the work failed")) };
        let failed = with_task_lock(&ctx.db, TASK, failing).await;
        let error = failed.expect_err("the work failed");

        assert_eq!(error.to_string(), "the work failed");
        assert!(lock_is_free(&ctx.db).await, "released after the failure");
        assert_eq!(over_lock_holders(&ctx.db, "count(*)").await, 0);
    })
    .await;
}

#[tokio::test]
async fn a_lock_lost_during_the_run_still_returns_the_works_result() {
    with_app(|_router, ctx| async move {
        let outcome = with_task_lock(&ctx.db, TASK, || async {
            // Ends the holder's session as a server restart would, and waits
            // until it has exited.
            let terminate = "count(*) FILTER (WHERE pg_terminate_backend(pid, 10000))";
            Ok(over_lock_holders(&ctx.db, terminate).await)
        })
        .await;

        let outcome = outcome.expect("the work's result, not the failed release");
        assert_eq!(outcome, TaskLockOutcome::Ran(1), "one holder terminated");
        assert!(lock_is_free(&ctx.db).await);
    })
    .await;
}

#[tokio::test]
async fn a_bad_name_or_a_connection_that_is_not_a_postgres_pool_is_refused() {
    let disconnected = DatabaseConnection::default();
    let ran = AtomicBool::new(false);
    let work = || async {
        ran.store(true, Ordering::SeqCst);
        Ok(())
    };

    let bad_name = with_task_lock(&disconnected, "Lock-Probe", work).await;
    let message = bad_name.expect_err("refused").to_string();
    let expected = "\"Lock-Probe\" is not snake_case";
    assert!(message.contains(expected), "{message}");

    let not_postgres = with_task_lock(&disconnected, TASK, work).await;
    let message = not_postgres.expect_err("refused").to_string();
    assert!(message.contains("task lock_probe"), "{message}");
    assert!(message.contains("PostgreSQL connection pool"), "{message}");
    assert!(!ran.load(Ordering::SeqCst), "neither runs the work");
}

#[tokio::test]
async fn a_pool_of_one_connection_is_refused() {
    with_app(|_router, ctx| async move {
        let mut config = ctx.config.database.clone();
        config.min_connections = 1;
        config.max_connections = MIN_POOL_CONNECTIONS - 1;
        let pool = db::connect(&config).await.expect("connect as Loco does");
        let refused = with_task_lock(&pool, TASK, || async { Ok(()) }).await;
        // Closed before any assertion can panic: `with_app` drops the
        // per-test database only once no pool is connected to it.
        pool.close_by_ref().await.expect("close the extra pool");

        let message = refused.expect_err("refused").to_string();
        assert!(message.contains("task lock_probe"), "{message}");
        assert!(message.contains("at least 2 connections"), "{message}");
        assert!(message.contains("max_connections is 1"), "{message}");
    })
    .await;
}
