//! The import's source query layer (`tasks::import_mysql::source`,
//! data-migration.md §3.3): the allow-list refusal before anything is sent,
//! reads through the layer against the MySQL source of compose.yaml, and
//! that neither its errors nor its log lines show the source URL
//! (data-migration.md §4.1).
//!
//! The tests marked "compose source" read the MySQL that compose.yaml
//! starts under the profile `import` (`podman-compose --profile import up
//! -d mysql`; the CI Test job starts it before the tests), with the legacy
//! schema of `tests/fixtures/legacy/schema.sql` and no rows, through its
//! account `loco`, which has the cut-over account's grants. They only read,
//! so they run in parallel on the one source.

use std::{
    fmt::{self, Write as _},
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};

use backend3::{
    domain::import_mysql::{
        source::SourceDatabaseUrl, source_guard::COMPOSE_SOURCE_URL, source_tables::SOURCE_TABLES,
    },
    tasks::import_mysql::source::{connect_options, pool, select_rows},
};
use sqlx::{MySqlPool, Row, mysql::MySqlPoolOptions};
use tracing::{
    Event, Metadata, Subscriber,
    field::{Field, Visit},
    span::{Attributes, Id, Record},
    subscriber::Interest,
};

use crate::support::ConnectionProbe;

/// The legacy tables that the import never reads, with a name that no
/// legacy table has.
const REFUSED: [&str; 4] = [
    "newsletter",
    "newsletter_subscribers",
    "password_resets",
    "migrations",
];

/// A pool of one lazy connection to `url`, built the way the task builds
/// it, in the test environment.
pub(crate) fn task_pool(url: &str) -> MySqlPool {
    let options = connect_options(&SourceDatabaseUrl::new(url.to_owned()), "test")
        .unwrap_or_else(|error| panic!("{error}"));
    pool(options)
}

/// A pool of one lazy connection to the probe, which never answers: a read
/// that is sent waits for the server's greeting until the short acquire
/// timeout ends it.
fn probe_pool(probe: &ConnectionProbe) -> MySqlPool {
    let options = connect_options(
        &SourceDatabaseUrl::new(format!(
            "mysql://loco:loco@127.0.0.1:{}/verseghy",
            probe.port()
        )),
        "test",
    )
    .expect("a local source");
    MySqlPoolOptions::new()
        .max_connections(1)
        .acquire_timeout(Duration::from_millis(500))
        .connect_lazy_with(options.into_inner())
}

#[tokio::test]
async fn a_read_of_a_table_off_the_allow_list_is_refused_before_anything_is_sent() {
    let probe = ConnectionProbe::bind("127.0.0.1");
    let source = probe_pool(&probe);
    for table in REFUSED {
        let message = select_rows(&source, table, &["id", "email"], &["id"])
            .await
            .expect_err(table)
            .to_string();
        assert!(
            message.starts_with("refusing to read ") && message.contains(table),
            "{table}: {message}"
        );
    }
    assert!(
        !probe.was_connected(),
        "a refused read connected to the source"
    );

    // Positive control: an allow-listed read is sent, so the pool does
    // connect to the probe; it then times out waiting for the greeting.
    let message = select_rows(&source, "posts_data", &["id"], &["id"])
        .await
        .expect_err("the probe never answers")
        .to_string();
    assert_eq!(
        message,
        "reading the legacy table `posts_data` from the source failed: no connection to the \
         source could be opened in time: check that the source is running and reachable"
    );
    assert!(probe.was_connected(), "the allowed read never connected");
}

#[tokio::test]
async fn the_newsletter_refusal_names_the_table_and_its_reason() {
    let probe = ConnectionProbe::bind("127.0.0.1");
    let message = select_rows(&probe_pool(&probe), "newsletter", &["email"], &["id"])
        .await
        .expect_err("left behind")
        .to_string();
    assert!(
        message.starts_with(
            "refusing to read the legacy table `newsletter` from the source: the import never \
             reads it (data-migration.md §9.1: "
        ),
        "{message}"
    );
    assert!(message.contains("NWS-01"), "{message}");
    assert!(!probe.was_connected());
}

/// The compose source, checked to answer, or a panic that says how to
/// start it.
pub(crate) async fn compose_source() -> MySqlPool {
    // A first connection with a short timeout, so a source that is not
    // running fails the test in seconds, not after the pool's 30 s.
    let options = connect_options(
        &SourceDatabaseUrl::new(COMPOSE_SOURCE_URL.to_owned()),
        "test",
    )
    .expect("the compose source is local");
    let check = MySqlPoolOptions::new()
        .max_connections(1)
        .acquire_timeout(Duration::from_secs(5))
        .connect_lazy_with(options.into_inner());
    if let Err(error) = sqlx::query("SELECT 1").execute(&check).await {
        panic!(
            "the MySQL source of compose.yaml does not answer at 127.0.0.1:3306 ({error}); start \
             it with `podman-compose --profile import up -d mysql` (or `docker compose --profile \
             import up -d mysql`)"
        );
    }
    check.close().await;
    task_pool(COMPOSE_SOURCE_URL)
}

