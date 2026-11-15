//! Tests of the `import_mysql` task (ADR 0017, data-migration.md §4.1,
//! §11.3), run as Loco runs it: `run_task::<App>` with the task's
//! arguments, on the test's own database.
//!
//! A run that completes reads the MySQL source of compose.yaml (profile
//! `import`; the CI Test job starts it), whose legacy schema matches the
//! expected DDL and holds no rows, so the run imports nothing and its source
//! session finds no timestamp. The tests of refusals before the task lock
//! use a URL on the discard port, where nothing listens: they never connect.
//! Each test sets the source in its own copy of the app context, because
//! config/test.yaml configures none.
//!
//! The checks of the target and of the source session have their own tests
//! (`import_mysql_target.rs`, `import_mysql_session.rs`); the tests of the
//! report writer's own refusals call `tasks::import_mysql::report`
//! directly, with the result a failing step would return.

use std::{
    collections::BTreeMap,
    fs,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

use backend3::{
    app::{App, set_clock},
    domain::{
        activity::{Action, Actor},
        clock::Clock,
        import_mysql::{
            not_migrated::NOT_MIGRATED_TABLES,
            options::{CanteenAuditLocalBefore, DEFAULT_RESOLUTIONS, ImportOptions, Mode},
            report::{NO_STEP_YET, Outcome, ReportParts, RunSummary, escape_cell},
            resolutions::{FileDigest, MAX_FILE_BYTES, ResolutionFile},
            signoff,
            source_guard::COMPOSE_SOURCE_URL,
            source_session::{AutoIncrement, SourceFacts},
            source_tables::SOURCE_TABLES,
            target::IMPORT_TARGET_TABLES,
        },
    },
    models::activity_log::{self, EntityRef, Entry},
    tasks::import_mysql::{
        report::{self, REPORT_FILE_MODE},
        resolutions::load,
    },
};
use chrono::{DateTime, TimeDelta, Utc};
use loco_rs::{app::AppContext, boot::run_task, task::Vars};
use sea_orm::{ConnectionTrait, DatabaseConnection, DbBackend, Statement};
use serde_json::{Value, json};

use super::import_mysql_source::compose_source;
use crate::support::{ScratchDir, hold_task_lock, with_app, with_app_at};

/// The task's registered name, which is also its lock name.
const TASK: &str = "import_mysql";

/// The password of the source URL: no message, log or report may show it.
const PASSWORD: &str = "pw-source-never-shown-5b1e";

/// A source URL in the shape of production's, on the discard port of the
/// local host, where nothing answers. For the refusals before the task
/// lock, which never connect.
fn source_url() -> String {
    format!("mysql://backend3_import:{PASSWORD}@127.0.0.1:9/verseghy")
}

/// The compose source with an account it does not have: its server refuses
/// the sign-in at once (MySQL error 1045), so a run that reaches the source
/// fails quickly, without retrying a refused connection for the pool's
/// acquire timeout.
fn refused_source_url() -> String {
    format!("mysql://backend3_import:{PASSWORD}@127.0.0.1:3306/verseghy")
}

/// The instant the tests' runs start at.
fn start() -> DateTime<Utc> {
    "2026-10-31T18:00:00Z".parse().expect("RFC 3339")
}

/// The directories and the resolution file of one run, in a scratch
/// directory of their own.
struct Paths {
    /// Holds everything below; removed when the test ends.
    root: ScratchDir,
}

impl Paths {
    fn new() -> Self {
        let root = ScratchDir::new("import-task");
        fs::create_dir(root.path().join("storage")).expect("storage_root");
        fs::create_dir(root.path().join("report")).expect("report");
        fs::write(root.path().join("resolutions.toml"), "version = 1\n").expect("resolutions");
        Self { root }
    }

    fn storage_root(&self) -> PathBuf {
        self.root.path().join("storage")
    }

    fn report(&self) -> PathBuf {
        self.root.path().join("report")
    }

    fn resolutions(&self) -> PathBuf {
        self.root.path().join("resolutions.toml")
    }

    /// Every argument of a valid run.
    fn arguments(&self) -> BTreeMap<String, String> {
        let text = |path: PathBuf| path.to_string_lossy().into_owned();
        BTreeMap::from([
            ("snapshot_at".to_owned(), "2026-10-23T17:01:02Z".to_owned()),
            ("resolutions".to_owned(), text(self.resolutions())),
            ("storage_root".to_owned(), text(self.storage_root())),
            ("report".to_owned(), text(self.report())),
        ])
    }

    /// The names of the files in the report directory.
    fn report_files(&self) -> Vec<String> {
        let mut names: Vec<String> = fs::read_dir(self.report())
            .expect("list the report directory")
            .map(|entry| {
                entry
                    .expect("an entry")
                    .file_name()
                    .to_string_lossy()
                    .into_owned()
            })
            .collect();
        names.sort();
        names
    }
}

/// A copy of `ctx` whose `settings.import.source_database_url` is `source`.
/// The copy shares the database pool and the shared store (the clock) with
/// `ctx`.
fn with_source(ctx: &AppContext, source: &str) -> AppContext {
    let mut ctx = ctx.clone();
    let settings = ctx.config.settings.as_mut().expect("a settings section");
    settings["import"]["source_database_url"] = json!(source);
    ctx
}

/// Runs the task as Loco's CLI does.
async fn run(ctx: &AppContext, arguments: BTreeMap<String, String>) -> loco_rs::Result<()> {
    run_task::<App>(ctx, Some(&TASK.to_owned()), &Vars { cli: arguments }).await
}

/// The error message of a refused run.
async fn refusal(ctx: &AppContext, arguments: BTreeMap<String, String>) -> String {
    run(ctx, arguments)
        .await
        .expect_err("the task refuses the run")
        .to_string()
}

/// A clock that moves forward by `step` after every reading, so a test can
/// tell the task's readings apart.
#[derive(Debug)]
struct SteppingClock {
    next: Mutex<DateTime<Utc>>,
    step: TimeDelta,
}

impl Clock for SteppingClock {
    fn now(&self) -> DateTime<Utc> {
        let mut next = self.next.lock().expect("the clock's lock");
        let now = *next;
        *next = now + self.step;
        now
    }
}

#[tokio::test]
async fn a_run_writes_its_report_with_the_start_and_end_from_the_app_clock() {
    compose_source().await;
    with_app(|_router, ctx| async move {
        for mode in ["dry-run", "apply"] {
            let paths = Paths::new();
            // The task reads the clock twice: at the start of the run and at
            // its end, just before it writes the report.
            let step = TimeDelta::milliseconds(2_750);
            set_clock(
                &ctx,
                Arc::new(SteppingClock {
                    next: Mutex::new(start()),
                    step,
                }),
            );
            let mut arguments = paths.arguments();
            arguments.insert("mode".to_owned(), mode.to_owned());

            run(&with_source(&ctx, COMPOSE_SOURCE_URL), arguments)
                .await
                .unwrap_or_else(|error| panic!("{mode}: {error}"));

            assert_eq!(
                paths.report_files(),
                ["report.md", "signoff.md", "summary.json"],
                "{mode}"
            );
            let text = fs::read_to_string(paths.report().join("summary.json")).expect("summary");
            let summary: Value = serde_json::from_str(&text).expect("JSON");
            assert_eq!(summary["outcome"], "completed", "{mode}");
            assert_eq!(summary["started_at"], "2026-10-31T18:00:00Z", "{mode}");
            assert_eq!(summary["finished_at"], "2026-10-31T18:00:02.750Z", "{mode}");
            assert_eq!(summary["duration_ms"], 2_750, "{mode}");
            assert_eq!(summary["arguments"]["mode"], mode);
            assert_eq!(summary["arguments"]["snapshot_at"], "2026-10-23T17:01:02Z");
            assert_eq!(
                summary["arguments"]["canteen_audit_local_before"],
                "2022-01-21T00:00:00Z"
            );
            // The source session's part: the empty compose source matched
            // the expected DDL and holds no timestamp.
            assert_eq!(summary["source"]["fingerprint"], "matched", "{mode}");
            assert_eq!(summary["source"]["latest_timestamp"], Value::Null);
            assert_eq!(
                summary["source"]["auto_increment"]
                    .as_object()
                    .expect("an object")
                    .len(),
                SOURCE_TABLES.len()
            );
            let markdown = fs::read_to_string(paths.report().join("report.md")).expect("report.md");
            assert!(markdown.contains("| Duration | 2.750 s |"), "{markdown}");
            assert!(
                markdown.contains("\n## Source\n\nThe schema fingerprint of the 17 legacy tables"),
                "{markdown}"
            );
            assert!(markdown.contains(NO_STEP_YET), "{markdown}");
            for row in NOT_MIGRATED_TABLES {
                let cell = escape_cell(row.table);
                assert!(markdown.contains(&cell), "{cell} in {markdown}");
            }
            // The run reached its end with no detector in this build, so
            // its Defects part is complete and empty, and so is the sheet.
            assert_eq!(
                summary["defects"],
                json!({
                    "complete": true, "checked": [], "uncovered": 0, "refused": 0, "records": [],
                }),
                "{mode}"
            );
            assert!(
                markdown.contains("No defect detector ran: this build has none yet."),
                "{markdown}"
            );
            let sheet = fs::read_to_string(paths.report().join("signoff.md")).expect("signoff.md");
            assert!(sheet.starts_with(signoff::TITLE), "{sheet}");
            assert!(
                sheet.contains("in the source snapshot of 2026-10-23T17:01:02Z,"),
                "{sheet}"
            );
            assert!(
                sheet.ends_with(&format!("\n\n{}\n", signoff::NO_DETECTOR)),
                "{sheet}"
            );
        }
    })
    .await;
}

#[tokio::test]
async fn a_run_at_a_standing_clock_takes_no_time() {
    compose_source().await;
    with_app_at(start(), |_router, ctx, _clock| async move {
        let paths = Paths::new();
        run(&with_source(&ctx, COMPOSE_SOURCE_URL), paths.arguments())
            .await
            .expect("the run");
        let text = fs::read_to_string(paths.report().join("summary.json")).expect("summary");
        let summary: Value = serde_json::from_str(&text).expect("JSON");
        assert_eq!(summary["started_at"], summary["finished_at"]);
        assert_eq!(summary["duration_ms"], 0);
    })
    .await;
}

#[tokio::test]
async fn the_arguments_are_checked_with_a_message_that_names_each_one() {
    with_app_at(start(), |_router, ctx, _clock| async move {
        let ctx = with_source(&ctx, &source_url());
        let paths = Paths::new();
        let set = |name: &str, value: &str| {
            let mut arguments = paths.arguments();
            arguments.insert(name.to_owned(), value.to_owned());
            arguments
        };
        let without = |name: &str| {
            let mut arguments = paths.arguments();
            arguments.remove(name);
            arguments
        };
        let cases = [
            (set("mode", "commit"), "the argument mode is \"commit\""),
            (
                without("snapshot_at"),
                "the argument snapshot_at is missing",
            ),
            (
                set("snapshot_at", "2026-10-23T19:01:02+02:00"),
                "here snapshot_at:2026-10-23T17:01:02Z",
            ),
            (
                set("snapshot_at", "2026-11-01T00:00:00Z"),
                "later than the start of this run at 2026-10-31T18:00:00Z",
            ),
            (
                set("canteen_audit_local_before", "never"),
                "the argument canteen_audit_local_before",
            ),
            (
                without("storage_root"),
                "the argument storage_root is missing",
            ),
            (without("report"), "the argument report is missing"),
            (
                set("snapshot", "2026-10-23T17:01:02Z"),
                "unknown argument \"snapshot\"",
            ),
        ];
        for (arguments, expected) in cases {
            let message = refusal(&ctx, arguments).await;
            assert!(message.contains(expected), "{expected}: {message}");
        }
        assert!(
            paths.report_files().is_empty(),
            "a refused run wrote a report"
        );
    })
    .await;
}

#[tokio::test]
async fn the_paths_are_checked_before_the_run_with_a_message_that_names_each_one() {
    with_app_at(start(), |_router, ctx, _clock| async move {
        let ctx = with_source(&ctx, &source_url());
        let paths = Paths::new();
        let missing = paths.root.path().join("missing");
        let file = paths.resolutions();
        let set = |name: &str, value: &Path| {
            let mut arguments = paths.arguments();
            arguments.insert(name.to_owned(), value.to_string_lossy().into_owned());
            arguments
        };
        let inside_storage = paths.storage_root().join("reports");
        fs::create_dir(&inside_storage).expect("a directory inside storage_root");
        // A symbolic link outside storage_root to a directory inside it.
        let link = paths.root.path().join("link-to-storage");
        std::os::unix::fs::symlink(&inside_storage, &link).expect("a symbolic link");
        let mut defaults = paths.arguments();
        defaults.remove("resolutions");

        let cases = [
            (
                set("resolutions", &missing),
                "the argument resolutions is",
                "which cannot be read",
            ),
            (
                set("resolutions", &paths.storage_root()),
                "the argument resolutions is",
                "which is not a file",
            ),
            (
                defaults,
                "the argument resolutions is /app/data/import/resolutions.toml",
                "which cannot be read",
            ),
            (
                set("storage_root", &missing),
                "the argument storage_root is",
                "which cannot be read",
            ),
            (
                set("storage_root", &file),
                "the argument storage_root is",
                "which is not a directory",
            ),
            (
                set("report", &missing),
                "the argument report is",
                "which cannot be read",
            ),
            (
                set("report", &file),
                "the argument report is",
                "which is not a directory",
            ),
            (
                set("report", paths.root.path()),
                "the argument report is",
                "which is not empty",
            ),
            (
                set("report", &inside_storage),
                "the argument report is",
                "inside storage_root",
            ),
            (
                set("report", &link),
                "the argument report is",
                "inside storage_root",
            ),
        ];
        for (arguments, start_of_message, problem) in cases {
            let message = refusal(&ctx, arguments).await;
            assert!(
                message.starts_with(start_of_message),
                "{start_of_message}: {message}"
            );
            assert!(message.contains(problem), "{problem}: {message}");
        }
        assert!(
            paths.report_files().is_empty(),
            "a refused run wrote a report"
        );
        assert_eq!(
            fs::read_dir(&inside_storage).expect("list").count(),
            0,
            "a refused run wrote into storage_root"
        );
    })
    .await;
}

/// data-migration.md §4.1, §8.2: the task reads the resolution file and
/// validates it before the task lock, so a refused file stops the run before
/// the source is read or a report is written, and the message names the
/// argument, the path and every problem by entry and field.
#[tokio::test]
async fn the_resolution_file_is_validated_before_the_run_naming_entry_and_field() {
    with_app_at(start(), |_router, ctx, _clock| async move {
        let ctx = with_source(&ctx, &source_url());
        let paths = Paths::new();
        let file = paths.resolutions();
        let invalid = "which is not a valid resolution file (data-migration.md §8.2; its schema \
                       is documented in src/domain/import_mysql/resolutions.rs): ";
        let cases: [(&[u8], String); 5] = [
            (
                b"version = 1\n[[canteen_duplicate]]\ndate = 2022-04-01\nexpect_ids = [393, \
                  398]\nkeep = 400\n[[page_hidden]]\npage_id = 0\nexpect_updated_at = \
                  2024-03-01T10:00:00Z\n",
                format!(
                    "{invalid}2 problems:\n- canteen_duplicate[1] (D6), field keep: row 400 is \
                     not in expect_ids [393, 398]: keep one of the listed rows\n- page_hidden[1] \
                     (D10), field page_id: 0 is not a row id: write a whole number from 1 to \
                     4294967295"
                ),
            ),
            (
                b"version = 2\n",
                format!(
                    "{invalid}1 problem:\n- version: 2 is not a schema version this build \
                     reads: it reads version 1"
                ),
            ),
            (
                b"",
                format!(
                    "{invalid}1 problem:\n- version: missing: the file starts with version = 1"
                ),
            ),
            (
                b"version = 1\nsigned_off = 2026-11-20\npost_author = \"55\"\n",
                format!(
                    "{invalid}1 problem:\n- post_author (D2): expected an array of tables, \
                     written [[post_author]], found the string \"55\""
                ),
            ),
            (
                b"version = 1\n\xff\xfe\n",
                "which is not UTF-8 text (the first invalid byte is at offset 12): give the \
                 editor-signed resolution file (data-migration.md §8.2), a UTF-8 TOML file"
                    .to_owned(),
            ),
        ];
        for (contents, problem) in cases {
            fs::write(&file, contents).expect("write the resolution file");
            let message = refusal(&ctx, paths.arguments()).await;
            assert_eq!(
                message,
                format!("the argument resolutions is {}, {problem}", file.display())
            );
        }
        assert!(
            paths.report_files().is_empty(),
            "a refused run wrote a report"
        );
    })
    .await;
}

/// The task reads at most `MAX_FILE_BYTES` of the resolution file: a file of
/// exactly that size is read, one byte more is refused before it is parsed.
#[test]
fn the_resolution_file_is_read_up_to_its_size_limit() {
    let root = ScratchDir::new("import-resolutions-limit");
    let path = root.path().join("resolutions.toml");
    let limit = usize::try_from(MAX_FILE_BYTES).expect("the limit fits in memory");
    // A valid list padded with a comment to exactly the limit.
    let header = "version = 1\n#";
    let at_limit = format!("{header}{}", "x".repeat(limit - header.len()));
    assert_eq!(at_limit.len(), limit);
    fs::write(&path, &at_limit).expect("write");
    let loaded = load(&path).expect("a file at the limit");
    assert_eq!(loaded.file, ResolutionFile::default());
    assert_eq!(loaded.digest, FileDigest::of(at_limit.as_bytes()));

    fs::write(&path, format!("{at_limit}x")).expect("write");
    assert_eq!(
        load(&path)
            .expect_err("one byte over the limit")
            .to_string(),
        format!(
            "the argument resolutions is {}, which is larger than 1048576 bytes: give the \
             editor-signed resolution file (data-migration.md §8.2), which is far smaller",
            path.display()
        )
    );

    let missing = root.path().join("missing.toml");
    let message = load(&missing).expect_err("a missing file").to_string();
    assert!(
        message.starts_with(&format!(
            "the argument resolutions is {}, which cannot be opened (",
            missing.display()
        )),
        "{message}"
    );
}

/// The committed list is what the image ships and the nightly reads: the
/// task's own loader accepts it, and its digest is that of the committed
/// bytes.
#[test]
fn the_task_loads_the_committed_resolution_file() {
    let committed = crate::support::repository_root().join("data/import/resolutions.toml");
    let loaded = load(&committed).unwrap_or_else(|error| panic!("{error}"));
    let bytes = fs::read(&committed).expect("the committed file");
    assert_eq!(loaded.digest, FileDigest::of(&bytes));
}

/// The digest is that of the bytes the task read and validated, comments
/// and layout included, as `sha256sum` prints it for the file
/// (data-migration.md §5.4).
#[test]
fn the_loaded_digest_is_the_sha256_of_the_file_as_written() {
    let root = ScratchDir::new("import-resolutions-digest");
    let path = root.path().join("resolutions.toml");
    fs::write(&path, "version = 1\n").expect("write");
    let plain = load(&path).expect("the empty list");
    // sha256sum's output for the 12 bytes "version = 1\n".
    assert_eq!(
        plain.digest.to_string(),
        "dbab12665d98aef021ba64953c61b0ed8a908cfb56a1c01e2fcb4b052b71a2a1"
    );

    fs::write(&path, "# The empty list.\nversion = 1\n").expect("write");
    let commented = load(&path).expect("the empty list with a comment");
    assert_eq!(commented.file, plain.file);
    assert_ne!(commented.digest, plain.digest);
}

/// ADR 0016 §4, data-migration.md §4.1: the resolution file is validated
/// before the task lock is taken, so a run that finds the lock held still
/// refuses an invalid list, with the full message and no report, instead of
/// exiting 0 with nothing done.
#[tokio::test]
async fn an_invalid_resolution_file_is_refused_while_another_run_holds_the_lock() {
    with_app_at(start(), |_router, ctx, _clock| async move {
        let ctx = with_source(&ctx, &source_url());
        let paths = Paths::new();
        fs::write(paths.resolutions(), "version = 2\n").expect("write the resolution file");

        let other_run = hold_task_lock(&ctx.db, TASK).await;
        let message = refusal(&ctx, paths.arguments()).await;
        assert_eq!(
            message,
            format!(
                "the argument resolutions is {}, which is not a valid resolution file \
                 (data-migration.md §8.2; its schema is documented in \
                 src/domain/import_mysql/resolutions.rs): 1 problem:\n- version: 2 is not a \
                 schema version this build reads: it reads version 1",
                paths.resolutions().display()
            )
        );
        assert!(
            paths.report_files().is_empty(),
            "a refused run wrote a report"
        );
        other_run.rollback().await.expect("release the other run");
    })
    .await;
}

#[tokio::test]
async fn an_unset_source_is_refused_in_both_spellings_naming_the_setting() {
    with_app_at(start(), |_router, ctx, _clock| async move {
        let paths = Paths::new();
        for unset in ["", "disabled"] {
            let message = refusal(&with_source(&ctx, unset), paths.arguments()).await;
            assert!(
                message
                    .starts_with("refusing to start: settings.import.source_database_url is unset"),
                "{unset:?}: {message}"
            );
            assert!(message.contains("IMPORT_SOURCE_DATABASE_URL"), "{message}");
        }
        assert!(
            paths.report_files().is_empty(),
            "a refused run wrote a report"
        );
    })
    .await;
}

#[tokio::test]
async fn an_unusable_source_is_refused_without_showing_the_url() {
    with_app_at(start(), |_router, ctx, _clock| async move {
        let paths = Paths::new();
        for (source, problem) in [
            (
                format!("postgres://backend3_import:{PASSWORD}@127.0.0.1:9/verseghy"),
                "is not a mysql:// URL",
            ),
            (
                format!("mysql://backend3_import:{PASSWORD}@[127.0.0.1/verseghy"),
                "is not a URL",
            ),
            (
                format!("mysql://backend3_import:{PASSWORD}@127.0.0.1:9/verseghy?ssl-mode=sometimes"),
                "the MySQL driver refuses",
            ),
            (
                format!("mysql://backend3_import:{PASSWORD}@127.0.0.1:9/verseghy?statement-cache-capacity=many"),
                "the MySQL driver refuses",
            ),
        ] {
            let message = refusal(&with_source(&ctx, &source), paths.arguments()).await;
            assert!(message.contains(problem), "{problem}: {message}");
            assert!(message.contains("settings.import.source_database_url"), "{message}");
            assert!(!message.contains(PASSWORD), "{message}");
            assert!(!message.contains("backend3_import"), "{message}");
        }
        assert!(paths.report_files().is_empty(), "a refused run wrote a report");
    })
    .await;
}

/// ADR 0024 §4, layer 3: in the test environment (as in development and
/// e2e) the task refuses a source that is not local, before the lock and
/// before any connection, naming the setting and never showing the URL.
#[tokio::test]
async fn a_source_that_is_not_local_is_refused_naming_the_setting() {
    with_app_at(start(), |_router, ctx, _clock| async move {
        let paths = Paths::new();
        for host in [
            // The compose service's container-network name.
            "mysql:3306",
            "source-host-3f9a.example",
            "127.0.0.2:3306",
            "[::2]:3306",
            "local%68ost",
            "localhost.",
        ] {
            let source = format!("mysql://backend3_import:{PASSWORD}@{host}/verseghy");
            let message = refusal(&with_source(&ctx, &source), paths.arguments()).await;
            assert!(
                message.starts_with(
                    "refusing to start: LOCO_ENV=test, but settings.import.source_database_url \
                     (IMPORT_SOURCE_DATABASE_URL) points at a MySQL server that is not local."
                ),
                "{host}: {message}"
            );
            for shown in [PASSWORD, "backend3_import", host] {
                assert!(!message.contains(shown), "{shown}: {message}");
            }
        }
        // Behind the lock nothing ran: no report, as for every refusal.
        assert!(
            paths.report_files().is_empty(),
            "a refused run wrote a report"
        );
    })
    .await;
}

/// The local spellings pass the guard: the run goes on, takes the lock and
/// opens the source session, which fails at the connection, and writes its
/// report. A socket parameter replaces the host, as in the MySQL driver.
/// (`[::1]` passes the guard as well, `source.rs`'s unit tests show; the
/// compose source listens on 127.0.0.1 only, and the pool would retry a
/// refused connection for its whole acquire timeout.)
#[tokio::test]
async fn a_local_source_passes_the_guard() {
    compose_source().await;
    with_app_at(start(), |_router, ctx, _clock| async move {
        for (source, problem) in [
            (
                format!("mysql://backend3_import:{PASSWORD}@localhost:3306/verseghy"),
                "the MySQL server answered with error 1045 (SQLSTATE 28000)",
            ),
            (
                format!(
                    "mysql://backend3_import:{PASSWORD}@source-host-3f9a.example/verseghy\
                     ?socket=%2Frun%2Fmysqld-absent-7c1d%2Fmysqld.sock"
                ),
                "the connection to the source failed",
            ),
        ] {
            let paths = Paths::new();
            let message = refusal(&with_source(&ctx, &source), paths.arguments()).await;
            assert!(
                message.starts_with("opening the source session from the source failed: "),
                "{message}"
            );
            assert!(message.contains(problem), "{problem}: {message}");
            assert_eq!(
                paths.report_files(),
                ["report.md", "signoff.md", "summary.json"]
            );
            let text = fs::read_to_string(paths.report().join("summary.json")).expect("summary");
            let summary: Value = serde_json::from_str(&text).expect("JSON");
            assert_eq!(summary["outcome"], "failed");
            assert_eq!(summary["error"], message.as_str());
            assert_eq!(summary["source"], Value::Null);
        }
    })
    .await;
}

/// A run that fails at the source keeps every part of the URL out of its
/// error and its report (data-migration.md §4.1).
#[tokio::test]
async fn the_report_never_holds_the_source_url() {
    compose_source().await;
    with_app_at(start(), |_router, ctx, _clock| async move {
        let paths = Paths::new();
        let message = refusal(&with_source(&ctx, &refused_source_url()), paths.arguments()).await;
        assert!(message.contains("error 1045"), "{message}");
        assert!(!message.contains(PASSWORD), "{message}");
        for file in ["summary.json", "report.md"] {
            let text = fs::read_to_string(paths.report().join(file)).expect(file);
            assert!(text.contains("error 1045"), "{file}: {text}");
            assert!(!text.contains(PASSWORD), "{file}: {text}");
            assert!(!text.contains("backend3_import"), "{file}: {text}");
        }
        // The sign-off sheet holds no error text at all: it says only that
        // the run stopped before its detectors.
        let sheet = fs::read_to_string(paths.report().join("signoff.md")).expect("signoff.md");
        assert!(sheet.contains(signoff::STOPPED_FIRST), "{sheet}");
        for leaked in ["1045", PASSWORD, "backend3_import", "127.0.0.1"] {
            assert!(!sheet.contains(leaked), "{leaked} in {sheet}");
        }
    })
    .await;
}

/// The number of rows of every table the import writes that the target
/// has.
pub(crate) async fn target_row_counts(db: &DatabaseConnection) -> Vec<(&'static str, i64)> {
    let mut counts = Vec::new();
    for table in IMPORT_TARGET_TABLES {
        let exists: bool = scalar(db, &format!("SELECT to_regclass('{table}') IS NOT NULL")).await;
        if exists {
            counts.push((
                table,
                scalar(db, &format!("SELECT count(*) FROM \"{table}\"")).await,
            ));
        }
    }
    counts
}

/// The single value in the first column of the first row of `sql`.
async fn scalar<T: sea_orm::TryGetable>(db: &DatabaseConnection, sql: &str) -> T {
    db.query_one_raw(Statement::from_string(DbBackend::Postgres, sql))
        .await
        .unwrap_or_else(|error| panic!("{sql}: {error}"))
        .expect("one row")
        .try_get_by_index(0)
        .expect("a value")
}

/// data-migration.md §11.1: a dry run, and so far an apply run too, since
/// no step writes yet, leave every table the import writes empty, and a
/// later run into the same database is not refused.
#[tokio::test]
async fn dry_run_and_apply_leave_the_target_empty() {
    compose_source().await;
    with_app_at(start(), |_router, ctx, _clock| async move {
        let ctx = with_source(&ctx, COMPOSE_SOURCE_URL);
        for mode in ["dry-run", "apply", "dry-run"] {
            let paths = Paths::new();
            let mut arguments = paths.arguments();
            arguments.insert("mode".to_owned(), mode.to_owned());
            run(&ctx, arguments)
                .await
                .unwrap_or_else(|error| panic!("{mode}: {error}"));
            for (table, rows) in target_row_counts(&ctx.db).await {
                assert_eq!(rows, 0, "{mode}: {table}");
            }
        }
    })
    .await;
}

/// data-migration.md §4.2: the task refuses a target in which a table it
/// writes holds a row, before it reads the source, and names the table in
/// its error and its report.
#[tokio::test]
async fn the_task_refuses_a_target_that_holds_a_row() {
    compose_source().await;
    with_app_at(start(), |_router, ctx, _clock| async move {
        let ctx = with_source(&ctx, COMPOSE_SOURCE_URL);
        activity_log::record(
            &ctx.db,
            Actor::Import,
            Entry {
                entity: Some(EntityRef::table("labels")),
                ..Entry::new(Action::Import)
            },
        )
        .await
        .expect("an activity entry");
        for (table, write_first) in [("activity_log", false), ("users", true)] {
            if write_first {
                ctx.db
                    .execute_unprepared(
                        "INSERT INTO users (pid, name, email) VALUES (gen_random_uuid(), \
                         'Próba', 'elso@example.test')",
                    )
                    .await
                    .expect("an account");
            }
            let paths = Paths::new();
            let message = refusal(&ctx, paths.arguments()).await;
            assert!(
                message.starts_with(&format!(
                    "refusing to import: the target table `{table}` already holds a row."
                )),
                "{table}: {message}"
            );
            let text = fs::read_to_string(paths.report().join("summary.json")).expect("summary");
            let summary: Value = serde_json::from_str(&text).expect("JSON");
            assert_eq!(summary["outcome"], "failed");
            assert_eq!(summary["error"], message.as_str());
            // Refused before the source session.
            assert_eq!(summary["source"], Value::Null);
        }
    })
    .await;
}

/// The import maps the legacy roles onto the target's `roles` rows by
/// name, so a target whose roles differ from the code is refused first.
#[tokio::test]
async fn the_task_refuses_a_target_whose_roles_differ_from_the_code() {
    compose_source().await;
    with_app_at(start(), |_router, ctx, _clock| async move {
        let ctx = with_source(&ctx, COMPOSE_SOURCE_URL);
        ctx.db
            .execute_unprepared("DELETE FROM roles WHERE id = 4")
            .await
            .expect("remove a role");
        let message = refusal(&ctx, Paths::new().arguments()).await;
        assert!(
            message.contains(": missing 4 `admin`. Migrate the target"),
            "{message}"
        );
    })
    .await;
}

/// ADR 0016 (Consequences): a run that finds the lock held exits 0 and does
/// none of the work; here, it writes no report.
#[tokio::test]
async fn the_task_exits_successfully_without_its_work_while_another_run_holds_the_lock() {
    compose_source().await;
    with_app_at(start(), |_router, ctx, _clock| async move {
        let ctx = with_source(&ctx, COMPOSE_SOURCE_URL);
        let paths = Paths::new();

        let other_run = hold_task_lock(&ctx.db, TASK).await;
        run(&ctx, paths.arguments())
            .await
            .expect("exit 0 while the lock is held");
        assert!(
            paths.report_files().is_empty(),
            "the run must not write a report"
        );

        other_run.rollback().await.expect("release the other run");
        run(&ctx, paths.arguments())
            .await
            .expect("a free lock runs the task");
        assert_eq!(
            paths.report_files(),
            ["report.md", "signoff.md", "summary.json"]
        );
    })
    .await;
}

#[tokio::test]
async fn a_second_run_into_the_same_report_directory_is_refused() {
    compose_source().await;
    with_app_at(start(), |_router, ctx, _clock| async move {
        let ctx = with_source(&ctx, COMPOSE_SOURCE_URL);
        let paths = Paths::new();
        run(&ctx, paths.arguments()).await.expect("the first run");
        let first = fs::read_to_string(paths.report().join("summary.json")).expect("summary");
        let message = refusal(&ctx, paths.arguments()).await;
        assert!(message.contains("which is not empty"), "{message}");
        let kept = fs::read_to_string(paths.report().join("summary.json")).expect("summary");
        assert_eq!(kept, first, "the first run's report was changed");
    })
    .await;
}

/// The options of a run whose report goes into `report`, for the tests that
/// call the report writer directly.
fn options_into(report: &Path) -> ImportOptions {
    ImportOptions {
        mode: Mode::DryRun,
        snapshot_at: "2026-10-23T17:01:02Z".parse().expect("RFC 3339"),
        resolutions: PathBuf::from(DEFAULT_RESOLUTIONS),
        storage_root: PathBuf::from("/storage"),
        report: report.to_path_buf(),
        canteen_audit_local_before: CanteenAuditLocalBefore::default_cutoff(),
    }
}

/// The permission bits of `path`.
fn mode_of(path: &Path) -> u32 {
    fs::metadata(path)
        .unwrap_or_else(|error| panic!("{}: {error}", path.display()))
        .permissions()
        .mode()
        & 0o777
}

/// data-migration.md §11.3: the report stays private, so only the task's
/// own user may read its files.
#[tokio::test]
async fn the_report_files_are_readable_by_the_task_user_only() {
    compose_source().await;
    with_app_at(start(), |_router, ctx, _clock| async move {
        let paths = Paths::new();
        run(&with_source(&ctx, COMPOSE_SOURCE_URL), paths.arguments())
            .await
            .expect("the run");
        for file in ["report.md", "signoff.md", "summary.json"] {
            assert_eq!(
                mode_of(&paths.report().join(file)),
                REPORT_FILE_MODE,
                "{file}"
            );
        }
    })
    .await;
}

/// The path check refuses a report directory that is not empty, but the
/// writer does not rely on it: a file that appears after the check is left
/// as it was, and the error names it (§11.3: neither file ever overwrites
/// an existing one).
#[test]
fn the_report_writer_never_overwrites_a_file() {
    let summary = |report: &Path| RunSummary {
        options: options_into(report),
        started_at: start(),
        finished_at: start(),
        outcome: Outcome::Completed,
        parts: ReportParts::default(),
    };

    // An existing report.md: nothing is written at all.
    let directory = ScratchDir::new("import-report-md-exists");
    fs::write(directory.path().join("report.md"), "an earlier report\n").expect("report.md");
    let error = report::write(directory.path(), &summary(directory.path()))
        .expect_err("report.md exists")
        .to_string();
    assert!(
        error.starts_with("cannot write the run report file ") && error.contains("report.md"),
        "{error}"
    );
    assert_eq!(
        fs::read_to_string(directory.path().join("report.md")).expect("report.md"),
        "an earlier report\n"
    );
    assert!(!directory.path().join("summary.json").exists());
    assert!(!directory.path().join("signoff.md").exists());

    // An existing signoff.md: report.md is new, the sheet is kept, and no
    // summary.json says the report is complete.
    let directory = ScratchDir::new("import-signoff-exists");
    fs::write(directory.path().join("signoff.md"), "an earlier sheet\n").expect("signoff.md");
    let error = report::write(directory.path(), &summary(directory.path()))
        .expect_err("signoff.md exists")
        .to_string();
    assert!(error.contains("signoff.md"), "{error}");
    assert_eq!(
        fs::read_to_string(directory.path().join("signoff.md")).expect("signoff.md"),
        "an earlier sheet\n"
    );
    assert!(directory.path().join("report.md").exists());
    assert!(!directory.path().join("summary.json").exists());

    // An existing summary.json: report.md is new, summary.json is kept.
    let directory = ScratchDir::new("import-summary-exists");
    fs::write(directory.path().join("summary.json"), "{}\n").expect("summary.json");
    let error = report::write(directory.path(), &summary(directory.path()))
        .expect_err("summary.json exists")
        .to_string();
    assert!(error.contains("summary.json"), "{error}");
    assert_eq!(
        fs::read_to_string(directory.path().join("summary.json")).expect("summary.json"),
        "{}\n"
    );

    // A dangling symbolic link is not followed: its target is not created.
    let directory = ScratchDir::new("import-report-link");
    let target = directory.path().join("elsewhere.md");
    std::os::unix::fs::symlink(&target, directory.path().join("report.md")).expect("a link");
    report::write(directory.path(), &summary(directory.path())).expect_err("report.md is a link");
    assert!(!target.exists(), "the writer followed the link");
}

/// A run whose import fails still writes its report, with the outcome
/// `failed`, the error and the parts of the steps that ended before it
/// failed, and the task returns the import's error.
#[test]
fn a_failed_run_writes_its_report_and_returns_its_own_error() {
    let directory = ScratchDir::new("import-failed-run");
    let message = "V8 failed: duplicate live canteen dates";
    let finished_at = start() + TimeDelta::milliseconds(1_500);
    // The source session ended before the step that failed.
    let parts = ReportParts {
        source: Some(
            SourceFacts::checked(
                SOURCE_TABLES
                    .iter()
                    .map(|table| AutoIncrement {
                        table: *table,
                        next_id: Some(7),
                    })
                    .collect(),
                None,
                start(),
            )
            .expect("an empty source bounds no snapshot_at"),
        ),
        ..ReportParts::default()
    };

    let returned = report::finish(
        &options_into(directory.path()),
        start(),
        finished_at,
        parts,
        Err(loco_rs::Error::Message(message.to_owned())),
    )
    .expect_err("the run's error");

    assert_eq!(returned.to_string(), message);
    let text = fs::read_to_string(directory.path().join("summary.json")).expect("summary");
    let summary: Value = serde_json::from_str(&text).expect("JSON");
    assert_eq!(summary["outcome"], "failed");
    assert_eq!(summary["error"], message);
    assert_eq!(summary["duration_ms"], 1_500);
    assert_eq!(summary["source"]["fingerprint"], "matched");
    assert_eq!(summary["source"]["auto_increment"]["users"], 7);
    let markdown = fs::read_to_string(directory.path().join("report.md")).expect("report.md");
    assert!(markdown.contains("| Outcome | failed |"), "{markdown}");
    assert!(
        markdown.contains(&format!("\n## Error\n\n```text\n{message}\n```\n")),
        "{markdown}"
    );
    for file in ["report.md", "signoff.md", "summary.json"] {
        assert_eq!(
            mode_of(&directory.path().join(file)),
            REPORT_FILE_MODE,
            "{file}"
        );
    }
}

/// Which error the task returns when the report cannot be written: the
/// import's own error for a failed run, the report's for a completed one.
#[test]
fn a_report_that_cannot_be_written_fails_only_a_completed_run() {
    let blocked = || {
        let directory = ScratchDir::new("import-report-blocked");
        fs::write(directory.path().join("report.md"), "in the way\n").expect("report.md");
        directory
    };

    let directory = blocked();
    let returned = report::finish(
        &options_into(directory.path()),
        start(),
        start(),
        ReportParts::default(),
        Err(loco_rs::Error::Message("V3 failed".to_owned())),
    )
    .expect_err("the run's error");
    assert_eq!(returned.to_string(), "V3 failed");

    let directory = blocked();
    let returned = report::finish(
        &options_into(directory.path()),
        start(),
        start(),
        ReportParts::default(),
        Ok(()),
    )
    .expect_err("the report's error")
    .to_string();
    assert!(
        returned.starts_with("cannot write the run report file ") && returned.contains("report.md"),
        "{returned}"
    );
}
