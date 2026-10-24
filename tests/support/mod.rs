//! Helpers shared by the integration tests.

use std::{
    io::{self, Read},
    net::{SocketAddr, TcpListener},
    panic::AssertUnwindSafe,
    path::PathBuf,
    process::{Command, ExitStatus, Stdio},
    thread,
    time::{Duration, Instant},
};

use axum::{ServiceExt, extract::Request};
use axum_test::TestServer;
use backend3::app::{App, trim_trailing_slash};
use futures_util::FutureExt;
use loco_rs::{app::AppContext, testing::prelude::*};

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
