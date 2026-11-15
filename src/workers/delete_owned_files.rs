//! `DeleteOwnedFiles`: removes the owned files that nothing refers to any
//! more, after the transaction that removed their references has committed
//! (ADR 0011 §6 "Files", ADR 0015 §3, ADR 0016 §3).
//!
//! An entity's permanent delete removes its row and its revisions in one
//! transaction; its files (an author's photo, a post's index image and
//! gallery) are on the uploads volume, which no transaction covers. So the
//! purge collects their paths from the row and every revision
//! (`models::owned_files::files_of_record`), commits, and only then enqueues
//! this job with [`commit_then_enqueue`]. The job runs in the backend3 process, which
//! mounts the volume (`start --server-and-worker`, ADR 0016 §3); a `CronJob`
//! pod never does (ADR 0015 §3).
//!
//! The ADR 0016 §3 rules for every worker, as this job keeps them:
//!
//! - **Arguments are paths only** ([`WorkerArgs`]): the paths below
//!   `media.root`, at most `MAX_PATHS_PER_JOB` per job, never row ids (the
//!   rows are gone) and never file contents.
//! - **Idempotent.** A file is removed only when no row, live or trashed, and
//!   no revision snapshot refers to it (`models::owned_files`), and a file
//!   that is already gone counts as removed, so a second run removes nothing
//!   more and does not fail. A replaced or cleared image therefore stays
//!   while a revision shows it, and goes with its record's purge.
//! - **The outcome is the file's absence.** The rule "the job records its
//!   outcome on the row it works on" has no row to apply to here, because
//!   the purge removed it: the job's outcome is that its unreferenced files
//!   are gone. A job that fails leaves those files as orphans, which nothing
//!   refers to and which ADR 0011 §6 counts as harmless ("files are orphaned,
//!   never referenced by a missing row"). The same reading holds for every
//!   job whose outcome is the state of files rather than of a row, such as
//!   the media trash purge and the sitemap.
//! - **A failure returns an error, without a retry loop.** Loco 1.2.0 marks
//!   the job `failed` with the error's text, and the recovery is the second
//!   one ADR 0016 §3 names: an operator runs `jobs retry` once the cause is
//!   fixed (a mounted volume, a folder that is a symbolic link). The job
//!   tries every path before it fails, so one bad path does not keep the
//!   others on the volume, and the retry is safe because the job is
//!   idempotent.
//!
//! **The file-owner registry.** The job checks a file against the columns
//! that `domain::owned_files::FILE_OWNERS` registers for its folder. Each
//! upload slice adds its column there with one line; [`OwnedFiles`] carries
//! the registry in the shared store, so tests run the job with
//! registrations of their own.
//!
//! The job writes no activity entry: the purge's own `purge` entry records
//! the delete (ADR 0011 §6, "Log"), and ADR 0011 §5 has no action for a
//! file that follows its row.

use loco_rs::prelude::*;
use sea_orm::{DatabaseTransaction, DbErr};
use serde::{Deserialize, Serialize};

use crate::{
    domain::owned_files::{
        FILE_OWNERS, FileOwner, FileOwnerError, OwnedPath, check_file_owners, check_job_paths,
        job_batches,
    },
    models::owned_files::{DeletionReport, delete_unreferenced},
    settings::{MediaSettings, Settings},
};

/// The `DeleteOwnedFiles` worker. Loco registers it in
/// `App::connect_workers` and names its jobs by [`Worker::class_name`].
pub struct Worker {
    /// The application context of the process that runs the job.
    pub ctx: AppContext,
}

/// The arguments of one `DeleteOwnedFiles` job: the paths only.
///
/// Members other than `paths` are ignored, on purpose: Loco 1.2.0 merges a
/// failed job's error into its stored data as an `error` member
/// (`fail_job` in loco-rs 1.2.0 `src/bgworker/pg.rs`), and `jobs retry` runs
/// the job again with that data, so refusing unknown members would make
/// every retried job fail.
#[derive(Deserialize, Debug, Serialize, Clone, PartialEq, Eq)]
pub struct WorkerArgs {
    /// The files to remove when nothing refers to them, as paths below
    /// `media.root` (`authors_images/<name>`, `posts_images/index/<name>`).
    pub paths: Vec<String>,
}

/// What the job needs besides the database: the media settings and the
/// file-owner registry. `App::after_context` stores it in the shared store
/// with [`FILE_OWNERS`]; a test replaces it with its own volume and
/// registrations.
#[derive(Debug, Clone)]
pub struct OwnedFiles {
    /// `settings.media`; only `root` is used.
    pub media: MediaSettings,
    /// The file-owner registry.
    pub owners: &'static [FileOwner],
}

