//! The `revisions` model (ADR 0011 §4, §6, §7): `append` writes every part
//! of a revision, numbers each record's revisions 1, 2, 3, ... also when
//! saves of one record run at the same time (and fails instead of repeating
//! a number under REPEATABLE READ), and refuses a revision that breaks a
//! rule before it writes anything; `record_save` writes a save's revision
//! and its activity entry together, with the changed field names read
//! across an upcast, and fails when the latest revision cannot be upcast; a
//! failed entry rolls the save back; `delete_for_record` removes exactly one
//! record's revisions in a permanent delete, including those of a save that
//! was still running; `App::truncate` leaves the table alone; the table's
//! CHECK allows exactly the reasons of `src/domain/revision.rs`.
//!
//! The records are rows of the test-only `probe_notes` table
//! (`tests/support/probe_notes.rs`). The SQL rules of the triggers are
//! tested in `tests/migration/revisions.rs`; every account is invented.

use std::{panic::AssertUnwindSafe, time::Duration};

use backend3::{
    app::App,
    domain::{
        activity::{Action, Actor},
        revision::{
            Reason, RevisionError, RevisionSnapshot, SnapshotError, SnapshotTypeError, UpcastError,
            UpcastStepError,
        },
    },
    models::{
        activity_log,
        revisions::{
            self, MissingRecord, Model, ReadSnapshotError, Save, append, delete_for_record,
            record_save,
        },
    },
};
use futures_util::{FutureExt, future::join_all};
use loco_rs::{app::Hooks, model::ModelError};
use sea_orm::{
    ConnectOptions, ConnectionTrait, Database, DatabaseConnection, DatabaseTransaction, DbBackend,
    EntityTrait, IsolationLevel, Statement, TransactionTrait,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::support::{
    probe_notes::{self, ProbeNoteRevision, first_note, insert_row, next_note, update_row},
    with_app,
};

/// The one value that `sql` returns.
async fn value<C, T>(db: &C, sql: &str) -> T
where
    C: ConnectionTrait,
    T: sea_orm::TryGetable,
{
    db.query_one_raw(Statement::from_string(DbBackend::Postgres, sql))
        .await
        .expect(sql)
        .expect("one row")
        .try_get_by_index(0)
        .expect(sql)
}

/// Inserts an invented account and returns its id.
async fn insert_user<C: ConnectionTrait>(db: &C) -> i64 {
    value(
        db,
        "INSERT INTO users (pid, name, email) \
         VALUES (gen_random_uuid(), 'Próba', 'proba@example.test') RETURNING id",
    )
    .await
}

/// Inserts a committed probe note with no revision and returns its id.
async fn committed_note(db: &DatabaseConnection) -> i64 {
    let txn = db.begin().await.expect("begin");
    let id = insert_row(&txn, Actor::System, &first_note()).await;
    txn.commit().await.expect("commit");
    id
}

async fn revision_count(db: &DatabaseConnection) -> i64 {
    value(db, "SELECT count(*) FROM revisions").await
}

async fn entry_count(db: &DatabaseConnection) -> i64 {
    value(db, "SELECT count(*) FROM activity_log").await
}

/// The error of type `E` inside a refusal of the model.
fn inner<E: std::error::Error + 'static>(error: &ModelError) -> &E {
    match error {
        ModelError::Any(inner) => inner
            .downcast_ref::<E>()
            .unwrap_or_else(|| panic!("not a {}: {inner}", std::any::type_name::<E>())),
        other => panic!("not ModelError::Any: {other:?}"),
    }
}

/// The numbers of the revisions of note `id`, oldest first.
async fn numbers(db: &DatabaseConnection, id: i64) -> Vec<i32> {
    Model::for_record(db, "probe_notes", id)
        .await
        .expect("revisions")
        .iter()
        .map(|revision| revision.revision_no)
        .collect()
}

/// Appends an `update` revision of note `id` in a transaction of its own.
async fn append_committed(db: &DatabaseConnection, id: i64) -> Model {
    let txn = db.begin().await.expect("begin");
    let revision = append(&txn, Actor::System, id, Reason::Update, &first_note(), None)
        .await
        .expect("append");
    txn.commit().await.expect("commit");
    revision
}

