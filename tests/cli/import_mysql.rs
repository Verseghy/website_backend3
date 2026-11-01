//! `import_mysql` in production, end to end through the built `backend3-cli`
//! (data-migration.md §4.1): config/production.yaml reads the source from
//! `IMPORT_SOURCE_DATABASE_URL` with the empty string as its default, the
//! task refuses an unset source in both spellings (the empty string and the
//! `backend-secrets` placeholder `disabled`), and neither the process's
//! output nor its JSON log ever shows the source URL, even at log level
//! `trace`.
//!
//! The child process connects to the test's own migrated database (Loco
//! builds a task's context with a database pool), and the source URL points
//! at a port where nothing answers: the task opens the source connection on
//! first use, and no import step reads the source yet.

use std::{fs, time::Duration};

use loco_rs::app::AppContext;
use sea_orm::{ConnectionTrait, DbBackend, Statement};

use crate::support::{CliRun, ScratchDir, cli_jwt_secret, cli_totp_key, run_cli, with_app};

const TIMEOUT: Duration = Duration::from_secs(120);

/// The `application_name` of the child processes' connections, so the test
/// can wait for them to end.
const CHILD_APPLICATION: &str = "backend3-import-cli-test";

/// The test database's URL for a child process, with its connections
/// tagged [`CHILD_APPLICATION`].
fn child_database_url(ctx: &AppContext) -> String {
    let mut url = url::Url::parse(&ctx.config.database.uri).expect("the test database URL");
    url.query_pairs_mut()
        .append_pair("application_name", CHILD_APPLICATION);
    url.into()
}

