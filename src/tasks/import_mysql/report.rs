//! Writes the run report into the run's `report` directory
//! (data-migration.md §11.3). The contents come from
//! `domain::import_mysql::report`; this module writes the files and decides
//! what a run returns once its report is written.
//!
//! `report.md` is written first and `summary.json` last, so a directory with
//! a `summary.json` holds a complete report: the nightly rehearsal saves that
//! file (cutover.md §5.2). Each file is created with `create_new`, so a run
//! never overwrites a file, and synced to disk before the task reports
//! success.
//!
//! Both files are created readable and writable by the task's own user only
//! (mode `0600`; the umask can only take bits away): the report holds
//! draft content and account counts and stays private (§11.3), so other
//! users of the machine or the pod may not read it. The directory itself is
//! the operator's, who made it empty for this run.

use std::{fs::OpenOptions, io::Write, os::unix::fs::OpenOptionsExt, path::Path};

use chrono::{DateTime, Utc};
use loco_rs::{Error, Result};

use crate::domain::import_mysql::{
    options::ImportOptions,
    report::{MARKDOWN_FILE, Outcome, RunSummary, SUMMARY_FILE, TASK_NAME},
};

/// The mode of both report files: read and write for the owner only.
pub const REPORT_FILE_MODE: u32 = 0o600;

/// Ends a run: writes its report into `options.report`, logs how it ended
/// and returns what the task returns.
///
/// - `options`: the run's validated arguments; the report goes into
///   `options.report`, and the summary lists them.
/// - `started_at`: the app clock's reading at the start of the run.
/// - `finished_at`: the app clock's reading at its end, taken after the
///   import returned.
/// - `result`: what the import returned. An error becomes the `failed`
///   outcome and its message the report's error.
///
/// The report is written for a failed run too, so the operator can see
/// which step refused and why (§11.3).
///
/// # Errors
///
/// The run's own error when the run failed, whether or not its report was
/// written: that is the one to fix, and a report that a failed run could
/// not write is logged instead. A completed run returns the error of a
/// report file that cannot be written ([`write`]).
pub fn finish(
    options: &ImportOptions,
    started_at: DateTime<Utc>,
    finished_at: DateTime<Utc>,
    result: Result<()>,
) -> Result<()> {
    let summary = RunSummary {
        options: options.clone(),
        started_at,
        finished_at,
        outcome: match &result {
            Ok(()) => Outcome::Completed,
            Err(error) => Outcome::Failed {
                message: error.to_string(),
            },
        },
    };
    let written = write(&options.report, &summary);
    match &written {
        Ok(()) => tracing::info!(
            task = TASK_NAME,
            outcome = summary.outcome.as_str(),
            duration_ms = summary.duration().num_milliseconds(),
            report = %options.report.display(),
            "the import run has written its report"
        ),
        Err(report_error) if result.is_err() => tracing::error!(
            task = TASK_NAME,
            %report_error,
            "the failed import run could not write its report"
        ),
        // A completed run returns this error below.
        Err(_) => {}
    }
    result.and(written)
}

/// Writes `report.md` and `summary.json` for `summary` into `directory`.
///
/// - `directory`: the run's `report` directory, which the path checks found
///   empty.
/// - `summary`: the finished run.
///
/// # Errors
///
/// An [`Error::Message`] naming the file and the operating system's reason
/// when a file exists already or cannot be written. A file that exists
/// already is left as it was.
pub fn write(directory: &Path, summary: &RunSummary) -> Result<()> {
    write_new(&directory.join(MARKDOWN_FILE), &summary.to_markdown())?;
    write_new(&directory.join(SUMMARY_FILE), &summary.to_json())
}

/// Creates `path` with [`REPORT_FILE_MODE`], which must not exist, writes
/// `contents` and syncs it.
fn write_new(path: &Path, contents: &str) -> Result<()> {
    let refuse = |error: std::io::Error| {
        Error::Message(format!(
            "cannot write the run report file {}: {error}",
            path.display()
        ))
    };
    // `create_new` fails if the path exists, also as a dangling symbolic
    // link, so nothing that is there already is truncated or followed. The
    // mode applies only to the file this call creates.
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(REPORT_FILE_MODE)
        .open(path)
        .map_err(refuse)?;
    file.write_all(contents.as_bytes()).map_err(refuse)?;
    file.sync_all().map_err(refuse)
}