#[tokio::test]
async fn append_stores_every_part_of_a_revision() {
    with_app(|_router, ctx| async move {
        let db = &ctx.db;
        probe_notes::create_table(db).await;
        let user = insert_user(db).await;
        let txn = db.begin().await.expect("begin");
        let start: String = value(&txn, "SELECT now()::text").await;
        let note = first_note();
        let id = insert_row(&txn, Actor::User(user), &note).await;
        let revision = append(
            &txn,
            Actor::User(user),
            id,
            Reason::Create,
            &note,
            Some("4bf92f35-request"),
        )
        .await
        .expect("append");
        txn.commit().await.expect("commit");

        let stored = revisions::Entity::find_by_id(revision.id)
            .one(db)
            .await
            .expect("find")
            .expect("stored");
        assert_eq!(stored, revision);
        assert_eq!(revision.entity_type, "probe_notes");
        assert_eq!(revision.entity_id, id);
        assert_eq!(revision.revision_no, 1);
        assert_eq!(revision.reason(), Ok(Reason::Create));
        assert_eq!(revision.schema_version, ProbeNoteRevision::SCHEMA_VERSION);
        assert_eq!(
            revision.snapshot,
            json!({"title": "Próba jegyzet", "body": "<p>Első változat</p>", "color": "#336699"})
        );
        assert_eq!(revision.created_by, Some(user));
        assert_eq!(revision.request_id.as_deref(), Some("4bf92f35-request"));
        // PostgreSQL's clock: the start of the writing transaction
        // (research: DECISIONS R15, D90).
        let created_at: String = value(
            db,
            &format!(
                "SELECT created_at::text FROM revisions WHERE id = {}",
                revision.id
            ),
        )
        .await;
        assert_eq!(created_at, start);
        assert_eq!(revision.read_snapshot::<ProbeNoteRevision>(), Ok(note));
    })
    .await;
}

#[tokio::test]
async fn each_record_counts_its_own_revisions_from_one() {
    with_app(|_router, ctx| async move {
        let db = &ctx.db;
        probe_notes::create_table(db).await;
        let first = committed_note(db).await;
        let second = committed_note(db).await;
        for id in [first, second, first, first, second] {
            append_committed(db, id).await;
        }
        assert_eq!(numbers(db, first).await, [1, 2, 3]);
        assert_eq!(numbers(db, second).await, [1, 2]);
        // Several revisions of one record in one transaction count on too.
        let txn = db.begin().await.expect("begin");
        for _ in 0..2 {
            append(
                &txn,
                Actor::System,
                second,
                Reason::Update,
                &first_note(),
                None,
            )
            .await
            .expect("append");
        }
        txn.commit().await.expect("commit");
        assert_eq!(numbers(db, second).await, [1, 2, 3, 4]);
    })
    .await;
}

#[tokio::test]
async fn the_importer_writes_import_original_then_import_without_a_user() {
    with_app(|_router, ctx| async move {
        let db = &ctx.db;
        probe_notes::create_table(db).await;
        let txn = db.begin().await.expect("begin");
        let original = ProbeNoteRevision {
            body: Some("<P>Régi&nbsp;törzs</P>".to_owned()),
            ..first_note()
        };
        let id = insert_row(&txn, Actor::Import, &first_note()).await;
        let first = append(
            &txn,
            Actor::Import,
            id,
            Reason::ImportOriginal,
            &original,
            None,
        )
        .await
        .expect("import_original");
        let second = append(&txn, Actor::Import, id, Reason::Import, &first_note(), None)
            .await
            .expect("import");
        txn.commit().await.expect("commit");
        assert_eq!(
            (first.revision_no, first.reason(), first.created_by),
            (1, Ok(Reason::ImportOriginal), None)
        );
        assert_eq!(
            (second.revision_no, second.reason(), second.created_by),
            (2, Ok(Reason::Import), None)
        );
        assert_eq!(first.read_snapshot::<ProbeNoteRevision>(), Ok(original));
        // Saves follow the import as updates.
        let third = append_committed(db, id).await;
        assert_eq!(third.revision_no, 3);
    })
    .await;
}

#[tokio::test]
async fn append_refuses_a_revision_that_breaks_a_rule_before_writing_it() {
    with_app(|_router, ctx| async move {
        let db = &ctx.db;
        probe_notes::create_table(db).await;
        let id = committed_note(db).await;
        append_committed(db, id).await;
        let before = revision_count(db).await;
        let cases = [
            (
                Actor::System,
                Reason::Create,
                None,
                RevisionError::NotFirst {
                    reason: Reason::Create,
                    previous: Reason::Update,
                },
            ),
            (
                Actor::Import,
                Reason::Import,
                None,
                RevisionError::ImportAfter {
                    previous: Reason::Update,
                },
            ),
            (
                Actor::Import,
                Reason::ImportOriginal,
                None,
                RevisionError::NotFirst {
                    reason: Reason::ImportOriginal,
                    previous: Reason::Update,
                },
            ),
            (
                Actor::System,
                Reason::Import,
                None,
                RevisionError::ImportReasonNeedsImporter {
                    reason: Reason::Import,
                    actor: Actor::System,
                },
            ),
            (
                Actor::Import,
                Reason::Update,
                None,
                RevisionError::ImporterWritesOnlyImport {
                    reason: Reason::Update,
                },
            ),
            (
                Actor::System,
                Reason::Update,
                Some(""),
                RevisionError::RequestIdLength { chars: 0 },
            ),
        ];
        for (actor, reason, request_id, expected) in cases {
            let txn = db.begin().await.expect("begin");
            let error = append(&txn, actor, id, reason, &first_note(), request_id)
                .await
                .expect_err("refused");
            assert_eq!(inner::<RevisionError>(&error), &expected);
            txn.rollback().await.expect("rollback");
        }
        assert_eq!(revision_count(db).await, before);
    })
    .await;
}

/// A snapshot type whose allow-list leaves out a field the struct has.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct UnlistedField {
    title: String,
    secret_note: String,
}

