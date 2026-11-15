//! The `DeleteOwnedFiles` worker as the queue runs it (ADR 0011 §6, ADR 0015
//! §3, ADR 0016 §3).
//!
//! `config/test.yaml` runs jobs in `ForegroundBlocking` mode, so
//! `perform_later` runs the job inline and returns its outcome. Each test
//! puts a media volume of its own and the registrations of the test-only
//! `owned_file_probes` entity (`tests/support/owned_file_probes.rs`) into
//! the context's shared store, where `App::after_context` put the
//! production handles. Two tests switch a copy of the context to
//! `BackgroundQueue` with a Postgres queue on the test's database
//! (`queued_context`), to see the jobs as the production queue stores them,
//! and that a commit that fails leaves none.

use std::{
    collections::BTreeSet,
    os::unix::fs::PermissionsExt as _,
    path::{Path, PathBuf},
    sync::Arc,
};

use backend3::{
    domain::{
        media_rules::EntityImageFolder,
        owned_files::{FILE_OWNERS, FileOwner, MAX_PATHS_PER_JOB, OwnedPath},
    },
    models::{activity_log, owned_files::files_of_record, revisions},
    workers::delete_owned_files::{
        AfterCommit, OwnedFiles, Worker, WorkerArgs, commit_then_enqueue, owned_files,
    },
};
use loco_rs::{
    app::AppContext,
    bgworker::{
        BackgroundWorker, Queue,
        pg::{JobRegistry, PgQueue, RunOpts},
    },
    config::WorkerMode,
};
use sea_orm::{
    ConnectionTrait, DatabaseTransaction, DbBackend, Statement, TransactionTrait, Value,
};

use crate::support::{
    TempMedia, create_queue_table,
    owned_file_probes::{
        self, OWNERS, ProbeRevision, append_revision, insert, new_name, path, stored, update,
    },
    with_app,
};

const AUTHORS: EntityImageFolder = EntityImageFolder::AuthorImages;
const GALLERY: EntityImageFolder = EntityImageFolder::PostImages;
const INDEX: EntityImageFolder = EntityImageFolder::PostIndexImages;

/// A registration whose column name is misspelt: the probe table has no
/// `imgae`.
static MISSPELT: [FileOwner; 1] = [FileOwner {
    entity: owned_file_probes::TABLE,
    column: "imgae",
    folder: EntityImageFolder::AuthorImages,
}];

/// Points the context's `DeleteOwnedFiles` handles at `volume` and `owners`.
fn use_volume(ctx: &AppContext, volume: &TempMedia, owners: &'static [FileOwner]) {
    ctx.shared_store.insert(
        OwnedFiles::with_owners(volume.settings().clone(), owners).expect("valid registrations"),
    );
}

/// Writes an invented file at `path` below the volume's root, with its
/// folders.
fn write_file(volume: &TempMedia, path: &OwnedPath) {
    let file = volume.settings().root.join(path.path());
    std::fs::create_dir_all(file.parent().expect("a folder")).expect("create the folder");
    std::fs::write(&file, b"invented image bytes").expect("write the file");
}

/// The files below the volume's root, as paths relative to it.
fn files(volume: &TempMedia) -> BTreeSet<PathBuf> {
    TempMedia::files(&volume.settings().root)
        .into_iter()
        .collect()
}

/// The same set, from owned paths.
fn set(paths: &[&OwnedPath]) -> BTreeSet<PathBuf> {
    paths
        .iter()
        .map(|path| PathBuf::from(path.path()))
        .collect()
}

/// Runs one job for `paths`, inline.
async fn run(ctx: &AppContext, paths: &[OwnedPath]) -> loco_rs::Result<String> {
    Worker::perform_later(
        ctx,
        WorkerArgs {
            paths: paths.iter().map(|path| path.path().to_owned()).collect(),
        },
    )
    .await
}

