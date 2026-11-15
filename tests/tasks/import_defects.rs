//! The defect detector as a run of the `import_mysql` task uses it
//! (ADR 0017 §3, ADR 0047, data-migration.md §4.2, §8.2, §11.1): a
//! synthetic resolution file read by the task's loader, a synthetic rule
//! over synthetic legacy rows, and the report and sign-off sheet that the
//! task writes from what the detector recorded.
//!
//! The table steps that bring the real rules come with their own slices;
//! these tests pin what every one of them inherits: `apply` stops at the
//! first problem, a dry run lists every problem and fails at its end, and
//! the sign-off sheet holds ids, public paths and proposed resolutions,
//! never a row's content. Most of them need no database: they end the run
//! with the same `DefectsPart::conclude` as the task's `run`, and write the
//! report with the task's own writer. The last two go through the task's
//! own `run` (`tasks::import_mysql::run`), against the MySQL source of
//! compose.yaml and the test's own database, so that `run` itself is seen
//! to fail a dry run that collected a problem, after every step, and to
//! pass a run whose defect check holds.

use std::{fs, num::NonZeroU32, os::unix::fs::PermissionsExt, path::PathBuf};

use backend3::{
    domain::import_mysql::{
        defects::{
            Decision, DefectDetector, DefectId, DefectRule, DefectsPart, Instance, Proposal,
            Subject, Value,
        },
        options::{CanteenAuditLocalBefore, ImportOptions, Mode},
        report::ReportParts,
        resolutions::{PostDateShift, ResolutionFile, SENTINEL_DATE},
        signoff,
    },
    tasks::import_mysql::{
        report::{self, REPORT_FILE_MODE},
        resolutions::load,
        run,
    },
};
use chrono::{DateTime, NaiveDate, Utc};
use serde_json::{Value as Json, json};

use super::{import_mysql::target_row_counts, import_mysql_source::compose_source};
use crate::support::{ScratchDir, with_app};

/// The synthetic resolution file: posts 12, 14 and 16 signed for D1. Post
/// 14's date changed after the sign-off, and post 16 is gone from the
/// source.
const SIGNED: &str = "version = 1
signed_off = 2026-11-20

[[post_date_shift]]          # D1
post_id = 12
expect_date = 2019-05-02     # precondition

[[post_date_shift]]
post_id = 14
expect_date = 2020-02-29

[[post_date_shift]]
post_id = 16
expect_date = 2020-03-01
";

/// A synthetic legacy post with what the D1 rule reads, and a title, which
/// a real row has and which must never reach the sign-off sheet.
struct LegacyPost {
    id: u32,
    /// `created_at` before 2022-01-21, the end of the cast that left the
    /// dates one day early (data-migration.md §8.1).
    created_early: bool,
    date: Option<NaiveDate>,
    title: &'static str,
}

fn day(text: &str) -> NaiveDate {
    NaiveDate::parse_from_str(text, "%Y-%m-%d").expect("a date")
}

fn id(number: u32) -> NonZeroU32 {
    NonZeroU32::new(number).expect("a row id")
}

/// Post 12 as signed, post 14 with a corrected date, post 30 unsigned,
/// post 20 without the defect.
fn rows() -> Vec<LegacyPost> {
    vec![
        LegacyPost {
            id: 12,
            created_early: true,
            date: Some(day("2019-05-02")),
            title: "Synthetic post twelve",
        },
        LegacyPost {
            id: 14,
            created_early: true,
            date: Some(day("2020-03-02")),
            title: "Synthetic post fourteen",
        },
        LegacyPost {
            id: 20,
            created_early: false,
            date: Some(day("2023-03-01")),
            title: "Synthetic post twenty",
        },
        LegacyPost {
            id: 30,
            created_early: true,
            date: Some(day("2021-06-01")),
            title: "Synthetic post thirty",
        },
    ]
}

/// A D1 rule over the synthetic posts, in the shape the posts step will
/// have: an early post with a real date moves one day later, by default or
/// by its entry.
struct EarlyDates;

impl DefectRule for EarlyDates {
    type Row = LegacyPost;
    type Entry = PostDateShift;
    type Resolution = NaiveDate;
    const DEFECT: DefectId = DefectId::D1;

