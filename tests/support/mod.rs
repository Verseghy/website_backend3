//! Helpers shared by the integration tests.

use std::{
    io::{self, Read},
    net::{SocketAddr, TcpListener},
    panic::AssertUnwindSafe,
    path::PathBuf,
    process::{Command, ExitStatus, Stdio},
    sync::{Arc, Mutex, MutexGuard, PoisonError},
    thread,
    time::{Duration, Instant},
};

use axum::{ServiceExt, extract::Request};
use axum_test::TestServer;
use backend3::{
    app::{App, set_clock, trim_trailing_slash},
    domain::{clock::Clock, task_lock_key::task_lock_key},
};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use chrono::{DateTime, TimeDelta, Utc};
use futures_util::FutureExt;
use loco_rs::{app::AppContext, testing::prelude::*};
use sea_orm::{
    ConnectionTrait, DatabaseConnection, DatabaseTransaction, DbBackend, Statement,
    TransactionTrait, prelude::Uuid,
};

/// The admin UI fixture of `config/test.yaml` (ADR 0004 §4).
pub const FIXTURE_DIST: &str = "tests/fixtures/admin-dist";
/// The one hashed asset in the fixture.
pub const FIXTURE_ASSET: &str = "assets/index-B7xKq2Lm.js";

/// `JWT_SECRET` for the child processes that load config/production.yaml: a
/// throwaway test value, the base64 of the sentence "backend3 CLI test JWT
/// secret: public and throwaway, only for child processes.", which ADR 0034
/// §2 allows. Production must refuse every published key (ADR 0034 §3), so
/// the `secret-guards` slice replaces it with a key made at run time.
pub const CLI_JWT_SECRET: &str = "YmFja2VuZDMgQ0xJIHRlc3QgSldUIHNlY3JldDogcHVibGljIGFuZCB0aHJvd2F3YXksIG9ubHkgZm9yIGNoaWxkIHByb2Nlc3Nlcy4=";

/// The bytes of a file in the admin UI fixture.
pub fn fixture(relative: &str) -> Vec<u8> {
    std::fs::read(repository_root().join(FIXTURE_DIST).join(relative))
        .unwrap_or_else(|error| panic!("fixture {relative}: {error}"))
}

/// The package root, which is also the working directory of the tests.
pub fn repository_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// A TCP listener that never accepts on its own, used to prove whether a
/// process tried to connect to a database address.
///
/// The kernel completes the TCP handshake into the listen backlog even though
/// nobody calls `accept`, so after the process under test has exited a
/// non-blocking `accept` tells whether it ever opened a connection.
pub struct ConnectionProbe {
    listener: TcpListener,
}

impl ConnectionProbe {
    /// Binds a probe on an ephemeral port of `address`, for example
    /// `127.0.0.1` or `127.0.0.2` (any 127.0.0.0/8 address works on Linux).
    pub fn bind(address: &str) -> Self {
        let listener = TcpListener::bind((address, 0))
            .unwrap_or_else(|error| panic!("bind probe on {address}: {error}"));
        listener.set_nonblocking(true).expect("non-blocking probe");
        Self { listener }
    }

    /// The port the probe listens on.
    pub fn port(&self) -> u16 {
        self.listener.local_addr().expect("probe address").port()
    }

    /// Whether any connection reached the probe so far.
    pub fn was_connected(&self) -> bool {
        match self.listener.accept() {
            Ok(_) => true,
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => false,
            Err(error) => panic!("probe accept: {error}"),
        }
    }
}

/// The outcome of one `backend3-cli` run.
pub struct CliRun {
    /// The exit status.
    pub status: ExitStatus,
    /// Everything the process wrote to stderr.
    pub stderr: String,
}