/// A copy of `ctx` in production's mode, `BackgroundQueue`, with a Postgres
/// queue on the test's own pool, which `with_app` closes. No worker runs
/// its jobs, so they stay in `pg_loco_queue` as a request leaves them.
///
/// - `ctx`: the test's context, whose database holds the queue table
///   (`create_queue_table`).
fn queued_context(ctx: &AppContext) -> AppContext {
    let mut queued = ctx.clone();
    queued.config.workers.mode = WorkerMode::BackgroundQueue;
    #[allow(
        clippy::default_trait_access,
        reason = "the token is tokio-util's CancellationToken, a crate backend3 does not \
                  depend on, so its type is not named here"
    )]
    let queue = PgQueue {
        pool: ctx.db.get_postgres_connection_pool().clone(),
        registry: Arc::new(tokio::sync::Mutex::new(JobRegistry::new())),
        run_opts: RunOpts {
            num_workers: 1,
            poll_interval_sec: 5,
            reaper: None,
        },
        token: Default::default(),
    };
    queued.queue_provider = Some(Arc::new(Queue::from_provider(Arc::new(queue))));
    queued
}

/// The jobs in `pg_loco_queue`, as `(name, status, task_data)`, the one
/// with the most paths first.
///
/// - `ctx`: the test's context.
async fn queued_jobs(ctx: &AppContext) -> Vec<(String, String, serde_json::Value)> {
    let rows = ctx
        .db
        .query_all_raw(Statement::from_string(
            DbBackend::Postgres,
            "SELECT name, status, task_data FROM pg_loco_queue \
             ORDER BY jsonb_array_length(task_data -> 'paths') DESC"
                .to_owned(),
        ))
        .await
        .expect("the queue");
    rows.iter()
        .map(|row| {
            (
                row.try_get("", "name").expect("name"),
                row.try_get("", "status").expect("status"),
                row.try_get("", "task_data").expect("task_data"),
            )
        })
        .collect()
}

/// A transaction that deletes probe `id` and then fails at its commit: a
/// deferred unique constraint is checked only there.
///
/// - `ctx`: the test's context.
/// - `id`: the probe whose row the transaction deletes.
async fn purge_that_fails_at_commit(ctx: &AppContext, id: i64) -> DatabaseTransaction {
    ctx.db
        .execute_unprepared(
            "CREATE TABLE deferred_probes (n integer UNIQUE DEFERRABLE INITIALLY DEFERRED)",
        )
        .await
        .expect("create");
    let txn = ctx.db.begin().await.expect("begin");
    txn.execute_unprepared(&format!("DELETE FROM owned_file_probes WHERE id = {id}"))
        .await
        .expect("delete the row");
    txn.execute_unprepared("INSERT INTO deferred_probes VALUES (1), (1)")
        .await
        .expect("deferred");
    txn
}

/// How many probe rows have `id`.
///
/// - `ctx`: the test's context.
/// - `id`: the probe.
async fn probe_rows(ctx: &AppContext, id: i64) -> i64 {
    let row = ctx
        .db
        .query_one_raw(Statement::from_string(
            DbBackend::Postgres,
            format!("SELECT count(*) FROM owned_file_probes WHERE id = {id}"),
        ))
        .await
        .expect("count")
        .expect("one row");
    row.try_get_by_index::<i64>(0).expect("a count")
}