    fn detect(&self, rows: &[LegacyPost]) -> Vec<Instance<NaiveDate>> {
        rows.iter()
            .filter_map(|row| {
                assert!(!row.title.is_empty(), "the rule sees the whole row");
                let found = row
                    .date
                    .filter(|date| row.created_early && *date != SENTINEL_DATE)?;
                let next = found.succ_opt()?;
                Some(
                    Instance::new(Subject::Post(id(row.id)), Value::Date(found)).with_proposed(
                        Proposal {
                            resolution: next,
                            shown: Value::Date(next),
                        },
                    ),
                )
            })
            .collect()
    }

    fn found(&self, rows: &[LegacyPost], subject: &Subject) -> Option<Value> {
        let Subject::Post(post) = subject else {
            return None;
        };
        rows.iter()
            .find(|row| row.id == post.get())
            .map(|row| row.date.map_or(Value::Null, Value::Date))
    }

    fn resolution(&self, entry: &PostDateShift) -> Proposal<NaiveDate> {
        let next = entry.expect_date.succ_opt().expect("a later day");
        Proposal {
            resolution: next,
            shown: Value::Date(next),
        }
    }
}

fn start() -> DateTime<Utc> {
    "2026-10-31T18:00:00Z".parse().expect("RFC 3339")
}

/// The synthetic resolution file, written to `directory` and read by the
/// task's loader, as a run reads it before the task lock.
fn signed_file(directory: &ScratchDir) -> ResolutionFile {
    file_from(directory, SIGNED)
}

/// The resolution file `text`, written to `directory` and read by the
/// task's loader.
fn file_from(directory: &ScratchDir, text: &str) -> ResolutionFile {
    let path = directory.path().join("resolutions.toml");
    fs::write(&path, text).expect("the synthetic resolution file");
    load(&path).expect("a valid synthetic file").file
}

/// The options of a run in `mode` whose report goes into `report`.
fn options(mode: Mode, report: PathBuf) -> ImportOptions {
    ImportOptions {
        mode,
        snapshot_at: "2026-10-23T17:01:02Z".parse().expect("RFC 3339"),
        resolutions: PathBuf::from("resolutions.toml"),
        storage_root: PathBuf::from("/storage"),
        report,
        canteen_audit_local_before: CanteenAuditLocalBefore::default_cutoff(),
    }
}

/// `summary.json` of the report in `directory`.
fn summary(directory: &std::path::Path) -> Json {
    let text = fs::read_to_string(directory.join("summary.json")).expect("summary.json");
    serde_json::from_str(&text).expect("JSON")
}