/// The names of the columns of `table` in the source, in their order.
async fn columns_of(source: &MySqlPool, table: &str) -> Vec<String> {
    sqlx::query(
        "SELECT CAST(COLUMN_NAME AS CHAR) FROM information_schema.COLUMNS WHERE TABLE_SCHEMA = \
         DATABASE() AND TABLE_NAME = ? ORDER BY ORDINAL_POSITION",
    )
    .bind(table)
    .fetch_all(source)
    .await
    .expect("read the columns")
    .iter()
    .map(|row| row.get::<String, _>(0))
    .collect()
}

/// Compose source.
#[tokio::test]
async fn the_compose_source_runs_production_s_server_with_the_legacy_schema() {
    let source = compose_source().await;
    let version: String = sqlx::query_scalar("SELECT VERSION()")
        .fetch_one(&source)
        .await
        .expect("the server version");
    // website_k8s runs MySQL 8.4 (compose.yaml pins the same tag).
    assert!(version.starts_with("8.4."), "{version}");

    let mut tables: Vec<String> = sqlx::query_scalar(
        "SELECT CAST(TABLE_NAME AS CHAR) FROM information_schema.TABLES WHERE TABLE_SCHEMA = \
         DATABASE() ORDER BY TABLE_NAME",
    )
    .fetch_all(&source)
    .await
    .expect("the tables");
    let mut expected: Vec<String> = SOURCE_TABLES
        .iter()
        .map(|table| table.name().to_owned())
        .chain(["newsletter", "password_resets", "migrations"].map(str::to_owned))
        .collect();
    tables.sort();
    expected.sort();
    assert_eq!(tables, expected);
}

/// Compose source: the account has exactly the cut-over account's grants
/// (data-migration.md §3.3), so the tests need no more than production's
/// import has.
#[tokio::test]
async fn the_compose_account_has_exactly_the_grants_of_the_cut_over_account() {
    let source = compose_source().await;
    let grants: Vec<String> = sqlx::query("SHOW GRANTS")
        .fetch_all(&source)
        .await
        .expect("the grants")
        .iter()
        .map(|row| row.get::<String, _>(0))
        .collect();
    assert_eq!(
        grants,
        [
            "GRANT USAGE ON *.* TO `loco`@`%`",
            "GRANT SELECT, LOCK TABLES ON `verseghy`.* TO `loco`@`%`",
        ]
    );
}

/// Compose source: every allow-listed table reads through the layer with
/// its real columns, keyword-named ones (`date`, `type`) and the
/// camel-case `previewToken` included.
#[tokio::test]
async fn every_allow_listed_table_reads_through_the_source_layer() {
    let source = compose_source().await;
    for table in SOURCE_TABLES {
        let columns = columns_of(&source, table.name()).await;
        assert!(!columns.is_empty(), "{}: no columns", table.name());
        let columns: Vec<&str> = columns.iter().map(String::as_str).collect();
        let rows = select_rows(&source, table.name(), &columns, &columns[..1])
            .await
            .unwrap_or_else(|error| panic!("{}: {error}", table.name()));
        assert!(
            rows.is_empty(),
            "{}: the fixture holds no rows",
            table.name()
        );
    }
}

/// Compose source: the account may read the newsletter, as production's
/// may; only the layer keeps it unread.
#[tokio::test]
async fn the_newsletter_is_refused_although_the_account_may_read_it() {
    let source = compose_source().await;
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM `newsletter`")
        .fetch_one(&source)
        .await
        .expect("the account may read the table");
    assert_eq!(count, 0);
    let columns = columns_of(&source, "newsletter").await;
    let columns: Vec<&str> = columns.iter().map(String::as_str).collect();
    let message = select_rows(&source, "newsletter", &columns, &["id"])
        .await
        .expect_err("left behind")
        .to_string();
    assert!(
        message.starts_with("refusing to read the legacy table `newsletter`"),
        "{message}"
    );
}

/// The user name and password of a sign-in that the compose source
/// refuses; neither may appear anywhere.
const UNKNOWN_USER: &str = "import-user-4e7b";
const UNKNOWN_PASSWORD: &str = "pw-refused-91c2d";

/// Compose source: a failed read names the server's error number, not its
/// text, which would name the account.
#[tokio::test]
async fn a_failed_read_shows_the_error_number_and_no_part_of_the_url() {
    let _ = compose_source().await;
    let refused = task_pool(&format!(
        "mysql://{UNKNOWN_USER}:{UNKNOWN_PASSWORD}@127.0.0.1:3306/verseghy"
    ));
    let message = select_rows(&refused, "users", &["id"], &["id"])
        .await
        .expect_err("access denied")
        .to_string();
    assert_eq!(
        message,
        "reading the legacy table `users` from the source failed: the MySQL server answered with \
         error 1045 (SQLSTATE 28000); look the number up in the MySQL error reference. The \
         server's text is not shown, because it may name the source account"
    );

    let source = compose_source().await;
    let message = select_rows(&source, "users", &["id", "no_such_column"], &["id"])
        .await
        .expect_err("unknown column")
        .to_string();
    assert!(message.contains("error 1054 (SQLSTATE 42S22)"), "{message}");
    assert!(!message.contains("loco"), "{message}");
}