#[tokio::test]
async fn the_job_removes_exactly_the_unreferenced_files_and_a_second_run_nothing_more() {
    with_app(|_router, ctx| async move {
        owned_file_probes::create_table(&ctx.db).await;
        let volume = TempMedia::new();
        use_volume(&ctx, &volume, &OWNERS);

        let live = path(AUTHORS, &new_name());
        let trashed = path(AUTHORS, &new_name());
        let replaced = path(AUTHORS, &new_name());
        let gallery = path(GALLERY, &new_name());
        let gallery_history = path(GALLERY, &new_name());
        let index = path(INDEX, &new_name());
        let unreferenced = [
            path(AUTHORS, &new_name()),
            path(GALLERY, &new_name()),
            path(INDEX, &new_name()),
        ];
        let never_written = path(AUTHORS, &new_name());
        let not_in_the_job = path(AUTHORS, &new_name());

        // A live row with all three columns; it once had other files,
        // which only its revision still shows.
        let id = insert(
            &ctx.db,
            &ProbeRevision {
                image: Some(stored(AUTHORS, live.name())),
                index_image: Some(index.stored_value().to_owned()),
                images: vec![stored(GALLERY, gallery.name())],
            },
            false,
        )
        .await;
        append_revision(
            &ctx.db,
            id,
            &ProbeRevision {
                image: Some(stored(AUTHORS, replaced.name())),
                images: vec![stored(GALLERY, gallery_history.name())],
                ..ProbeRevision::default()
            },
        )
        .await;
        insert(
            &ctx.db,
            &ProbeRevision {
                image: Some(stored(AUTHORS, trashed.name())),
                ..ProbeRevision::default()
            },
            true,
        )
        .await;
        let referenced = [
            &live,
            &trashed,
            &replaced,
            &gallery,
            &gallery_history,
            &index,
        ];
        for file in referenced
            .iter()
            .copied()
            .chain(&unreferenced)
            .chain([&not_in_the_job])
        {
            write_file(&volume, file);
        }
        let mut job: Vec<OwnedPath> = referenced.iter().map(|path| (*path).clone()).collect();
        job.extend(unreferenced.iter().cloned());
        job.push(never_written.clone());

        let mut expected = set(&referenced);
        expected.insert(PathBuf::from(not_in_the_job.path()));
        run(&ctx, &job).await.expect("the job succeeds");
        assert_eq!(files(&volume), expected);

        // Idempotent (ADR 0016 §3): the same job again removes nothing more
        // and does not fail on the files the first run removed.
        run(&ctx, &job).await.expect("the second run succeeds");
        assert_eq!(files(&volume), expected);
    })
    .await;
}

#[tokio::test]
async fn a_replaced_image_stays_until_its_record_is_purged() {
    with_app(|_router, ctx| async move {
        owned_file_probes::create_table(&ctx.db).await;
        let volume = TempMedia::new();
        use_volume(&ctx, &volume, &OWNERS);
        let first = path(AUTHORS, &new_name());
        let second = path(AUTHORS, &new_name());
        write_file(&volume, &first);
        write_file(&volume, &second);

        // Saved with the first image, then with the second (parity SHR-15):
        // the first is no longer in the row, but revision 1 shows it, and a
        // revert would bring it back.
        let saved = |name: &str| ProbeRevision {
            image: Some(stored(AUTHORS, name)),
            ..ProbeRevision::default()
        };
        let id = insert(&ctx.db, &saved(first.name()), false).await;
        append_revision(&ctx.db, id, &saved(first.name())).await;
        update(&ctx.db, id, &saved(second.name())).await;
        append_revision(&ctx.db, id, &saved(second.name())).await;
        run(&ctx, std::slice::from_ref(&first))
            .await
            .expect("the job succeeds");
        assert_eq!(files(&volume), set(&[&first, &second]));

        // The purge (ADR 0011 §6): the files the row and its revisions name,
        // then the row and its revisions in one transaction, then the job.
        let txn = ctx.db.begin().await.expect("begin");
        let owners = owned_files(&ctx).expect("the handles").owners;
        let named = files_of_record::<ProbeRevision>(&txn, owners, id)
            .await
            .expect("the files");
        assert_eq!(named, vec![second.clone(), first.clone()]);
        activity_log::with_permanent_delete_flag(&txn)
            .await
            .expect("the permanent-delete flag");
        revisions::delete_for_record::<ProbeRevision>(&txn, id)
            .await
            .expect("delete the revisions");
        txn.execute_raw(Statement::from_sql_and_values(
            DbBackend::Postgres,
            "DELETE FROM owned_file_probes WHERE id = $1",
            [Value::from(id)],
        ))
        .await
        .expect("delete the row");
        let outcome = commit_then_enqueue(&ctx, txn, &named)
            .await
            .expect("the commit");
        assert_eq!(outcome, AfterCommit::Enqueued { jobs: 1 });
        assert_eq!(files(&volume), BTreeSet::new());
    })
    .await;
}

