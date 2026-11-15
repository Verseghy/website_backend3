//! Helpers for model tests that run two saves at the same time, on one
//! record or on one value that a unique index guards (ADR 0011 §4; the
//! write contract, recipe 12 of the project skill).
//!
//! Such a test needs more sessions than the test configuration's pool
//! holds (two), so it opens a pool of its own ([`with_second_pool`]). And
//! it must not decide the order of the two saves with a timer: the first
//! save is committed or rolled back only once the database shows the
//! second one waiting for its lock ([`until_one_waits`],
//! [`commit_once_one_waits`]), so the test checks what happens to a save
//! that really waited, on a slow machine too.

use std::{future::Future, panic::AssertUnwindSafe, time::Duration};

use futures_util::FutureExt;
use sea_orm::{
    ConnectOptions, ConnectionTrait, Database, DatabaseConnection, DatabaseTransaction, DbBackend,
    Statement,
};

/// Runs `test` with a pool of its own on the test's database, with room for
/// `connections` sessions at once (the test configuration's pool has two),
/// and closes that pool afterwards, also when `test` panics: `with_app`
/// can drop the test's database only once no session is left on it.
///
/// - `uri`: the test's database, `ctx.config.database.uri`.
/// - `connections`: the pool's size.
/// - `test`: the test body; it gets the pool.
pub async fn with_second_pool<F, Fut>(uri: &str, connections: u32, test: F)
where
    F: FnOnce(DatabaseConnection) -> Fut,
    Fut: Future<Output = ()>,
{
    let mut options = ConnectOptions::new(uri.to_owned());
    options
        .max_connections(connections)
        .min_connections(1)
        .sqlx_logging(false);
    let pool = Database::connect(options)
        .await
        .expect("open a second pool");
    let outcome = AssertUnwindSafe(test(pool.clone())).catch_unwind().await;
    pool.close().await.expect("close the second pool");
    if let Err(panic) = outcome {
        std::panic::resume_unwind(panic);
    }
}

/// How many sessions of the test's database wait for another
/// transaction's row lock or its uncommitted row: a save of a locked row,
/// and an insert or update that meets an uncommitted entry of a unique
/// index, both wait on the other transaction's id (`transactionid`) or on
/// the row (`tuple`).
///
/// - `db`: a connection to the test's database, outside the pool whose
///   sessions take part.
async fn row_lock_waiters(db: &DatabaseConnection) -> i64 {
    let sql = "SELECT count(*) FROM pg_stat_activity WHERE datname = current_database() \
               AND wait_event_type = 'Lock' AND wait_event IN ('transactionid', 'tuple')";
    db.query_one_raw(Statement::from_string(DbBackend::Postgres, sql))
        .await
        .expect(sql)
        .expect("one row")
        .try_get_by_index(0)
        .expect(sql)
}

/// Polls until one session of the test's database waits for another
/// transaction's lock; panics after about five seconds.
///
/// - `db`: a connection to the test's database for the poll, outside the
///   pool whose sessions take part.
pub async fn until_one_waits(db: &DatabaseConnection) {
    for _ in 0..500 {
        if row_lock_waiters(db).await == 1 {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("no session ever waited for the first transaction's lock");
}

/// Polls until one session of the test's database waits for a row lock,
/// then commits `first`, the transaction it waits for; panics after about
/// five seconds.
///
/// - `db`: a connection to the test's database for the poll, outside the
///   pool whose sessions take part.
/// - `first`: the transaction that holds the lock.
pub async fn commit_once_one_waits(db: &DatabaseConnection, first: DatabaseTransaction) {
    until_one_waits(db).await;
    first.commit().await.expect("commit the first transaction");
}