/// Compose source: a failed TLS handshake shows the layer's fixed
/// description, not rustls's text, which names the URL's host when a
/// certificate does not match it. `VERIFY_IDENTITY` without `ssl-ca`
/// refuses the certificate that the image generates, because no public root
/// signed it, and sqlx-core 0.9 passes the rustls error on as an I/O error,
/// so this checks that path with the real driver.
#[tokio::test]
async fn a_failed_tls_handshake_shows_the_fixed_description() {
    let _ = compose_source().await;
    for host in ["127.0.0.1", "localhost"] {
        let untrusted = task_pool(&format!(
            "mysql://loco:loco@{host}:3306/verseghy?ssl-mode=VERIFY_IDENTITY"
        ));
        let message = select_rows(&untrusted, "users", &["id"], &["id"])
            .await
            .expect_err("the certificate is not trusted")
            .to_string();
        assert_eq!(
            message,
            "reading the legacy table `users` from the source failed: the TLS connection to the \
             source failed: check the URL's ssl-mode and ssl-ca and the server's certificate (the \
             driver's text is not shown, because it may name the host)",
            "{host}"
        );
        untrusted.close().await;
    }
}

/// Every log event and span field the code under test records, as text.
#[derive(Clone, Default)]
pub(crate) struct CapturedLog {
    lines: Arc<Mutex<Vec<String>>>,
    next_span: Arc<AtomicU64>,
}

impl CapturedLog {
    pub(crate) fn text(&self) -> String {
        self.lines.lock().expect("the log").join("\n")
    }

    fn push(&self, metadata: &Metadata<'_>, fields: &str) {
        self.lines.lock().expect("the log").push(format!(
            "{} {} {}{fields}",
            metadata.level(),
            metadata.target(),
            metadata.name()
        ));
    }
}

/// Writes every field of an event or span as ` name=value`.
struct Fields(String);

impl Visit for Fields {
    fn record_debug(&mut self, field: &Field, value: &dyn fmt::Debug) {
        let _ = write!(self.0, " {}={value:?}", field.name());
    }

    fn record_str(&mut self, field: &Field, value: &str) {
        let _ = write!(self.0, " {}={value}", field.name());
    }
}

/// A subscriber that records everything at every level. A hand-written one,
/// so the test needs no crate of its own (tracing's `Subscriber` is the
/// interface; `tracing-subscriber` is not a dependency of backend3).
impl Subscriber for CapturedLog {
    fn register_callsite(&self, _: &'static Metadata<'static>) -> Interest {
        Interest::always()
    }

    fn enabled(&self, _: &Metadata<'_>) -> bool {
        true
    }

    fn new_span(&self, span: &Attributes<'_>) -> Id {
        let mut fields = Fields(String::new());
        span.record(&mut fields);
        self.push(span.metadata(), &fields.0);
        Id::from_u64(self.next_span.fetch_add(1, Ordering::Relaxed) + 1)
    }

    fn record(&self, _: &Id, values: &Record<'_>) {
        let mut fields = Fields(String::new());
        values.record(&mut fields);
        self.lines.lock().expect("the log").push(fields.0);
    }

    fn record_follows_from(&self, _: &Id, _: &Id) {}

    fn event(&self, event: &Event<'_>) {
        let mut fields = Fields(String::new());
        event.record(&mut fields);
        self.push(event.metadata(), &fields.0);
    }

    fn enter(&self, _: &Id) {}

    fn exit(&self, _: &Id) {}
}

/// Compose source: what the driver logs, at every level, while the layer
/// reads, refuses a table and fails to sign in, holds no part of the URLs.
/// The test runs on one thread (`#[tokio::test]`'s runtime), so the
/// subscriber it sets for that thread sees every event of the reads.
#[tokio::test]
async fn the_source_layer_logs_no_part_of_the_url() {
    let log = CapturedLog::default();
    let guard = tracing::subscriber::set_default(log.clone());

    let source = compose_source().await;
    select_rows(&source, "posts_labels", &["id", "name", "color"], &["id"])
        .await
        .expect("an allowed read");
    select_rows(&source, "newsletter", &["email"], &["id"])
        .await
        .expect_err("a refused read");
    let refused = task_pool(&format!(
        "mysql://{UNKNOWN_USER}:{UNKNOWN_PASSWORD}@127.0.0.1:3306/verseghy"
    ));
    select_rows(&refused, "users", &["id"], &["id"])
        .await
        .expect_err("a refused sign-in");
    source.close().await;
    refused.close().await;
    drop(guard);

    let text = log.text();
    // The driver did log the reads, so the check below has something to
    // look at.
    assert!(text.contains("FROM `posts_labels`"), "{text}");
    // `loco` is both the user name and the password of the compose source;
    // no backend3 or Loco code runs here, so the word appears nowhere else.
    for part in [
        "loco",
        UNKNOWN_USER,
        UNKNOWN_PASSWORD,
        "mysql://",
        "127.0.0.1:3306",
    ] {
        assert!(!text.contains(part), "{part} in the log:\n{text}");
    }
}
