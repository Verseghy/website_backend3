//! `models::job_queue`: the per-status summary of Loco's queue table that
//! `/metrics` reads (ADR 0016 §3, ADR 0023 §9), and the server-side time
//! limit that keeps a blocked read from holding a connection.

use std::time::{Duration, Instant};

use backend3::{
    domain::runtime_metrics::pool_usage,
    models::job_queue::{JobsByStatus, jobs_by_status},
};
use sea_orm::{ConnectionTrait, DbBackend, DbErr, Statement};
use tokio::time::timeout;

use crate::support::{QueueTableLock, create_queue_table, enqueue_job, with_app};

/// A limit that a healthy query never reaches.
const GENEROUS: Duration = Duration::from_secs(30);

/// Sets the status of one job, as a worker or `jobs cancel` would.
async fn set_status(ctx: &loco_rs::app::AppContext, id: &str, status: &str) {
    ctx.db
        .execute_raw(Statement::from_sql_and_values(
            DbBackend::Postgres,
            "UPDATE pg_loco_queue SET status = $1 WHERE id = $2",
            [status.into(), id.into()],
        ))
        .await
        .expect("update the job");
}

/// The `statement_timeout` of a connection of `ctx`'s pool.
async fn session_statement_timeout(ctx: &loco_rs::app::AppContext) -> String {
    let row = ctx
        .db
        .query_one_raw(Statement::from_string(
            DbBackend::Postgres,
            "SELECT current_setting('statement_timeout') AS value",
        ))
        .await
        .expect("query")
        .expect("one row");
    row.try_get("", "value").expect("a text value")
}

#[tokio::test]
async fn a_database_without_the_queue_table_is_an_error() {
    with_app(|_router, ctx| async move {
        let error = jobs_by_status(&ctx.db, GENEROUS)
            .await
            .expect_err("no pg_loco_queue");
        assert!(error.to_string().contains("pg_loco_queue"), "{error}");
    })
    .await;
}

#[tokio::test]
async fn an_empty_queue_has_no_rows() {
    with_app(|_router, ctx| async move {
        create_queue_table(&ctx).await;
        assert_eq!(jobs_by_status(&ctx.db, GENEROUS).await.expect("query"), []);
    })
    .await;
}

#[tokio::test]
async fn jobs_are_counted_by_status_and_only_due_jobs_have_an_age() {
    with_app(|_router, ctx| async move {
        create_queue_table(&ctx).await;
        enqueue_job(&ctx, -90).await;
        enqueue_job(&ctx, -10).await;
        // Scheduled for later: counted, but not waiting yet.
        enqueue_job(&ctx, 3600).await;
        let failed = enqueue_job(&ctx, -500).await;
        set_status(&ctx, &failed, "failed").await;

        let mut rows = jobs_by_status(&ctx.db, GENEROUS).await.expect("query");
        rows.sort_by(|a, b| a.status.cmp(&b.status));
        let [failed_row, queued_row] = rows.as_slice() else {
            panic!("one row per status: {rows:?}");
        };
        assert_eq!((failed_row.status.as_str(), failed_row.jobs), ("failed", 1));
        let JobsByStatus {
            status,
            jobs,
            oldest_due_age_seconds,
        } = queued_row;
        assert_eq!((status.as_str(), *jobs), ("queued", 3));
        // The 90-second-old job is the oldest due one; the bound leaves
        // room for a slow test machine.
        let age = oldest_due_age_seconds.expect("two queued jobs are due");
        assert!((90.0..150.0).contains(&age), "{age}");
    })
    .await;
}

#[tokio::test]
async fn queued_jobs_that_are_all_scheduled_for_later_have_no_age() {
    with_app(|_router, ctx| async move {
        create_queue_table(&ctx).await;
        enqueue_job(&ctx, 600).await;
        let rows = jobs_by_status(&ctx.db, GENEROUS).await.expect("query");
        assert_eq!(
            rows,
            [JobsByStatus {
                status: "queued".to_owned(),
                jobs: 1,
                oldest_due_age_seconds: None,
            }]
        );
    })
    .await;
}

#[tokio::test]
async fn a_time_limit_outside_what_postgresql_accepts_is_refused() {
    with_app(|_router, ctx| async move {
        create_queue_table(&ctx).await;
        // 0 would turn the limit off; above i32::MAX ms PostgreSQL refuses it.
        for limit in [
            Duration::ZERO,
            Duration::from_micros(999),
            Duration::from_millis(u64::from(u32::MAX)),
        ] {
            let error = jobs_by_status(&ctx.db, limit).await.expect_err("refused");
            assert!(
                matches!(&error, DbErr::Custom(message)
                    if message.contains("between 1 ms and 2147483647 ms")),
                "{error}"
            );
        }
    })
    .await;
}

#[tokio::test]
async fn the_server_cancels_a_blocked_read_at_the_time_limit() {
    with_app(|_router, ctx| async move {
        create_queue_table(&ctx).await;
        let lock = QueueTableLock::take(&ctx).await;

        let started = Instant::now();
        let error = timeout(
            Duration::from_secs(5),
            jobs_by_status(&ctx.db, Duration::from_millis(200)),
        )
        .await
        .expect("PostgreSQL ends the read before the test does")
        .expect_err("cancelled");
        assert!(error.to_string().contains("statement timeout"), "{error}");
        assert!(
            started.elapsed() < Duration::from_secs(2),
            "{:?}",
            started.elapsed()
        );
        // The limit was local to the read's transaction.
        assert_eq!(session_statement_timeout(&ctx).await, "0");

        lock.release().await;
        assert_eq!(jobs_by_status(&ctx.db, GENEROUS).await.expect("query"), []);
    })
    .await;
}

#[tokio::test]
async fn a_read_the_caller_abandons_gives_its_connection_back() {
    with_app(|_router, ctx| async move {
        create_queue_table(&ctx).await;
        let lock = QueueTableLock::take(&ctx).await;
        // The caller gives up first; the server's limit then ends the query
        // that sqlx is still waiting for in the background.
        let abandoned = timeout(
            Duration::from_millis(100),
            jobs_by_status(&ctx.db, Duration::from_millis(300)),
        )
        .await;
        assert!(abandoned.is_err(), "the caller timed out: {abandoned:?}");

        // While the lock is still held, so only the server's limit can have
        // freed the connection.
        let pool = ctx.db.get_postgres_connection_pool();
        timeout(Duration::from_secs(5), async {
            while pool_usage(pool.size(), pool.num_idle()).used > 0 {
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .expect("the connection is back in the pool within 5 s");
        // It came back outside any transaction and without the limit.
        assert_eq!(session_statement_timeout(&ctx).await, "0");
        lock.release().await;
    })
    .await;
}