impl RevisionSnapshot for UnlistedField {
    const ENTITY_TYPE: &'static str = "probe_notes";
    const SCHEMA_VERSION: i32 = 1;
    const FIELDS: &'static [&'static str] = &["title"];
}

/// A snapshot type at version 2 without the step from version 1.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct MissingStep {
    title: String,
}

impl RevisionSnapshot for MissingStep {
    const ENTITY_TYPE: &'static str = "probe_notes";
    const SCHEMA_VERSION: i32 = 2;
    const FIELDS: &'static [&'static str] = &["title"];
}

/// A snapshot type whose table does not exist.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct NoTable {
    title: String,
}

impl RevisionSnapshot for NoTable {
    const ENTITY_TYPE: &'static str = "no_such_table";
    const SCHEMA_VERSION: i32 = 1;
    const FIELDS: &'static [&'static str] = &["title"];
}

#[tokio::test]
async fn append_refuses_a_snapshot_or_a_record_it_cannot_store() {
    with_app(|_router, ctx| async move {
        let db = &ctx.db;
        probe_notes::create_table(db).await;
        let id = committed_note(db).await;
        let txn = db.begin().await.expect("begin");

        let unlisted = UnlistedField {
            title: "x".to_owned(),
            secret_note: "y".to_owned(),
        };
        let error = append(&txn, Actor::System, id, Reason::Update, &unlisted, None)
            .await
            .expect_err("unlisted");
        assert_eq!(
            inner::<SnapshotError>(&error),
            &SnapshotError::UnlistedField {
                entity_type: "probe_notes",
                field: "secret_note".to_owned()
            }
        );
        let error = append(
            &txn,
            Actor::System,
            id,
            Reason::Update,
            &MissingStep {
                title: "x".to_owned(),
            },
            None,
        )
        .await
        .expect_err("registry gap");
        assert_eq!(
            inner::<SnapshotError>(&error),
            &SnapshotError::Type(SnapshotTypeError::UpcastCount {
                entity_type: "probe_notes",
                version: 2,
                expected: 1,
                steps: 0
            })
        );
        // A record that does not exist, or that another transaction has not
        // committed yet, has no revisions.
        let error = append(
            &txn,
            Actor::System,
            id + 1000,
            Reason::Create,
            &first_note(),
            None,
        )
        .await
        .expect_err("no record");
        assert_eq!(
            inner::<MissingRecord>(&error),
            &MissingRecord {
                entity_type: "probe_notes",
                entity_id: id + 1000
            }
        );
        assert_eq!(
            error.to_string(),
            format!(
                "there is no probe_notes row with id {} to write a revision of; write the record \
                 first, in the same transaction (ADR 0011 §7)",
                id + 1000
            )
        );
        let other = db.begin().await.expect("begin another");
        let unseen = insert_row(&other, Actor::System, &first_note()).await;
        let error = append(
            &txn,
            Actor::System,
            unseen,
            Reason::Create,
            &first_note(),
            None,
        )
        .await
        .expect_err("uncommitted record");
        inner::<MissingRecord>(&error);
        other.rollback().await.expect("rollback the other");
        txn.rollback().await.expect("rollback");
        assert_eq!(revision_count(db).await, 0);
    })
    .await;
}

#[tokio::test]
async fn append_reports_an_unknown_table_or_account_as_a_database_error() {
    // Both fail in the database, which leaves the transaction to its
    // rollback.
    with_app(|_router, ctx| async move {
        let db = &ctx.db;
        probe_notes::create_table(db).await;
        let id = committed_note(db).await;
        for (result, needle) in [
            (
                {
                    let txn = db.begin().await.expect("begin");
                    let result = append(
                        &txn,
                        Actor::System,
                        1,
                        Reason::Create,
                        &NoTable {
                            title: "x".to_owned(),
                        },
                        None,
                    )
                    .await;
                    txn.rollback().await.expect("rollback");
                    result
                },
                "no_such_table",
            ),
            (
                {
                    let txn = db.begin().await.expect("begin");
                    let result = append(
                        &txn,
                        Actor::User(999_999),
                        id,
                        Reason::Create,
                        &first_note(),
                        None,
                    )
                    .await;
                    txn.rollback().await.expect("rollback");
                    result
                },
                "revisions_created_by_fkey",
            ),
        ] {
            let error = result.expect_err(needle);
            assert!(matches!(error, ModelError::DbErr(_)), "{error:?}");
            assert!(error.to_string().contains(needle), "{needle}: {error}");
        }
        assert_eq!(revision_count(db).await, 0);
    })
    .await;
}

/// Runs `test` with a pool of its own on the test's database, with room for
/// `connections` sessions at once (the test configuration's pool has two),
/// and closes that pool afterwards, also when `test` panics: `with_app`
/// can drop the test's database only once no session is left on it.
///
/// - `uri`: the test's database, `ctx.config.database.uri`.
/// - `connections`: the pool's size.
/// - `test`: the test body; it gets the pool.
async fn with_second_pool<F, Fut>(uri: &str, connections: u32, test: F)
where
    F: FnOnce(DatabaseConnection) -> Fut,
    Fut: Future<Output = ()>,
{
    let mut options = ConnectOptions::new(uri.to_owned());
    options
        .max_connections(connections)
        .min_connections(1)
        .sqlx_logging(false);
    let pool = Database::connect(options)
        .await
        .expect("open a second pool");
    let outcome = AssertUnwindSafe(test(pool.clone())).catch_unwind().await;
    pool.close().await.expect("close the second pool");
    if let Err(panic) = outcome {
        std::panic::resume_unwind(panic);
    }
}