#[tokio::test]
async fn a_job_with_a_path_outside_the_entity_folders_removes_nothing() {
    with_app(|_router, ctx| async move {
        owned_file_probes::create_table(&ctx.db).await;
        let volume = TempMedia::new();
        use_volume(&ctx, &volume, &OWNERS);
        let file = path(AUTHORS, &new_name());
        write_file(&volume, &file);
        let library = volume.settings().root.join("uploads/docs/a.pdf");
        std::fs::create_dir_all(library.parent().expect("a folder")).expect("create");
        std::fs::write(&library, b"invented").expect("write");

        for bad in [
            "uploads/docs/a.pdf",
            "authors_images/../uploads/docs/a.pdf",
            "/etc/hostname",
        ] {
            let error = Worker::perform_later(
                &ctx,
                WorkerArgs {
                    paths: vec![file.path().to_owned(), bad.to_owned()],
                },
            )
            .await
            .expect_err(bad);
            assert!(error.to_string().contains("path 1"), "{bad}: {error}");
        }
        let error = Worker::perform_later(
            &ctx,
            WorkerArgs {
                paths: vec![file.path().to_owned(); MAX_PATHS_PER_JOB + 1],
            },
        )
        .await
        .expect_err("too many paths");
        assert!(error.to_string().contains("at most"), "{error}");
        assert_eq!(
            files(&volume),
            BTreeSet::from([
                PathBuf::from(file.path()),
                PathBuf::from("uploads/docs/a.pdf")
            ])
        );
    })
    .await;
}

#[tokio::test]
async fn a_folder_without_a_registered_owner_fails_the_job() {
    with_app(|_router, ctx| async move {
        owned_file_probes::create_table(&ctx.db).await;
        let volume = TempMedia::new();
        let author = path(AUTHORS, &new_name());
        let colleague = path(EntityImageFolder::ColleagueImages, &new_name());
        write_file(&volume, &author);
        write_file(&volume, &colleague);

        // The probe registers no colleague column, so nothing could tell
        // whether a colleague still shows the file.
        use_volume(&ctx, &volume, &OWNERS);
        let error = run(&ctx, &[author.clone(), colleague.clone()])
            .await
            .expect_err("no owner");
        assert!(error.to_string().contains("FILE_OWNERS"), "{error}");
        // The production registry has no owner yet, so it refuses even the
        // probe's folders.
        use_volume(&ctx, &volume, FILE_OWNERS);
        run(&ctx, std::slice::from_ref(&author))
            .await
            .expect_err("no owner yet");
        assert_eq!(files(&volume), set(&[&author, &colleague]));
    })
    .await;
}

#[tokio::test]
async fn a_missing_media_root_fails_the_job_and_a_retry_after_mounting_removes_the_file() {
    with_app(|_router, ctx| async move {
        owned_file_probes::create_table(&ctx.db).await;
        let volume = TempMedia::new();
        use_volume(&ctx, &volume, &OWNERS);
        let file = path(AUTHORS, &new_name());
        let root = volume.settings().root.clone();
        let unmounted = volume.base().join("unmounted");
        std::fs::rename(&root, &unmounted).expect("unmount the root");

        let error = run(&ctx, std::slice::from_ref(&file))
            .await
            .expect_err("no root");
        assert!(error.to_string().contains("settings.media.root"), "{error}");

        // `jobs retry` runs the same job once the volume is back.
        std::fs::rename(&unmounted, &root).expect("mount the root");
        write_file(&volume, &file);
        run(&ctx, std::slice::from_ref(&file))
            .await
            .expect("the retry succeeds");
        assert_eq!(files(&volume), BTreeSet::new());
    })
    .await;
}