impl OwnedFiles {
    /// The handles of the process: its media settings and [`FILE_OWNERS`].
    ///
    /// - `settings`: the validated settings of the process.
    ///
    /// # Errors
    ///
    /// The [`FileOwnerError`] of a registry that breaks
    /// `check_file_owners`, so the process refuses to start on it.
    pub fn new(settings: &Settings) -> std::result::Result<Self, FileOwnerError> {
        Self::with_owners(settings.media.clone(), FILE_OWNERS)
    }

    /// Handles with a registry of the caller's: a test's registrations.
    ///
    /// - `media`: the media settings of the volume to work on.
    /// - `owners`: the file-owner registry.
    ///
    /// # Errors
    ///
    /// The [`FileOwnerError`] of a registry that breaks
    /// `check_file_owners`.
    pub fn with_owners(
        media: MediaSettings,
        owners: &'static [FileOwner],
    ) -> std::result::Result<Self, FileOwnerError> {
        check_file_owners(owners)?;
        Ok(Self { media, owners })
    }
}

/// The owned-files handles of this app context.
///
/// - `ctx`: the context of the job, or of the request that enqueues it.
///
/// # Errors
///
/// An error that says the context was not built by `App::boot`, whose
/// `after_context` stores them.
pub fn owned_files(ctx: &AppContext) -> Result<OwnedFiles> {
    ctx.shared_store.get::<OwnedFiles>().ok_or_else(|| {
        Error::string(
            "the owned-files handles are missing from the shared store; App::after_context \
             registers them, so this context was not built by App::boot",
        )
    })
}

#[async_trait]
impl BackgroundWorker<WorkerArgs> for Worker {
    /// A worker on `ctx`. Loco calls it when it registers the worker, and
    /// for every job that runs inline (`ForegroundBlocking`).
    ///
    /// - `ctx`: the application context.
    fn build(ctx: &AppContext) -> Self {
        Self { ctx: ctx.clone() }
    }

    /// `DeleteOwnedFiles`: the name of this worker's jobs in the queue
    /// (`jobs dump`, `jobs retry`).
    fn class_name() -> String {
        "DeleteOwnedFiles".to_string()
    }

    /// No tags: every process that runs workers runs this one.
    fn tags() -> Vec<String> {
        Vec::new()
    }

    /// Removes the job's unreferenced files and logs what it did.
    ///
    /// - `args`: the job's paths.
    ///
    /// # Errors
    ///
    /// Before anything is removed: the paths break
    /// `domain::owned_files::check_job_paths` (more than
    /// `MAX_PATHS_PER_JOB`, or a path that is not one file in an entity image
    /// folder), a path's folder has no registered owner, or `media.root`
    /// cannot be opened. After every path was tried: the paths that failed,
    /// with their reasons. The error is the job's outcome in the queue;
    /// `jobs retry` runs it again (module documentation).
    async fn perform(&self, args: WorkerArgs) -> Result<()> {
        let handles = owned_files(&self.ctx)?;
        let paths = check_job_paths(&args.paths).map_err(Error::wrap)?;
        let report = delete_unreferenced(&self.ctx.db, &handles.media, handles.owners, &paths)
            .await
            .map_err(Error::wrap)?;
        tracing::info!(
            deleted = ?report.deleted,
            kept = ?report.kept,
            absent = ?report.absent,
            failed = report.failed.len(),
            "DeleteOwnedFiles: removed the unreferenced files of the job"
        );
        failure(&report).map_or(Ok(()), Err)
    }
}

/// The job's error when a path failed, naming each with its reason.
///
/// - `report`: what the job did.
fn failure(report: &DeletionReport) -> Option<Error> {
    if report.is_complete() {
        return None;
    }
    let reasons: Vec<String> = report
        .failed
        .iter()
        .map(|failed| format!("{}: {}", failed.path, failed.reason))
        .collect();
    Some(Error::Message(format!(
        "DeleteOwnedFiles could not finish {} of its paths; they stay on the volume, \
         unreferenced, until `jobs retry` runs the job again: {}",
        reasons.len(),
        reasons.join("; ")
    )))
}

/// What [`commit_then_enqueue`] did after the commit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AfterCommit {
    /// No paths were given, so no job was needed.
    NothingToDelete,
    /// The jobs are in the queue; under `ForegroundBlocking` (tests) they
    /// have run, and succeeded.
    Enqueued {
        /// How many jobs: one per `MAX_PATHS_PER_JOB` paths.
        jobs: usize,
    },
    /// The jobs could not be enqueued, or, run inline, one failed. The
    /// commit stands; the files nothing refers to any more stay on the
    /// volume as orphans, which ADR 0011 §6 counts as harmless, and the
    /// error is logged with the paths.
    NotEnqueued,
}