/// How many sessions wait for a lock on a row of the test's database.
async fn row_lock_waiters(db: &DatabaseConnection) -> i64 {
    value(
        db,
        "SELECT count(*) FROM pg_stat_activity WHERE datname = current_database() \
         AND wait_event_type = 'Lock' AND wait_event IN ('transactionid', 'tuple')",
    )
    .await
}

#[tokio::test]
async fn a_concurrent_save_of_the_same_record_waits_and_takes_the_next_number() {
    with_app(|_router, ctx| async move {
        let db = &ctx.db;
        probe_notes::create_table(db).await;
        let id = committed_note(db).await;
        append_committed(db, id).await;
        with_second_pool(ctx.config.database.uri.as_str(), 3, |pool| async move {
            let first: DatabaseTransaction = pool.begin().await.expect("begin the first save");
            let mine = append(
                &first,
                Actor::System,
                id,
                Reason::Update,
                &first_note(),
                None,
            )
            .await
            .expect("the first save's revision");
            assert_eq!(mine.revision_no, 2);
            // The second save starts while the first still holds the
            // record: its append waits for the row lock (without the lock it
            // would take number 2 as well, wait on the unique index, and
            // fail once the first commits). The first commits only once the
            // database shows the second waiting.
            let second = async {
                let txn = pool.begin().await.expect("begin the second save");
                let revision = append(&txn, Actor::System, id, Reason::Update, &first_note(), None)
                    .await
                    .expect("the second save's revision");
                txn.commit().await.expect("commit the second save");
                revision
            };
            let (theirs, ()) = tokio::join!(second, commit_once_one_waits(db, first));
            assert_eq!(theirs.revision_no, 3, "the number after the committed one");
        })
        .await;
        assert_eq!(numbers(db, id).await, [1, 2, 3]);
    })
    .await;
}

#[tokio::test]
async fn many_concurrent_saves_of_one_record_take_distinct_consecutive_numbers() {
    with_app(|_router, ctx| async move {
        let db = &ctx.db;
        probe_notes::create_table(db).await;
        let id = committed_note(db).await;
        with_second_pool(ctx.config.database.uri.as_str(), 8, |pool| async move {
            let saves = (0..8).map(|round| {
                let pool = &pool;
                async move {
                    let txn = pool.begin().await.expect("begin");
                    let note = ProbeNoteRevision {
                        title: format!("Változat {round}"),
                        ..first_note()
                    };
                    // Half of the saves write the record's row first, as an
                    // update does, which already takes the row's lock; the
                    // other half only append, so their order rests on
                    // `append`'s own lock.
                    if round % 2 == 0 {
                        update_row(&txn, Actor::System, id, &note).await;
                    }
                    append(&txn, Actor::System, id, Reason::Update, &note, None)
                        .await
                        .expect("append");
                    txn.commit().await.expect("commit");
                }
            });
            join_all(saves).await;
        })
        .await;
        assert_eq!(numbers(db, id).await, (1..=8).collect::<Vec<_>>());
    })
    .await;
}