#[tokio::test]
async fn a_failing_path_fails_the_job_after_the_other_files_are_removed() {
    with_app(|_router, ctx| async move {
        owned_file_probes::create_table(&ctx.db).await;
        let volume = TempMedia::new();
        use_volume(&ctx, &volume, &OWNERS);
        let file = path(AUTHORS, &new_name());
        write_file(&volume, &file);
        // A directory where the job expects a file is never removed.
        let directory = path(AUTHORS, &new_name());
        let inside = volume
            .settings()
            .root
            .join(directory.path())
            .join("kept.txt");
        std::fs::create_dir_all(inside.parent().expect("a folder")).expect("create");
        std::fs::write(&inside, b"invented").expect("write");

        let error = run(&ctx, &[directory.clone(), file.clone()])
            .await
            .expect_err("a directory");
        let text = error.to_string();
        assert!(text.contains(directory.path()), "{text}");
        assert!(text.contains("names a directory"), "{text}");
        assert!(text.contains("jobs retry"), "{text}");
        assert!(!text.contains(file.path()), "{text}");
        let kept = PathBuf::from(directory.path()).join("kept.txt");
        assert_eq!(files(&volume), BTreeSet::from([kept.clone()]));

        // A retry still fails on the directory, and only on it.
        let error = run(&ctx, &[directory.clone(), file.clone()])
            .await
            .expect_err("still a directory");
        assert!(
            error
                .to_string()
                .contains("could not finish 1 of its paths"),
            "{error}"
        );
        assert_eq!(files(&volume), BTreeSet::from([kept]));
    })
    .await;
}

#[tokio::test]
async fn a_folder_that_is_a_symbolic_link_is_never_followed() {
    with_app(|_router, ctx| async move {
        owned_file_probes::create_table(&ctx.db).await;
        let volume = TempMedia::new();
        use_volume(&ctx, &volume, &OWNERS);
        let file = path(AUTHORS, &new_name());
        let elsewhere = volume.base().join("elsewhere");
        std::fs::create_dir_all(&elsewhere).expect("create");
        let target = elsewhere.join(file.name());
        std::fs::write(&target, b"invented").expect("write");
        std::os::unix::fs::symlink(&elsewhere, volume.settings().root.join("authors_images"))
            .expect("link the folder");

        let error = run(&ctx, std::slice::from_ref(&file))
            .await
            .expect_err("a linked folder");
        assert!(error.to_string().contains("symbolic link"), "{error}");
        assert!(target.exists(), "the file behind the link stays");
    })
    .await;
}

#[tokio::test]
async fn a_registered_column_that_the_schema_lacks_fails_the_job_and_keeps_the_file() {
    with_app(|_router, ctx| async move {
        owned_file_probes::create_table(&ctx.db).await;
        let volume = TempMedia::new();
        use_volume(&ctx, &volume, &MISSPELT);
        let file = path(AUTHORS, &new_name());
        write_file(&volume, &file);

        let error = run(&ctx, std::slice::from_ref(&file))
            .await
            .expect_err("no such column");
        assert!(error.to_string().contains("refers to the file"), "{error}");
        assert_eq!(files(&volume), set(&[&file]));
    })
    .await;
}

#[tokio::test]
async fn no_job_runs_when_the_commit_fails() {
    with_app(|_router, ctx| async move {
        owned_file_probes::create_table(&ctx.db).await;
        let volume = TempMedia::new();
        use_volume(&ctx, &volume, &OWNERS);
        // The row names `named`; no row or revision names `orphan`, so a
        // job that ran, before the commit or after a failed one, would
        // remove `orphan` at once.
        let named = path(AUTHORS, &new_name());
        let orphan = path(AUTHORS, &new_name());
        write_file(&volume, &named);
        write_file(&volume, &orphan);
        let id = insert(
            &ctx.db,
            &ProbeRevision {
                image: Some(stored(AUTHORS, named.name())),
                ..ProbeRevision::default()
            },
            false,
        )
        .await;
        let txn = purge_that_fails_at_commit(&ctx, id).await;

        commit_then_enqueue(&ctx, txn, &[named.clone(), orphan.clone()])
            .await
            .expect_err("the commit fails");
        // The delete rolled back, and no job ran: both files are there.
        assert_eq!(probe_rows(&ctx, id).await, 1);
        assert_eq!(files(&volume), set(&[&named, &orphan]));
    })
    .await;
}