#[test]
fn a_dry_run_lists_every_defect_problem_in_its_report_and_fails_at_its_end() {
    let scratch = ScratchDir::new("import-defects-dry-run");
    let file = signed_file(&scratch);
    let report_dir = scratch.path().join("report");
    fs::create_dir(&report_dir).expect("the report directory");

    let mut parts = ReportParts::default();
    let decisions = DefectDetector::new(Mode::DryRun, &file)
        .run(&EarlyDates, &rows(), &mut parts.defects)
        .expect("a dry run goes on past every problem");
    // The signed post takes its entry, the changed and the unsigned one the
    // proposed default, and the post without the defect is no instance.
    assert_eq!(
        decisions.get(&Subject::Post(id(12))),
        Some(&Decision::Entry(day("2019-05-03")))
    );
    assert_eq!(
        decisions.get(&Subject::Post(id(14))),
        Some(&Decision::Default(day("2020-03-03")))
    );
    assert_eq!(
        decisions.get(&Subject::Post(id(30))),
        Some(&Decision::Default(day("2021-06-02")))
    );
    assert_eq!(decisions.get(&Subject::Post(id(20))), None);

    // The end of the run, as `tasks::import_mysql::run` ends it.
    let result = parts
        .defects
        .conclude()
        .map_err(|problems| loco_rs::Error::Message(problems.to_string()));
    let returned = report::finish(
        &options(Mode::DryRun, report_dir.clone()),
        start(),
        start(),
        parts,
        result,
    )
    .expect_err("the dry run exits non-zero")
    .to_string();
    assert!(
        returned.starts_with(
            "the run found 3 defect problems (data-migration.md §4.2, §11.1): 1 instance that \
             the resolution file does not cover, and 2 entries whose precondition does not hold."
        ),
        "{returned}"
    );
    for problem in [
        "\n- D1 post 14: the precondition of post_date_shift[2] does not hold: expect_date is \
         2020-02-29, but the source has 2020-03-02.",
        "\n- D1 post 16: post_date_shift[3] covers it, but the D1 rule no longer finds it \
         (expect_date is 2020-03-01, the source has no such row)",
        "\n- D1 post 30 is not covered by the resolution file (its source value is 2021-06-01)",
    ] {
        assert!(returned.contains(problem), "{problem} in {returned}");
    }

    let summary = summary(&report_dir);
    assert_eq!(summary["outcome"], "failed");
    assert_eq!(summary["error"], returned.as_str());
    assert_eq!(summary["defects"]["complete"], true);
    assert_eq!(summary["defects"]["checked"], json!(["D1"]));
    assert_eq!(summary["defects"]["uncovered"], 1);
    assert_eq!(summary["defects"]["refused"], 2);
    let outcomes: Vec<&str> = summary["defects"]["records"]
        .as_array()
        .expect("records")
        .iter()
        .map(|record| record["outcome"].as_str().expect("an outcome"))
        .collect();
    assert_eq!(outcomes, ["covered", "changed", "stale", "uncovered"]);

    let sheet = fs::read_to_string(report_dir.join("signoff.md")).expect("signoff.md");
    assert!(
        sheet.contains(&format!(
            "\n\n{}\n\n## D1: Post dates one day early\n\n| Rows | Open on the live site | \
             Proposed resolution |\n|---|---|---|\n| 12 | /posts/12 | 2019-05-03 |\n| 14 | \
             /posts/14 | 2020-03-03 |\n| 30 | /posts/30 | 2021-06-02 |\n",
            signoff::COMPLETE
        )),
        "{sheet}"
    );
    for row in rows() {
        assert!(!sheet.contains(row.title), "{sheet}");
    }
    for file in ["report.md", "signoff.md", "summary.json"] {
        let mode = fs::metadata(report_dir.join(file))
            .expect(file)
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, REPORT_FILE_MODE, "{file}");
    }
}

#[test]
fn apply_stops_at_the_first_problem_and_its_report_shows_how_far_it_got() {
    let scratch = ScratchDir::new("import-defects-apply");
    let file = signed_file(&scratch);
    let report_dir = scratch.path().join("report");
    fs::create_dir(&report_dir).expect("the report directory");

    let mut parts = ReportParts::default();
    let refusal = DefectDetector::new(Mode::Apply, &file)
        .run(&EarlyDates, &rows(), &mut parts.defects)
        .expect_err("apply stops at post 14");
    let message = "refusing to import: D1 post 14: the precondition of post_date_shift[2] does \
                   not hold: expect_date is 2020-02-29, but the source has 2020-03-02. The row \
                   changed since the sign-off: sign the entry again (data-migration.md §8.2)";
    assert_eq!(refusal.to_string(), message);

    let returned = report::finish(
        &options(Mode::Apply, report_dir.clone()),
        start(),
        start(),
        parts,
        Err(loco_rs::Error::Message(refusal.to_string())),
    )
    .expect_err("the run's own error");
    assert_eq!(returned.to_string(), message);

    let summary = summary(&report_dir);
    assert_eq!(summary["error"], message);
    // Posts 12 and 14 were judged; the run stopped before post 16 and 30.
    assert_eq!(summary["defects"]["complete"], false);
    let records = summary["defects"]["records"].as_array().expect("records");
    assert_eq!(records.len(), 2);
    // The record that stopped the run shows what the import proposes, the
    // default; it never claims that the run applied it, as the run rolled
    // back.
    assert_eq!(records[1]["outcome"], "changed");
    assert_eq!(records[1]["proposed"], "2020-03-03");
    assert_eq!(records[1].get("applied"), None);
    let sheet = fs::read_to_string(report_dir.join("signoff.md")).expect("signoff.md");
    assert!(
        sheet.contains(&format!("\n\n{}\n\n## D1: ", signoff::INCOMPLETE)),
        "{sheet}"
    );
    assert!(
        sheet.ends_with("| 14 | /posts/14 | 2020-03-03 |\n"),
        "{sheet}"
    );
}