/// Runs the built `backend3-cli` with exactly the given environment (the
/// test's own environment is not inherited) and the package root as working
/// directory, and waits at most `timeout`.
pub fn run_cli(env: &[(&str, &str)], args: &[&str], timeout: Duration) -> CliRun {
    let mut child = Command::new(env!("CARGO_BIN_EXE_backend3-cli"))
        .env_clear()
        .envs(env.iter().copied())
        .args(args)
        .current_dir(repository_root())
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn backend3-cli");

    let deadline = Instant::now() + timeout;
    let status = loop {
        if let Some(status) = child.try_wait().expect("wait for backend3-cli") {
            break status;
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            panic!("backend3-cli {args:?} did not exit within {timeout:?}");
        }
        thread::sleep(Duration::from_millis(20));
    };

    let mut stderr = String::new();
    child
        .stderr
        .take()
        .expect("piped stderr")
        .read_to_string(&mut stderr)
        .expect("read stderr");
    CliRun { status, stderr }
}

/// A `TOTP_ENCRYPTION_KEY` for the child processes that load
/// config/production.yaml ([`run_cli`]): the base64 of 64 bytes made when
/// the test runs, a new key on every call.
///
/// ADR 0034 §3: production refuses every key that this repository holds,
/// and a test that starts the production configuration uses a key that no
/// committed file holds. A fixed value here would be such a key that
/// production accepts. Tests that need a key production refuses use
/// `backend3::domain::totp_secret::PUBLISHED_LOCAL_KEY` instead.
///
/// The bytes are four version 4 UUIDs, from the generator of the uuid crate
/// that Loco already builds, so the tests need no random crate of their
/// own. Six bits of each UUID are fixed (version and variant), which leaves
/// 488 random bits: plenty for a key that only has to be new and unknown.
pub fn cli_totp_key() -> String {
    let bytes: Vec<u8> = (0..4).flat_map(|_| Uuid::new_v4().into_bytes()).collect();
    STANDARD.encode(bytes)
}

/// Boots the app on its own fresh database (Loco's per-test databases), runs
/// `test` with the full router and the app context, drops the database
/// afterwards, also when `test` panics, and returns what `test` returned.
///
/// Loco 1.2.0 drops the per-test database when its boot result is dropped,
/// with a plain `DROP DATABASE` that PostgreSQL refuses while the app's pool
/// still holds connections (loco-rs 1.2.0 `testing/db.rs`, `cleanup_db`).
/// This helper closes the pool first, so the database is always removed.
pub async fn with_app<F, Fut, T>(test: F) -> T
where
    F: FnOnce(axum::Router, AppContext) -> Fut,
    Fut: Future<Output = T>,
{
    let boot = boot_test_with_create_db::<App>()
        .await
        .expect("boot the test app on a fresh database");
    let router = boot.router.clone().expect("server mode builds a router");
    let outcome = AssertUnwindSafe(test(router, boot.app_context.clone()))
        .catch_unwind()
        .await;
    boot.app_context
        .db
        .close_by_ref()
        .await
        .expect("close the database pool");
    drop(boot);
    match outcome {
        Ok(value) => value,
        Err(panic) => std::panic::resume_unwind(panic),
    }
}

/// Like [`with_app`], with Loco's test server in front of the router, so
/// requests pass the whole middleware stack.
///
/// The router is wrapped exactly as `App::serve` wraps it in production
/// ([`backend3::app::trim_trailing_slash`]), so request tests see the
/// trailing-slash normalisation too (public-api.md DEC-10).
pub async fn with_server<F, Fut, T>(test: F) -> T
where
    F: FnOnce(TestServer, AppContext) -> Fut,
    Fut: Future<Output = T>,
{
    with_app(|router, ctx| async move {
        let server = TestServer::new_with_config(
            ServiceExt::<Request>::into_make_service_with_connect_info::<SocketAddr>(
                trim_trailing_slash(router),
            ),
            RequestConfig::default(),
        )
        .expect("start the test server");
        test(server, ctx).await
    })
    .await
}