#[tokio::test]
async fn no_job_is_queued_when_the_commit_fails() {
    with_app(|_router, ctx| async move {
        owned_file_probes::create_table(&ctx.db).await;
        create_queue_table(&ctx).await;
        let volume = TempMedia::new();
        use_volume(&ctx, &volume, &OWNERS);
        let queued = queued_context(&ctx);
        let file = path(AUTHORS, &new_name());
        let id = insert(
            &ctx.db,
            &ProbeRevision {
                image: Some(stored(AUTHORS, file.name())),
                ..ProbeRevision::default()
            },
            false,
        )
        .await;
        let txn = purge_that_fails_at_commit(&ctx, id).await;

        commit_then_enqueue(&queued, txn, std::slice::from_ref(&file))
            .await
            .expect_err("the commit fails");
        // The queue's pool commits each job on its own, so a job enqueued
        // before the commit would be here now.
        assert_eq!(queued_jobs(&ctx).await, Vec::new());
        assert_eq!(probe_rows(&ctx, id).await, 1);
    })
    .await;
}

#[tokio::test]
async fn a_folder_the_process_may_not_change_fails_the_job_with_the_systems_reason() {
    // The mode of a folder binds every account but root, so as root there
    // is nothing to see.
    if rustix::process::geteuid().is_root() {
        return;
    }
    with_app(|_router, ctx| async move {
        owned_file_probes::create_table(&ctx.db).await;
        let volume = TempMedia::new();
        use_volume(&ctx, &volume, &OWNERS);
        let file = path(AUTHORS, &new_name());
        write_file(&volume, &file);
        let folder = volume.settings().root.join("authors_images");
        let set_mode = |mode| {
            std::fs::set_permissions(&folder, std::fs::Permissions::from_mode(mode))
                .expect("set the folder's mode");
        };

        // Readable, not writable: the file cannot be unlinked.
        set_mode(0o555);
        let unwritable = run(&ctx, std::slice::from_ref(&file)).await;
        // Not even readable: the folder cannot be opened.
        set_mode(0o000);
        let unreadable = run(&ctx, std::slice::from_ref(&file)).await;
        // Back to normal before the asserts, so the volume can be removed.
        set_mode(0o755);

        let text = unwritable.expect_err("a read-only folder").to_string();
        assert!(
            text.contains(&format!("{}: cannot remove the file", file.path())),
            "{text}"
        );
        assert!(text.contains("Permission denied"), "{text}");
        assert!(text.contains("may not write"), "{text}");
        let text = unreadable.expect_err("a closed folder").to_string();
        assert!(
            text.contains("cannot open the entity image folder authors_images/"),
            "{text}"
        );
        assert!(text.contains("Permission denied"), "{text}");
        assert_eq!(files(&volume), set(&[&file]));

        // `jobs retry` once the mode is fixed removes it.
        run(&ctx, std::slice::from_ref(&file))
            .await
            .expect("the retry succeeds");
        assert_eq!(files(&volume), BTreeSet::new());
    })
    .await;
}

#[tokio::test]
async fn a_job_that_fails_after_the_commit_leaves_the_commit_and_an_orphan() {
    with_app(|_router, ctx| async move {
        owned_file_probes::create_table(&ctx.db).await;
        let volume = TempMedia::new();
        use_volume(&ctx, &volume, &OWNERS);
        let file = path(AUTHORS, &new_name());
        write_file(&volume, &file);
        let id = insert(
            &ctx.db,
            &ProbeRevision {
                image: Some(stored(AUTHORS, file.name())),
                ..ProbeRevision::default()
            },
            false,
        )
        .await;
        // The job will find no root (the volume is not mounted).
        let unmounted = volume.base().join("unmounted");
        std::fs::rename(&volume.settings().root, &unmounted).expect("unmount");

        let txn = ctx.db.begin().await.expect("begin");
        txn.execute_unprepared(&format!("DELETE FROM owned_file_probes WHERE id = {id}"))
            .await
            .expect("delete the row");
        let outcome = commit_then_enqueue(&ctx, txn, std::slice::from_ref(&file))
            .await
            .expect("the commit stands");
        assert_eq!(outcome, AfterCommit::NotEnqueued);
        // ADR 0011 §6: the file is an orphan, never a reference to a
        // missing row; the row is gone and the file is still there.
        let row = ctx
            .db
            .query_one_raw(Statement::from_string(
                DbBackend::Postgres,
                "SELECT count(*) FROM owned_file_probes".to_owned(),
            ))
            .await
            .expect("count")
            .expect("one row");
        assert_eq!(row.try_get_by_index::<i64>(0).expect("a count"), 0);
        assert!(unmounted.join(file.path()).exists());
    })
    .await;
}