#[test]
fn the_empty_list_leaves_every_instance_uncovered() {
    // The committed list starts empty (data-migration.md §8.2): until it is
    // filled, a dry run proposes the default for every instance, and apply
    // refuses the first.
    let scratch = ScratchDir::new("import-defects-empty-list");
    let path = scratch.path().join("resolutions.toml");
    fs::write(&path, "version = 1\n").expect("the empty list");
    let file = load(&path).expect("the empty list is valid").file;

    let mut dry = DefectsPart::default();
    DefectDetector::new(Mode::DryRun, &file)
        .run(&EarlyDates, &rows(), &mut dry)
        .expect("a dry run goes on");
    assert_eq!((dry.uncovered(), dry.refused()), (3, 0));
    let problems = dry.conclude().expect_err("three uncovered instances");
    assert_eq!(problems.lines.len(), 3);

    let mut applied = DefectsPart::default();
    let refusal = DefectDetector::new(Mode::Apply, &file)
        .run(&EarlyDates, &rows(), &mut applied)
        .expect_err("apply refuses the first");
    assert!(
        refusal
            .to_string()
            .starts_with("refusing to import: D1 post 12 is not covered by the resolution file"),
        "{refusal}"
    );
}

/// Compose source: the task's own `run` concludes the defect check last.
/// A dry run whose detectors collected a problem goes through every step
/// (the source session included), then fails naming the problem, with its
/// Defects part complete, and leaves the target empty (ADR 0047 §2,
/// data-migration.md §4.2).
#[tokio::test]
async fn the_tasks_run_fails_a_dry_run_that_collected_a_problem_at_its_end() {
    let pool = compose_source().await;
    let source = &pool;
    with_app(|_router, ctx| async move {
        let scratch = ScratchDir::new("import-defects-task-dry-run");
        let file = file_from(&scratch, "version = 1\n");
        let options = options(Mode::DryRun, scratch.path().join("report"));
        let mut parts = ReportParts::default();
        // Post 12 alone, which the empty list does not cover.
        DefectDetector::new(Mode::DryRun, &file)
            .run(&EarlyDates, &rows()[..1], &mut parts.defects)
            .expect("a dry run goes on");

        let error = run(source, &ctx.db, &options, &mut parts)
            .await
            .expect_err("the dry run fails at its end")
            .to_string();
        assert!(
            error.starts_with(
                "the run found 1 defect problem (data-migration.md §4.2, §11.1): 1 instance that \
                 the resolution file does not cover, and 0 entries whose precondition does not hold."
            ),
            "{error}"
        );
        assert!(
            error.contains("\n- D1 post 12 is not covered by the resolution file"),
            "{error}"
        );
        assert!(parts.defects.is_complete());
        // The run went through its steps before it failed.
        assert!(parts.source.is_some());
        for (table, rows) in target_row_counts(&ctx.db).await {
            assert_eq!(rows, 0, "{table}");
        }
    })
    .await;
    pool.close().await;
}

/// Compose source: an `apply` run whose every instance is signed passes the
/// task's own `run`, and its Defects part is complete.
#[tokio::test]
async fn the_tasks_run_passes_a_run_whose_defect_check_holds() {
    let pool = compose_source().await;
    let source = &pool;
    with_app(|_router, ctx| async move {
        let scratch = ScratchDir::new("import-defects-task-apply");
        let file = file_from(
            &scratch,
            "version = 1\n[[post_date_shift]]\npost_id = 12\nexpect_date = 2019-05-02\n",
        );
        let options = options(Mode::Apply, scratch.path().join("report"));
        let mut parts = ReportParts::default();
        let decisions = DefectDetector::new(Mode::Apply, &file)
            .run(&EarlyDates, &rows()[..1], &mut parts.defects)
            .expect("post 12 is signed");
        assert_eq!(
            decisions.get(&Subject::Post(id(12))),
            Some(&Decision::Entry(day("2019-05-03")))
        );

        run(source, &ctx.db, &options, &mut parts)
            .await
            .expect("nothing to refuse");
        assert!(parts.defects.is_complete());
        assert_eq!(parts.defects.records().len(), 1);
        assert!(parts.source.is_some());
    })
    .await;
    pool.close().await;
}
