//! Tests of the `import_mysql` task (ADR 0017, data-migration.md §4.1,
//! §11.3), run as Loco runs it: `run_task::<App>` with the task's
//! arguments, on the test's own database.
//!
//! The source is a URL that nothing listens on: the task opens the source
//! connection on first use, and no import step reads the source yet. Each
//! test sets the source in its own copy of the app context, because
//! config/test.yaml configures none.
//!
//! No import step can fail yet, so the tests of a failed run and of the
//! report writer's own refusals call `tasks::import_mysql::report` directly,
//! with the result a failing step would return.

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
        clock::Clock,
        import_mysql::{
            not_migrated::NOT_MIGRATED_TABLES,
            options::{CanteenAuditLocalBefore, DEFAULT_RESOLUTIONS, ImportOptions, Mode},
            report::{NO_STEP_YET, Outcome, RunSummary, escape_cell},
        },
    },
    tasks::import_mysql::report::{self, REPORT_FILE_MODE},
};
use chrono::{DateTime, TimeDelta, Utc};
use loco_rs::{app::AppContext, boot::run_task, task::Vars};
use serde_json::{Value, json};

use crate::support::{ScratchDir, hold_task_lock, with_app, with_app_at};

/// The task's registered name, which is also its lock name.
const TASK: &str = "import_mysql";

/// The password of the source URL: no message, log or report may show it.
const PASSWORD: &str = "pw-source-never-shown-5b1e";

/// A source URL in the shape of production's, on the discard port of the
/// local host, where nothing answers.
fn source_url() -> String {
    format!("mysql://backend3_import:{PASSWORD}@127.0.0.1:9/verseghy")
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

            run(&with_source(&ctx, &source_url()), arguments)
                .await
                .unwrap_or_else(|error| panic!("{mode}: {error}"));

            assert_eq!(
                paths.report_files(),
                ["report.md", "summary.json"],
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
            let markdown = fs::read_to_string(paths.report().join("report.md")).expect("report.md");
            assert!(markdown.contains("| Duration | 2.750 s |"), "{markdown}");
            assert!(markdown.contains(NO_STEP_YET), "{markdown}");
            for row in NOT_MIGRATED_TABLES {
                let cell = escape_cell(row.table);
                assert!(markdown.contains(&cell), "{cell} in {markdown}");
            }
        }
    })
    .await;
}

#[tokio::test]
async fn a_run_at_a_standing_clock_takes_no_time() {
    with_app_at(start(), |_router, ctx, _clock| async move {
        let paths = Paths::new();
        run(&with_source(&ctx, &source_url()), paths.arguments())
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

#[tokio::test]
async fn the_report_never_holds_the_source_url() {
    with_app_at(start(), |_router, ctx, _clock| async move {
        let paths = Paths::new();
        run(&with_source(&ctx, &source_url()), paths.arguments())
            .await
            .expect("the run");
        for file in ["summary.json", "report.md"] {
            let text = fs::read_to_string(paths.report().join(file)).expect(file);
            assert!(!text.contains(PASSWORD), "{file}: {text}");
            assert!(!text.contains("backend3_import"), "{file}: {text}");
        }
    })
    .await;
}

/// ADR 0016 (Consequences): a run that finds the lock held exits 0 and does
/// none of the work; here, it writes no report.
#[tokio::test]
async fn the_task_exits_successfully_without_its_work_while_another_run_holds_the_lock() {
    with_app_at(start(), |_router, ctx, _clock| async move {
        let ctx = with_source(&ctx, &source_url());
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
        assert_eq!(paths.report_files(), ["report.md", "summary.json"]);
    })
    .await;
}

#[tokio::test]
async fn a_second_run_into_the_same_report_directory_is_refused() {
    with_app_at(start(), |_router, ctx, _clock| async move {
        let ctx = with_source(&ctx, &source_url());
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
    with_app_at(start(), |_router, ctx, _clock| async move {
        let paths = Paths::new();
        run(&with_source(&ctx, &source_url()), paths.arguments())
            .await
            .expect("the run");
        for file in ["report.md", "summary.json"] {
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
/// `failed` and the error, and the task returns the import's error.
#[test]
fn a_failed_run_writes_its_report_and_returns_its_own_error() {
    let directory = ScratchDir::new("import-failed-run");
    let message = "V8 failed: duplicate live canteen dates";
    let finished_at = start() + TimeDelta::milliseconds(1_500);

    let returned = report::finish(
        &options_into(directory.path()),
        start(),
        finished_at,
        Err(loco_rs::Error::Message(message.to_owned())),
    )
    .expect_err("the run's error");

    assert_eq!(returned.to_string(), message);
    let text = fs::read_to_string(directory.path().join("summary.json")).expect("summary");
    let summary: Value = serde_json::from_str(&text).expect("JSON");
    assert_eq!(summary["outcome"], "failed");
    assert_eq!(summary["error"], message);
    assert_eq!(summary["duration_ms"], 1_500);
    let markdown = fs::read_to_string(directory.path().join("report.md")).expect("report.md");
    assert!(markdown.contains("| Outcome | failed |"), "{markdown}");
    assert!(
        markdown.contains(&format!("\n## Error\n\n```text\n{message}\n```\n")),
        "{markdown}"
    );
    for file in ["report.md", "summary.json"] {
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
        Err(loco_rs::Error::Message("V3 failed".to_owned())),
    )
    .expect_err("the run's error");
    assert_eq!(returned.to_string(), "V3 failed");

    let directory = blocked();
    let returned = report::finish(&options_into(directory.path()), start(), start(), Ok(()))
        .expect_err("the report's error")
        .to_string();
    assert!(
        returned.starts_with("cannot write the run report file ") && returned.contains("report.md"),
        "{returned}"
    );
}