/// Takes the advisory lock of the task `task_name` on a transaction of its
/// own and returns it, playing another run of the task that is still going
/// (ADR 0016 §4). The lock is held until the transaction ends or is dropped.
///
/// - `db`: the app's pool (`ctx.db`); the task under test runs on its other
///   connection.
/// - `task_name`: the name the task registers in its `TaskInfo`. Every
///   task's lock-held test locks by that name, so a task that locked under
///   any other name would run its work and fail the test.
pub async fn hold_task_lock(db: &DatabaseConnection, task_name: &str) -> DatabaseTransaction {
    let key = task_lock_key(task_name).unwrap_or_else(|error| panic!("{error}"));
    let holder = db.begin().await.expect("open the holding transaction");
    let sql = "SELECT pg_try_advisory_xact_lock($1)";
    let statement = Statement::from_sql_and_values(DbBackend::Postgres, sql, [key.into()]);
    let row = holder
        .query_one_raw(statement)
        .await
        .expect("query the task lock");
    let taken = row
        .expect("one row")
        .try_get_by_index::<bool>(0)
        .expect(sql);
    assert!(taken, "the lock of task {task_name} is already held");
    holder
}

/// The app clock of a time-dependent test (public-api.md §16.2): it stands
/// still at the instant the test chose and moves only when the test moves
/// it. [`with_app_at`] and [`with_server_at`] install one and hand it over.
#[derive(Debug)]
pub struct TestClock {
    now: Mutex<DateTime<Utc>>,
}

impl TestClock {
    /// A clock that stands at `start`.
    pub fn new(start: DateTime<Utc>) -> Self {
        Self {
            now: Mutex::new(start),
        }
    }

    /// Moves the clock to `instant`, forwards or backwards.
    pub fn set(&self, instant: DateTime<Utc>) {
        *self.lock() = instant;
    }

    /// Moves the clock forward by `step` (zero keeps it) and returns the new
    /// instant.
    ///
    /// # Panics
    ///
    /// On a negative `step` (a test that goes back says so with
    /// [`TestClock::set`]), and past the end of chrono's range.
    pub fn advance(&self, step: TimeDelta) -> DateTime<Utc> {
        assert!(
            step >= TimeDelta::zero(),
            "TestClock::advance moves forward only, and got {step}; use TestClock::set to move \
             the clock back"
        );
        let mut now = self.lock();
        *now = now.checked_add_signed(step).unwrap_or_else(|| {
            panic!(
                "TestClock::advance: {} plus {step} is outside chrono's range",
                *now
            )
        });
        *now
    }