#[tokio::test]
async fn no_paths_make_no_job_and_the_commit_still_happens() {
    with_app(|_router, ctx| async move {
        owned_file_probes::create_table(&ctx.db).await;
        let id = insert(&ctx.db, &ProbeRevision::default(), false).await;
        let txn = ctx.db.begin().await.expect("begin");
        txn.execute_unprepared(&format!("DELETE FROM owned_file_probes WHERE id = {id}"))
            .await
            .expect("delete the row");
        // No handles are needed when there is nothing to delete.
        ctx.shared_store
            .remove::<OwnedFiles>()
            .expect("the production handles");
        let outcome = commit_then_enqueue(&ctx, txn, &[])
            .await
            .expect("the commit");
        assert_eq!(outcome, AfterCommit::NothingToDelete);
        let row = ctx
            .db
            .query_one_raw(Statement::from_string(
                DbBackend::Postgres,
                "SELECT count(*) FROM owned_file_probes".to_owned(),
            ))
            .await
            .expect("count")
            .expect("one row");
        assert_eq!(row.try_get_by_index::<i64>(0).expect("a count"), 0);
    })
    .await;
}

#[tokio::test]
async fn the_booted_app_carries_the_production_registry_and_media_settings() {
    with_app(|_router, ctx| async move {
        let handles = owned_files(&ctx).expect("the handles");
        assert!(std::ptr::eq(handles.owners, FILE_OWNERS));
        let settings = backend3::settings::Settings::from_section(ctx.config.settings.as_ref())
            .expect("the settings");
        assert_eq!(handles.media, settings.media);
        ctx.shared_store
            .remove::<OwnedFiles>()
            .expect("remove them");
        let error = owned_files(&ctx).expect_err("missing");
        assert!(error.to_string().contains("App::after_context"), "{error}");
    })
    .await;
}

#[tokio::test]
async fn in_the_queue_a_job_carries_only_paths_and_waits_for_the_worker() {
    with_app(|_router, ctx| async move {
        owned_file_probes::create_table(&ctx.db).await;
        create_queue_table(&ctx).await;
        let volume = TempMedia::new();
        use_volume(&ctx, &volume, &OWNERS);
        let queued = queued_context(&ctx);

        let paths: Vec<OwnedPath> = (0..=MAX_PATHS_PER_JOB)
            .map(|_| path(AUTHORS, &new_name()))
            .collect();
        write_file(&volume, &paths[0]);
        let txn = ctx.db.begin().await.expect("begin");
        let outcome = commit_then_enqueue(&queued, txn, &paths)
            .await
            .expect("the commit");
        assert_eq!(outcome, AfterCommit::Enqueued { jobs: 2 });

        let jobs = queued_jobs(&ctx).await;
        assert_eq!(jobs.len(), 2);
        let mut queued_paths = Vec::new();
        for (name, status, data) in &jobs {
            assert_eq!(name, "DeleteOwnedFiles");
            assert_eq!(status, "queued");
            let args: WorkerArgs = serde_json::from_value(data.clone()).expect("the args");
            assert_eq!(
                data.as_object().map(serde_json::Map::len),
                Some(1),
                "paths only: {data}"
            );
            queued_paths.extend(args.paths);
        }
        assert_eq!(
            jobs[0].2["paths"].as_array().map(Vec::len),
            Some(MAX_PATHS_PER_JOB)
        );
        let expected: Vec<String> = paths.iter().map(|path| path.path().to_owned()).collect();
        assert_eq!(queued_paths, expected);
        // Nothing ran: the file waits for the worker.
        assert!(volume.settings().root.join(paths[0].path()).exists());
        assert!(Path::new(&volume.settings().root).is_dir());
    })
    .await;
}