/// Commits `txn`, which removed the references to `paths`, and only then
/// enqueues the `DeleteOwnedFiles` jobs for them, so no job can run while
/// the rows that name the files may still be rolled back.
///
/// - `ctx`: the context of the request or task; its queue takes the jobs.
/// - `txn`: the transaction that removed the references, such as a purge
///   (ADR 0011 §6).
/// - `paths`: the files the transaction's rows and revisions named; build
///   each from its stored value with `OwnedPath::from_stored_value` before
///   the commit. Repeats are dropped, and the paths go into jobs of at most
///   `MAX_PATHS_PER_JOB` each, enqueued in one round trip.
///
/// After the commit this never fails: the references are gone for good, so
/// a job that cannot be enqueued only leaves orphans, and the caller's
/// change is reported as done. [`AfterCommit::NotEnqueued`] says so, and the
/// log line names the paths for the operator.
///
/// # Errors
///
/// The commit's error; nothing is enqueued then, and the transaction's
/// changes are not made.
pub async fn commit_then_enqueue(
    ctx: &AppContext,
    txn: DatabaseTransaction,
    paths: &[OwnedPath],
) -> std::result::Result<AfterCommit, DbErr> {
    txn.commit().await?;
    let jobs: Vec<WorkerArgs> = job_batches(paths)
        .into_iter()
        .map(|paths| WorkerArgs { paths })
        .collect();
    if jobs.is_empty() {
        return Ok(AfterCommit::NothingToDelete);
    }
    let count = jobs.len();
    match Worker::perform_all_later(ctx, jobs).await {
        Ok(_) => Ok(AfterCommit::Enqueued { jobs: count }),
        Err(error) => {
            let paths: Vec<&str> = paths.iter().map(OwnedPath::path).collect();
            tracing::error!(
                error = %error,
                paths = ?paths,
                "DeleteOwnedFiles could not be enqueued, or failed inline, after the commit; \
                 the files stay on the volume, unreferenced"
            );
            Ok(AfterCommit::NotEnqueued)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::owned_files::PathFailure;

    #[test]
    fn the_jobs_are_named_delete_owned_files() {
        assert_eq!(Worker::class_name(), "DeleteOwnedFiles");
        assert_eq!(Worker::tags(), Vec::<String>::new());
    }

    #[test]
    fn a_retried_job_reads_its_paths_past_the_error_loco_stored() {
        // Loco merges `{"error": ...}` into a failed job's data, and
        // `jobs retry` runs it with that data.
        let data = serde_json::json!({
            "paths": ["authors_images/a.jpg"],
            "error": "DeleteOwnedFiles could not finish 1 of its paths",
        });
        assert_eq!(
            serde_json::from_value::<WorkerArgs>(data).expect("args"),
            WorkerArgs {
                paths: vec!["authors_images/a.jpg".to_owned()]
            }
        );
        assert_eq!(
            serde_json::to_value(WorkerArgs { paths: Vec::new() }).expect("json"),
            serde_json::json!({ "paths": [] })
        );
    }

    #[test]
    fn a_complete_report_is_no_failure_and_a_failed_path_is_named() {
        let mut report = DeletionReport {
            deleted: vec!["authors_images/a.jpg".to_owned()],
            ..DeletionReport::default()
        };
        assert!(failure(&report).is_none());
        report.failed = vec![
            PathFailure {
                path: "authors_images/b.jpg".to_owned(),
                reason: "cannot remove the file".to_owned(),
            },
            PathFailure {
                path: "authors_images/c.jpg".to_owned(),
                reason: "the path names a directory".to_owned(),
            },
        ];
        let text = failure(&report).expect("a failure").to_string();
        assert!(text.contains("could not finish 2 of its paths"), "{text}");
        assert!(
            text.contains("authors_images/b.jpg: cannot remove the file; authors_images/c.jpg"),
            "{text}"
        );
        assert!(text.contains("jobs retry"), "{text}");
    }

    #[test]
    fn the_handles_refuse_a_registry_that_could_carry_sql() {
        static BAD: [FileOwner; 1] = [FileOwner {
            entity: "authors; --",
            column: "image",
            folder: crate::domain::media_rules::EntityImageFolder::AuthorImages,
        }];
        let media = MediaSettings {
            root: "/data/app/public".into(),
            tmp_dir: "/data/backend3/tmp".into(),
            trash_dir: "/data/backend3/trash".into(),
            min_free_bytes: 0,
            public_base_url: crate::domain::base_url::BaseUrl::parse("http://localhost/storage")
                .expect("a base URL"),
            serve_storage: false,
        };
        assert!(matches!(
            OwnedFiles::with_owners(media.clone(), &BAD),
            Err(FileOwnerError::EntityNotSnakeCase { .. })
        ));
        let handles = OwnedFiles::with_owners(media, FILE_OWNERS).expect("the production registry");
        assert!(std::ptr::eq(handles.owners, FILE_OWNERS));
    }
}