/// Waits until the server has ended every session of the child processes.
///
/// A child closes its sockets when it exits, and PostgreSQL ends their
/// sessions a moment later, on its own time. `with_app` drops the test's
/// database when the test ends, and PostgreSQL refuses that while another
/// session still uses it, so the test waits here first.
async fn wait_for_children_to_disconnect(ctx: &AppContext) {
    let sql = format!(
        "SELECT count(*) FROM pg_stat_activity WHERE datname = current_database() 
         AND application_name = '{CHILD_APPLICATION}'"
    );
    for _ in 0..500 {
        let row = ctx
            .db
            .query_one_raw(Statement::from_string(DbBackend::Postgres, sql.clone()))
            .await
            .expect("count the child sessions")
            .expect("one row");
        if row.try_get_by_index::<i64>(0).expect("a count") == 0 {
            return;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    panic!("the child processes' database sessions did not end within 10 s");
}

/// The password of the source URL: no output may contain it.
const PASSWORD: &str = "pw-cli-source-never-shown-c41d";

/// The scratch directories and the resolution file of one run.
struct Run {
    root: ScratchDir,
}

impl Run {
    fn new() -> Self {
        let root = ScratchDir::new("import-cli");
        fs::create_dir(root.path().join("storage")).expect("storage_root");
        fs::create_dir(root.path().join("report")).expect("report");
        fs::write(root.path().join("resolutions.toml"), "version = 1\n").expect("resolutions");
        Self { root }
    }

    /// `backend3-cli task import_mysql` with every argument of a valid run.
    fn args(&self) -> Vec<String> {
        let path = |name: &str| self.root.path().join(name).to_string_lossy().into_owned();
        vec![
            "task".to_owned(),
            "import_mysql".to_owned(),
            "mode:dry-run".to_owned(),
            "snapshot_at:2026-10-23T17:01:02Z".to_owned(),
            format!("resolutions:{}", path("resolutions.toml")),
            format!("storage_root:{}", path("storage")),
            format!("report:{}", path("report")),
        ]
    }

    /// Runs the task in production against `database_url`, with
    /// `IMPORT_SOURCE_DATABASE_URL` set to `source` (unset for `None`).
    fn cli(&self, database_url: &str, source: Option<&str>) -> CliRun {
        let jwt_secret = cli_jwt_secret();
        let totp_key = cli_totp_key();
        let mut env = vec![
            ("LOCO_ENV", "production"),
            ("DATABASE_URL", database_url),
            ("HOST", "http://localhost"),
            ("JWT_SECRET", jwt_secret.as_str()),
            ("TOTP_ENCRYPTION_KEY", totp_key.as_str()),
            // The child really connects to the test database, unlike the
            // other production CLI tests (300 ms, never meant to connect).
            // production.yaml's 5000 ms default becomes the database pool's
            // acquire timeout, which a loaded test machine can exceed ("pool
            // timed out while waiting for an open connection"), so the run
            // waits longer instead of failing; TIMEOUT still bounds the run.
            ("DB_CONNECT_TIMEOUT", "30000"),
            // Every log line the process can write, to show that none of
            // them carries the source URL.
            ("LOG_LEVEL", "trace"),
        ];
        if let Some(source) = source {
            env.push(("IMPORT_SOURCE_DATABASE_URL", source));
        }
        let args = self.args();
        let args: Vec<&str> = args.iter().map(String::as_str).collect();
        run_cli(&env, &args, TIMEOUT)
    }

    /// Everything in the report directory, as text.
    fn report_text(&self) -> String {
        let mut text = String::new();
        for entry in fs::read_dir(self.root.path().join("report")).expect("the report directory") {
            let path = entry.expect("an entry").path();
            text.push_str(&fs::read_to_string(&path).expect("a report file"));
        }
        text
    }
}

#[tokio::test]
async fn production_refuses_an_unset_source_in_both_spellings_and_when_the_variable_is_missing() {
    with_app(|_router, ctx| async move {
        let database_url = child_database_url(&ctx);
        for source in [None, Some(""), Some("disabled")] {
            let run = Run::new();
            let cli = run.cli(&database_url, source);
            wait_for_children_to_disconnect(&ctx).await;
            assert!(!cli.status.success(), "{source:?}: {}", cli.stderr);
            assert!(
                cli.stderr.contains(
                    "Error: refusing to start: settings.import.source_database_url is unset"
                ),
                "{source:?}: {}",
                cli.stderr
            );
            assert!(
                cli.stderr.contains("IMPORT_SOURCE_DATABASE_URL"),
                "{}",
                cli.stderr
            );
            assert!(
                run.report_text().is_empty(),
                "{source:?}: a refused run wrote a report"
            );
        }
    })
    .await;
}

#[tokio::test]
async fn production_never_shows_the_source_url_in_its_output_or_its_logs() {
    with_app(|_router, ctx| async move {
        let database_url = child_database_url(&ctx);
        let source = format!("mysql://backend3_import:{PASSWORD}@127.0.0.1:9/verseghy");

        // A run that completes and writes its report.
        let run = Run::new();
        let cli = run.cli(&database_url, Some(&source));
        wait_for_children_to_disconnect(&ctx).await;
        assert!(cli.status.success(), "{}\n{}", cli.stdout, cli.stderr);
        assert!(
            cli.stdout.contains("the import run has written its report"),
            "the JSON log is on stdout: {}",
            cli.stdout
        );
        let report = run.report_text();
        assert!(report.contains("\"outcome\": \"completed\""), "{report}");

        // A run that the task refuses: the report directory is not empty any
        // more.
        let refused = run.cli(&database_url, Some(&source));
        wait_for_children_to_disconnect(&ctx).await;
        assert!(!refused.status.success(), "{}", refused.stderr);
        assert!(
            refused.stderr.contains("which is not empty"),
            "{}",
            refused.stderr
        );

        // A source the driver refuses, with the password still in it.
        let bad = format!("{source}?ssl-mode=sometimes");
        let rejected = Run::new().cli(&database_url, Some(&bad));
        wait_for_children_to_disconnect(&ctx).await;
        assert!(!rejected.status.success(), "{}", rejected.stderr);
        assert!(
            rejected.stderr.contains("the MySQL driver refuses"),
            "{}",
            rejected.stderr
        );

        for (name, text) in [
            ("stdout", &cli.stdout),
            ("stderr", &cli.stderr),
            ("report", &report),
            ("refused stdout", &refused.stdout),
            ("refused stderr", &refused.stderr),
            ("rejected stdout", &rejected.stdout),
            ("rejected stderr", &rejected.stderr),
        ] {
            assert!(!text.contains(PASSWORD), "{name}: {text}");
            assert!(!text.contains("backend3_import"), "{name}: {text}");
        }
    })
    .await;
}