/// Polls until one session of the test's database waits for a row lock,
/// then commits `first`, the transaction it waits for; panics after about
/// five seconds.
///
/// - `db`: a connection to the test's database for the poll, outside the
///   pool whose sessions take part.
/// - `first`: the transaction that holds the lock.
async fn commit_once_one_waits(db: &DatabaseConnection, first: DatabaseTransaction) {
    for _ in 0..500 {
        if row_lock_waiters(db).await == 1 {
            first.commit().await.expect("commit the first transaction");
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("no session ever waited for the first transaction's row lock");
}

#[tokio::test]
async fn a_waiting_save_under_repeatable_read_fails_instead_of_repeating_a_number() {
    // backend3 runs at READ COMMITTED, where the waiting save takes the
    // next number (above). Under REPEATABLE READ it keeps the snapshot of
    // its first statement, which ran before the first save committed, so it
    // fails instead, in one of the two ways the module documentation of
    // `models::revisions` names: a serialization error when the first save
    // updated the row, and the unique index when it only locked it.
    with_app(|_router, ctx| async move {
        let db = &ctx.db;
        probe_notes::create_table(db).await;
        let id = committed_note(db).await;
        append_committed(db, id).await;
        with_second_pool(ctx.config.database.uri.as_str(), 3, |pool| async move {
            for (first_updates, expected) in [
                (true, "could not serialize access due to concurrent update"),
                (false, "revisions_entity_type_entity_id_revision_no_idx"),
            ] {
                let first = pool.begin().await.expect("begin the first save");
                if first_updates {
                    update_row(&first, Actor::System, id, &first_note()).await;
                }
                append(
                    &first,
                    Actor::System,
                    id,
                    Reason::Update,
                    &first_note(),
                    None,
                )
                .await
                .expect("the first save's revision");
                let second = async {
                    let txn = pool
                        .begin_with_config(Some(IsolationLevel::RepeatableRead), None)
                        .await
                        .expect("begin the second save");
                    let error =
                        append(&txn, Actor::System, id, Reason::Update, &first_note(), None)
                            .await
                            .expect_err("the waiting save fails");
                    txn.rollback().await.expect("roll the second save back");
                    error
                };
                let (error, ()) = tokio::join!(second, commit_once_one_waits(db, first));
                assert!(matches!(error, ModelError::DbErr(_)), "{error:?}");
                assert!(
                    error.to_string().contains(expected),
                    "first save updated the row: {first_updates}: {error}"
                );
            }
        })
        .await;
        // Only the two first saves wrote a revision.
        assert_eq!(numbers(db, id).await, [1, 2, 3]);
    })
    .await;
}

#[tokio::test]
async fn record_save_writes_the_revision_and_its_entry_together() {
    with_app(|_router, ctx| async move {
        let db = &ctx.db;
        probe_notes::create_table(db).await;
        let user = insert_user(db).await;
        let txn = db.begin().await.expect("begin");
        let (id, created) =
            probe_notes::create(&txn, Actor::User(user), &first_note(), Some("req-7"))
                .await
                .expect("create");
        txn.commit().await.expect("commit");
        assert_eq!(created.revision.revision_no, 1);
        let entry = &created.entry;
        assert_eq!(entry.action(), Ok(Action::Create));
        assert_eq!(entry.actor(), Ok(Actor::User(user)));
        assert_eq!(entry.entity_type.as_deref(), Some("probe_notes"));
        assert_eq!(entry.entity_id, Some(id));
        assert_eq!(entry.revision_id, Some(created.revision.id));
        assert_eq!(entry.label.as_deref(), Some("Próba jegyzet"));
        assert_eq!(entry.details, None);
        assert_eq!(entry.request_id.as_deref(), Some("req-7"));
        assert_eq!(created.revision.request_id.as_deref(), Some("req-7"));
        assert_eq!(entry.occurred_at, created.revision.created_at);
        let stored = activity_log::Entity::find_by_id(entry.id)
            .one(db)
            .await
            .expect("find")
            .expect("stored");
        assert_eq!(&stored, entry);

        // An update names the fields that changed since the last revision;
        // a revert does the same and keeps the caller's details beside them.
        let txn = db.begin().await.expect("begin");
        let updated = probe_notes::update(
            &txn,
            Actor::User(user),
            id,
            Reason::Update,
            &next_note(&first_note()),
        )
        .await
        .expect("update");
        txn.commit().await.expect("commit");
        assert_eq!(updated.revision.revision_no, 2);
        assert_eq!(updated.entry.action(), Ok(Action::Update));
        assert_eq!(
            updated.entry.details,
            Some(json!({"changed_fields": ["color", "title"]}))
        );

        let txn = db.begin().await.expect("begin");
        update_row(&txn, Actor::User(user), id, &first_note()).await;
        let reverted = record_save(
            &txn,
            Actor::User(user),
            Save {
                details: Some(json!({"revert_of": 1}).as_object().expect("object").clone()),
                label: Some(first_note().title),
                ..Save::new(id, Reason::Revert, &first_note())
            },
        )
        .await
        .expect("revert");
        txn.commit().await.expect("commit");
        assert_eq!(reverted.revision.reason(), Ok(Reason::Revert));
        assert_eq!(reverted.entry.action(), Ok(Action::Revert));
        assert_eq!(
            reverted.entry.details,
            Some(json!({"changed_fields": ["color", "title"], "revert_of": 1}))
        );
        assert_eq!(numbers(db, id).await, [1, 2, 3]);
    })
    .await;
}

#[tokio::test]
async fn record_save_compares_with_the_latest_revision_in_the_current_shape() {
    with_app(|_router, ctx| async move {
        let db = &ctx.db;
        probe_notes::create_table(db).await;
        let id = committed_note(db).await;
        // A version 1 revision, written before `name` became `title`.
        db.execute_unprepared(&format!(
            "INSERT INTO revisions (entity_type, entity_id, revision_no, reason, schema_version, \
             snapshot) VALUES ('probe_notes', {id}, 1, 'create', 1, '{{\"name\": \"Próba \
             jegyzet\", \"body\": \"<p>Első változat</p>\", \"color\": \"#336699\"}}')"
        ))
        .await
        .expect("a version 1 revision");
        let latest = revisions::Model::for_record(db, "probe_notes", id)
            .await
            .expect("revisions")
            .pop()
            .expect("one");
        assert_eq!(
            latest.read_snapshot::<ProbeNoteRevision>(),
            Ok(first_note())
        );

        // Only the colour changes; the renamed title is not a change.
        let recolored = ProbeNoteRevision {
            color: "#993366".to_owned(),
            ..first_note()
        };
        let txn = db.begin().await.expect("begin");
        let saved = probe_notes::update(&txn, Actor::System, id, Reason::Update, &recolored)
            .await
            .expect("update");
        txn.commit().await.expect("commit");
        assert_eq!(
            saved.entry.details,
            Some(json!({"changed_fields": ["color"]}))
        );

        // A revision from a newer release (rolled back from) is compared as
        // stored: its unknown field counts as changed, and the save goes on.
        db.execute_unprepared(&format!(
            "INSERT INTO revisions (entity_type, entity_id, revision_no, reason, schema_version, \
             snapshot) VALUES ('probe_notes', {id}, 3, 'update', 3, '{{\"title\": \"Próba \
             jegyzet\", \"body\": \"<p>Első változat</p>\", \"color\": \"#993366\", \
             \"pinned\": true}}')"
        ))
        .await
        .expect("a version 3 revision");
        let txn = db.begin().await.expect("begin");
        let saved = probe_notes::update(&txn, Actor::System, id, Reason::Update, &recolored)
            .await
            .expect("update after a newer revision");
        txn.commit().await.expect("commit");
        assert_eq!(saved.revision.revision_no, 4);
        assert_eq!(
            saved.entry.details,
            Some(json!({"changed_fields": ["pinned"]}))
        );
        // That revision itself cannot be read in this release's shape.
        let newer = revisions::Model::for_record(db, "probe_notes", id)
            .await
            .expect("revisions")
            .into_iter()
            .find(|revision| revision.revision_no == 3)
            .expect("revision 3");
        assert_eq!(
            newer.read_snapshot::<ProbeNoteRevision>(),
            Err(ReadSnapshotError::Upcast(UpcastError::NewerVersion {
                entity_type: "probe_notes",
                version: 3,
                current: 2
            }))
        );
    })
    .await;
}

#[tokio::test]
async fn a_latest_revision_that_cannot_be_upcast_fails_the_save() {
    // A version 1 snapshot without the `name` that version 1 always has:
    // the upcaster step refuses it. `record_save` fails the save rather than
    // write an entry with a guessed list of changed fields (its
    // documentation says why), and the caller's rollback keeps the record
    // as it was.
    with_app(|_router, ctx| async move {
        let db = &ctx.db;
        probe_notes::create_table(db).await;
        let id = committed_note(db).await;
        db.execute_unprepared(&format!(
            "INSERT INTO revisions (entity_type, entity_id, revision_no, reason, schema_version, \
             snapshot) VALUES ('probe_notes', {id}, 1, 'create', 1, '{{\"body\": null, \
             \"color\": \"#336699\"}}')"
        ))
        .await
        .expect("a malformed version 1 revision");
        let recolored = ProbeNoteRevision {
            color: "#993366".to_owned(),
            ..first_note()
        };
        let txn = db.begin().await.expect("begin");
        let error = probe_notes::update(&txn, Actor::System, id, Reason::Update, &recolored)
            .await
            .expect_err("the upcast fails");
        txn.rollback().await.expect("rollback");
        assert_eq!(
            inner::<UpcastError>(&error),
            &UpcastError::Step {
                entity_type: "probe_notes",
                from: 1,
                source: UpcastStepError::MissingField {
                    field: "name".to_owned()
                }
            }
        );
        assert_eq!(
            error.to_string(),
            "the upcast of a revision of probe_notes from version 1 to 2 failed: the field \
             \"name\" is missing"
        );
        assert_eq!(numbers(db, id).await, [1]);
        assert_eq!(entry_count(db).await, 0);
        assert_eq!(probe_notes::snapshot_of(db, id).await, first_note());
    })
    .await;
}

#[tokio::test]
async fn record_save_refuses_what_is_not_a_save() {
    with_app(|_router, ctx| async move {
        let db = &ctx.db;
        probe_notes::create_table(db).await;
        let id = committed_note(db).await;
        let txn = db.begin().await.expect("begin");
        for reason in [Reason::Import, Reason::ImportOriginal] {
            let error = record_save(&txn, Actor::Import, Save::new(id, reason, &first_note()))
                .await
                .expect_err("an import is not a save");
            assert_eq!(
                inner::<RevisionError>(&error),
                &RevisionError::NotASave { reason }
            );
        }
        let error = record_save(
            &txn,
            Actor::System,
            Save {
                details: Some(
                    json!({"changed_fields": ["title"]})
                        .as_object()
                        .expect("object")
                        .clone(),
                ),
                ..Save::new(id, Reason::Create, &first_note())
            },
        )
        .await
        .expect_err("reserved key");
        assert_eq!(
            inner::<RevisionError>(&error),
            &RevisionError::ReservedDetailKey {
                key: "changed_fields"
            }
        );
        txn.rollback().await.expect("rollback");
        assert_eq!(revision_count(db).await, 0);
        assert_eq!(entry_count(db).await, 0);
    })
    .await;
}

#[tokio::test]
async fn a_refused_entry_fails_the_save_and_its_rollback_removes_the_revision() {
    with_app(|_router, ctx| async move {
        let db = &ctx.db;
        probe_notes::create_table(db).await;
        let txn = db.begin().await.expect("begin");
        let id = insert_row(&txn, Actor::System, &first_note()).await;
        // A blank label breaks a rule of the activity log, checked after
        // the revision was inserted.
        let error = record_save(
            &txn,
            Actor::System,
            Save {
                label: Some("  ".to_owned()),
                ..Save::new(id, Reason::Create, &first_note())
            },
        )
        .await
        .expect_err("blank label");
        assert!(error.to_string().contains("blank label"), "{error}");
        assert_eq!(
            value::<_, i64>(&txn, "SELECT count(*) FROM revisions").await,
            1,
            "the revision is in the transaction until it rolls back"
        );
        txn.rollback().await.expect("rollback");
        assert_eq!(revision_count(db).await, 0);
        assert_eq!(entry_count(db).await, 0);
        assert_eq!(
            value::<_, i64>(db, "SELECT count(*) FROM probe_notes").await,
            0
        );
    })
    .await;
}

#[tokio::test]
async fn read_snapshot_refuses_a_revision_of_another_entity() {
    with_app(|_router, ctx| async move {
        let db = &ctx.db;
        let revision: Model = {
            db.execute_unprepared(
                "INSERT INTO revisions (entity_type, entity_id, revision_no, reason, \
                 schema_version, snapshot) VALUES ('pages', 4, 1, 'create', 1, '{\"title\": \
                 \"x\"}')",
            )
            .await
            .expect("insert");
            Model::for_record(db, "pages", 4)
                .await
                .expect("revisions")
                .pop()
                .expect("one")
        };
        assert_eq!(
            revision.read_snapshot::<ProbeNoteRevision>(),
            Err(ReadSnapshotError::WrongEntityType {
                id: revision.id,
                stored: "pages".to_owned(),
                expected: "probe_notes"
            })
        );
    })
    .await;
}

#[tokio::test]
async fn delete_for_record_deletes_exactly_that_records_revisions() {
    // A purge removes the purged record's revisions, and an anonymisation
    // those of the account record (ADR 0011 §6). Other records, also of
    // another entity with the same id, keep theirs.
    with_app(|_router, ctx| async move {
        let db = &ctx.db;
        probe_notes::create_table(db).await;
        let purged = committed_note(db).await;
        let kept = committed_note(db).await;
        for id in [purged, purged, kept] {
            append_committed(db, id).await;
        }
        db.execute_unprepared(&format!(
            "INSERT INTO revisions (entity_type, entity_id, revision_no, reason, schema_version, \
             snapshot) VALUES ('pages', {purged}, 1, 'create', 1, '{{}}')"
        ))
        .await
        .expect("a page revision with the same id");

        // The purge's order: the record's row first, then its revisions. The
        // row is gone then, and this transaction holds its lock already.
        let txn = db.begin().await.expect("begin");
        activity_log::with_permanent_delete_flag(&txn)
            .await
            .expect("set the flag");
        txn.execute_unprepared(&format!("DELETE FROM probe_notes WHERE id = {purged}"))
            .await
            .expect("purge the row");
        assert_eq!(
            delete_for_record::<ProbeNoteRevision>(&txn, purged)
                .await
                .expect("delete the revisions"),
            2
        );
        txn.commit().await.expect("commit");
        assert_eq!(numbers(db, purged).await, Vec::<i32>::new());
        assert_eq!(numbers(db, kept).await, [1]);
        assert_eq!(
            Model::for_record(db, "pages", purged)
                .await
                .expect("revisions")
                .len(),
            1
        );

        // A record whose row is still there (an anonymised account keeps
        // it), and a second call that finds nothing left.
        let txn = db.begin().await.expect("begin");
        activity_log::with_permanent_delete_flag(&txn)
            .await
            .expect("set the flag");
        assert_eq!(
            delete_for_record::<ProbeNoteRevision>(&txn, kept)
                .await
                .expect("delete the revisions"),
            1
        );
        assert_eq!(
            delete_for_record::<ProbeNoteRevision>(&txn, kept)
                .await
                .expect("nothing left"),
            0
        );
        txn.commit().await.expect("commit");
        assert_eq!(numbers(db, kept).await, Vec::<i32>::new());
        assert_eq!(revision_count(db).await, 1, "the page's revision");
    })
    .await;
}

#[tokio::test]
async fn delete_for_record_is_refused_outside_a_permanent_delete_or_for_a_broken_type() {
    with_app(|_router, ctx| async move {
        let db = &ctx.db;
        probe_notes::create_table(db).await;
        let id = committed_note(db).await;
        append_committed(db, id).await;
        // Without the flag the table's trigger refuses the DELETE.
        let txn = db.begin().await.expect("begin");
        let error = delete_for_record::<ProbeNoteRevision>(&txn, id)
            .await
            .expect_err("no permanent-delete flag");
        txn.rollback().await.expect("rollback");
        assert!(matches!(error, ModelError::DbErr(_)), "{error:?}");
        assert!(
            error
                .to_string()
                .contains("DELETE is allowed only in a permanent-delete transaction"),
            "{error}"
        );
        // A snapshot type that breaks a rule is refused before any
        // statement, also with the flag.
        let txn = db.begin().await.expect("begin");
        activity_log::with_permanent_delete_flag(&txn)
            .await
            .expect("set the flag");
        let error = delete_for_record::<MissingStep>(&txn, id)
            .await
            .expect_err("registry gap");
        assert_eq!(
            inner::<SnapshotTypeError>(&error),
            &SnapshotTypeError::UpcastCount {
                entity_type: "probe_notes",
                version: 2,
                expected: 1,
                steps: 0
            }
        );
        txn.rollback().await.expect("rollback");
        assert_eq!(numbers(db, id).await, [1]);
    })
    .await;
}

#[tokio::test]
async fn delete_for_record_waits_for_a_running_save_and_deletes_its_revision_too() {
    // The save has inserted revision 2 but not committed it. A DELETE on
    // its own would not see that revision, nor wait for it, and the
    // revision would outlive the permanent delete. `delete_for_record`
    // waits for the record's row lock first, so it deletes both. The
    // permanent delete starts here before it writes the record, to show
    // that the function's own lock is enough.
    with_app(|_router, ctx| async move {
        let db = &ctx.db;
        probe_notes::create_table(db).await;
        let id = committed_note(db).await;
        append_committed(db, id).await;
        with_second_pool(ctx.config.database.uri.as_str(), 3, |pool| async move {
            let save: DatabaseTransaction = pool.begin().await.expect("begin the save");
            let saved = probe_notes::update(
                &save,
                Actor::System,
                id,
                Reason::Update,
                &next_note(&first_note()),
            )
            .await
            .expect("the save");
            assert_eq!(saved.revision.revision_no, 2);
            let permanent_delete = async {
                let txn = pool.begin().await.expect("begin the permanent delete");
                activity_log::with_permanent_delete_flag(&txn)
                    .await
                    .expect("set the flag");
                let deleted = delete_for_record::<ProbeNoteRevision>(&txn, id)
                    .await
                    .expect("delete the revisions");
                txn.commit().await.expect("commit the permanent delete");
                deleted
            };
            let (deleted, ()) = tokio::join!(permanent_delete, commit_once_one_waits(db, save));
            assert_eq!(deleted, 2, "the committed revision and the save's");
        })
        .await;
        assert_eq!(numbers(db, id).await, Vec::<i32>::new());
    })
    .await;
}

#[tokio::test]
async fn the_check_allows_exactly_the_reasons_of_the_domain() {
    // A new reason is a variant of `Reason` plus a migration that replaces
    // the CHECK; this fails until both are there (ADR 0009, "Column types").
    with_app(|_router, ctx| async move {
        let db = &ctx.db;
        let stored: Vec<String> = db
            .query_all_raw(Statement::from_string(
                DbBackend::Postgres,
                "SELECT matched[1] FROM pg_constraint, \
                 regexp_matches(pg_get_constraintdef(pg_constraint.oid), '''([a-z_]+)''::text', \
                 'g') WITH ORDINALITY AS found(matched, n) \
                 WHERE conrelid = 'revisions'::regclass AND conname = 'revisions_reason_known' \
                 ORDER BY n",
            ))
            .await
            .expect("the CHECK")
            .iter()
            .map(|row| row.try_get_by_index(0).expect("text"))
            .collect();
        let names: Vec<&str> = Reason::ALL.iter().map(|reason| reason.as_str()).collect();
        assert_eq!(
            stored, names,
            "the revisions_reason_known CHECK and Reason::ALL list different reasons"
        );
        let imports: Vec<String> = db
            .query_all_raw(Statement::from_string(
                DbBackend::Postgres,
                "SELECT matched[1] FROM pg_constraint, \
                 regexp_matches(pg_get_constraintdef(pg_constraint.oid), '''([a-z_]+)''::text', \
                 'g') WITH ORDINALITY AS found(matched, n) \
                 WHERE conrelid = 'revisions'::regclass \
                 AND conname = 'revisions_import_without_user' ORDER BY n",
            ))
            .await
            .expect("the CHECK")
            .iter()
            .map(|row| row.try_get_by_index(0).expect("text"))
            .collect();
        let import_names: Vec<&str> = Reason::ALL
            .iter()
            .filter(|reason| reason.is_import())
            .map(|reason| reason.as_str())
            .collect();
        assert_eq!(
            imports, import_names,
            "the revisions_import_without_user CHECK and Reason::is_import differ"
        );
    })
    .await;
}

#[tokio::test]
async fn app_truncate_leaves_the_revisions_alone() {
    with_app(|_router, ctx| async move {
        let db = &ctx.db;
        probe_notes::create_table(db).await;
        let id = committed_note(db).await;
        let revision = append_committed(db, id).await;
        App::truncate(&ctx).await.expect("truncate");
        assert_eq!(
            revisions::Entity::find_by_id(revision.id)
                .one(db)
                .await
                .expect("find"),
            Some(revision)
        );
        // The stored JSON is the snapshot, member for member.
        assert_eq!(
            revisions::Model::for_record(db, "probe_notes", id)
                .await
                .expect("revisions")[0]
                .snapshot
                .get("title"),
            Some(&Value::from("Próba jegyzet"))
        );
    })
    .await;
}
