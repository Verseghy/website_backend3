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
use chrono::{DateTime, TimeDelta, Utc};
use futures_util::FutureExt;
use loco_rs::{app::AppContext, testing::prelude::*};
use sea_orm::{
    ConnectionTrait, DatabaseConnection, DatabaseTransaction, DbBackend, Statement,
    TransactionTrait,
};

/// The admin UI fixture of `config/test.yaml` (ADR 0004 §4).
pub const FIXTURE_DIST: &str = "tests/fixtures/admin-dist";
/// The one hashed asset in the fixture.
pub const FIXTURE_ASSET: &str = "assets/index-B7xKq2Lm.js";

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