    /// The guarded instant. Every write is one assignment, so a lock poisoned
    /// by a panicking test still holds a whole `DateTime` and is taken over.
    fn lock(&self) -> MutexGuard<'_, DateTime<Utc>> {
        self.now.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

impl Clock for TestClock {
    fn now(&self) -> DateTime<Utc> {
        *self.lock()
    }
}

/// Replaces the app clock of `ctx` with a [`TestClock`] at `start`.
fn install_test_clock(ctx: &AppContext, start: DateTime<Utc>) -> Arc<TestClock> {
    let clock = Arc::new(TestClock::new(start));
    set_clock(ctx, clock.clone());
    clock
}

/// Like [`with_app`], with the app clock at `start` until the test moves the
/// [`TestClock`] it receives with the router and the context.
///
/// - `start`: what `backend3::app::clock(&ctx)` answers when the test begins.
/// - `test`: the test body.
///
/// The clock replaces the system clock after boot, in the store that every
/// copy of the context shares, so routes, workers and tasks read it; code
/// that runs during boot (hooks, initializers) still sees the system clock.
pub async fn with_app_at<F, Fut, T>(start: DateTime<Utc>, test: F) -> T
where
    F: FnOnce(axum::Router, AppContext, Arc<TestClock>) -> Fut,
    Fut: Future<Output = T>,
{
    with_app(|router, ctx| async move {
        let clock = install_test_clock(&ctx, start);
        test(router, ctx, clock).await
    })
    .await
}

/// Like [`with_server`], with the test clock of [`with_app_at`].
///
/// - `start`: what `backend3::app::clock(&ctx)` answers when the test begins.
/// - `test`: the test body; it gets the server, the context and the clock.
pub async fn with_server_at<F, Fut, T>(start: DateTime<Utc>, test: F) -> T
where
    F: FnOnce(TestServer, AppContext, Arc<TestClock>) -> Fut,
    Fut: Future<Output = T>,
{
    with_server(|server, ctx| async move {
        let clock = install_test_clock(&ctx, start);
        test(server, ctx, clock).await
    })
    .await
}

/// Creates Loco's job queue table, `pg_loco_queue`, in the test's database
/// with Loco's own set-up function, as a process with `queue.kind: Postgres`
/// does at startup. `config/test.yaml` configures no queue (jobs run
/// inline), so a per-test database has no queue table until this runs.
///
/// - `ctx`: the test's context; the table goes into its database.
pub async fn create_queue_table(ctx: &AppContext) {
    loco_rs::bgworker::pg::initialize_database(ctx.db.get_postgres_connection_pool())
        .await
        .expect("create pg_loco_queue");
}

/// Enqueues one job through Loco's Postgres queue, due `due_in_seconds`
/// from now (negative: overdue), and returns its id. No worker runs in the
/// tests, so it stays queued like a job in a process that runs no worker.
///
/// - `ctx`: the test's context; the job goes into its database.
/// - `due_in_seconds`: when the job becomes due, relative to "now".
///
/// "Now" is PostgreSQL's `now()`, not the wall clock of the test process:
/// the queue query measures the age of a due job with the database's clock
/// (`src/models/job_queue.rs`), and recipe 7 of the project skill keeps
/// wall-clock reads out of the tests.
pub async fn enqueue_job(ctx: &AppContext, due_in_seconds: i64) -> String {
    let sql = "SELECT now()";
    let row = ctx
        .db
        .query_one_raw(Statement::from_string(DbBackend::Postgres, sql))
        .await
        .expect("read the database clock");
    let now = row
        .expect("one row")
        .try_get_by_index::<DateTime<Utc>>(0)
        .expect(sql);
    let run_at = now + TimeDelta::seconds(due_in_seconds);
    loco_rs::bgworker::pg::enqueue(
        ctx.db.get_postgres_connection_pool(),
        "TestWorker",
        serde_json::json!({ "id": 1 }),
        run_at,
        None,
        None,
        None,
    )
    .await
    .expect("enqueue a job")
}

/// A second session on the test's database that holds an `ACCESS EXCLUSIVE`
/// lock on `pg_loco_queue`, as a migration's `ALTER TABLE` would, so every
/// query of the table waits for it. The session has a connection of its
/// own, outside the application's pool.
pub struct QueueTableLock {
    session: DatabaseConnection,
    transaction: DatabaseTransaction,
}

impl QueueTableLock {
    /// Takes the lock; the table must exist ([`create_queue_table`]).
    ///
    /// - `ctx`: the test's context; the lock is on its database.
    pub async fn take(ctx: &AppContext) -> Self {
        let session = sea_orm::Database::connect(ctx.config.database.uri.as_str())
            .await
            .expect("open a second session");
        let transaction = session.begin().await.expect("begin");
        transaction
            .execute_unprepared("LOCK TABLE pg_loco_queue IN ACCESS EXCLUSIVE MODE")
            .await
            .expect("lock pg_loco_queue");
        Self {
            session,
            transaction,
        }
    }

    /// Releases the lock and closes the session, which `with_app` needs
    /// before it can drop the database.
    pub async fn release(self) {
        self.transaction.rollback().await.expect("release the lock");
        self.session
            .close()
            .await
            .expect("close the second session");
    }
}
