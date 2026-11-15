//! The safety-net helper of `tests/support/safety_net.rs`, proven on the
//! test-only `probe_notes` entity (`tests/support/probe_notes.rs`): the
//! probe, written the way recipe 2 of the project skill writes an entity,
//! passes every check for a user and for the system, and each broken variant
//! below fails the check that is there to catch its mistake.
//!
//! Each broken variant changes one thing of the probe's write methods and
//! nothing else, so its test shows which check notices it.

use backend3::{
    domain::{
        activity::{Action, Actor},
        revision::Reason,
    },
    models::{
        activity_log::{self, EntityRef, Entry},
        revisions::{self, Save, record_save},
    },
};
use loco_rs::model::ModelResult;
use sea_orm::{
    ConnectionTrait, DatabaseConnection, DatabaseTransaction, DbBackend, Statement, Value,
};

use crate::support::{
    probe_notes::{
        self, ProbeNoteRevision, ProbeNotes, first_note, insert_row, next_note, snapshot_of,
        update_row,
    },
    safety_net::{
        SafetyNetEntity, assert_create, assert_failed_entry_rolls_back, assert_safety_net,
        assert_update,
    },
    with_app,
};

/// Inserts an invented account and returns its id.
async fn insert_user(db: &DatabaseConnection) -> i64 {
    db.query_one_raw(Statement::from_string(
        DbBackend::Postgres,
        "INSERT INTO users (pid, name, email) \
         VALUES (gen_random_uuid(), 'Próba', 'proba@example.test') RETURNING id",
    ))
    .await
    .expect("insert the account")
    .expect("one row")
    .try_get_by_index(0)
    .expect("id")
}

/// The probe's own read side, which every broken variant shares.
macro_rules! probe_reads {
    () => {
        type Snapshot = ProbeNoteRevision;

        async fn snapshot(&self, db: &DatabaseConnection, id: i64) -> ProbeNoteRevision {
            snapshot_of(db, id).await
        }

        fn label(&self, snapshot: &ProbeNoteRevision) -> Option<String> {
            Some(snapshot.title.clone())
        }
    };
}

#[tokio::test]
async fn the_probe_entity_meets_every_check_for_a_user_and_for_the_system() {
    with_app(|_router, ctx| async move {
        let db = &ctx.db;
        probe_notes::create_table(db).await;
        let user = insert_user(db).await;
        for actor in [Actor::User(user), Actor::System] {
            let id = assert_safety_net(db, &ProbeNotes, actor).await;
            // Create and two updates: revisions 1 to 3.
            let numbers: Vec<i32> = revisions::Model::for_record(db, "probe_notes", id)
                .await
                .expect("revisions")
                .iter()
                .map(|revision| revision.revision_no)
                .collect();
            assert_eq!(numbers, [1, 2, 3]);
        }
    })
    .await;
}

/// Creates a note's row and its `create` entry, but no revision.
struct NoRevision;

impl SafetyNetEntity for NoRevision {
    probe_reads!();

    async fn create(&self, txn: &DatabaseTransaction, actor: Actor) -> ModelResult<i64> {
        let id = insert_row(txn, actor, &first_note()).await;
        activity_log::record(
            txn,
            actor,
            Entry {
                entity: Some(EntityRef::record("probe_notes", id)),
                ..Entry::new(Action::Create)
            },
        )
        .await?;
        Ok(id)
    }

    async fn update(&self, txn: &DatabaseTransaction, actor: Actor, id: i64) -> ModelResult<()> {
        ProbeNotes.update(txn, actor, id).await
    }

    fn changed_fields(&self) -> Vec<String> {
        ProbeNotes.changed_fields()
    }
}

#[tokio::test]
#[should_panic(expected = "has exactly one revision, revision 1")]
async fn a_create_without_a_revision_fails_the_check() {
    with_app(|_router, ctx| async move {
        probe_notes::create_table(&ctx.db).await;
        assert_create(&ctx.db, &NoRevision, Actor::System).await;
    })
    .await;
}

/// Writes the revision of a create, but its entry on the pool, outside the
/// save's transaction.
struct EntryOnThePool {
    pool: DatabaseConnection,
}

impl SafetyNetEntity for EntryOnThePool {
    probe_reads!();

    async fn create(&self, txn: &DatabaseTransaction, actor: Actor) -> ModelResult<i64> {
        let note = first_note();
        let id = insert_row(txn, actor, &note).await;
        let revision = revisions::append(txn, actor, id, Reason::Create, &note, None).await?;
        activity_log::record(
            &self.pool,
            actor,
            Entry {
                entity: Some(EntityRef::record("probe_notes", id)),
                revision_id: Some(revision.id),
                label: Some(note.title),
                ..Entry::new(Action::Create)
            },
        )
        .await?;
        Ok(id)
    }

    async fn update(&self, txn: &DatabaseTransaction, actor: Actor, id: i64) -> ModelResult<()> {
        ProbeNotes.update(txn, actor, id).await
    }

    fn changed_fields(&self) -> Vec<String> {
        ProbeNotes.changed_fields()
    }
}

#[tokio::test]
#[should_panic(expected = "succeeded although its activity entry was refused")]
async fn an_entry_outside_the_save_transaction_fails_the_check() {
    with_app(|_router, ctx| async move {
        probe_notes::create_table(&ctx.db).await;
        let entity = EntryOnThePool {
            pool: ctx.db.clone(),
        };
        assert_failed_entry_rolls_back(&ctx.db, &entity, Actor::System, None).await;
    })
    .await;
}

/// Writes the row of a create on the pool, outside the save's transaction,
/// and its revision and entry inside it.
struct RowOnThePool {
    pool: DatabaseConnection,
}

impl SafetyNetEntity for RowOnThePool {
    probe_reads!();

    async fn create(&self, txn: &DatabaseTransaction, actor: Actor) -> ModelResult<i64> {
        let note = first_note();
        let id: i64 = self
            .pool
            .query_one_raw(Statement::from_sql_and_values(
                DbBackend::Postgres,
                "INSERT INTO probe_notes (title, body, color, created_by, updated_by) \
                 VALUES ($1, $2, $3, $4, $4) RETURNING id",
                [
                    Value::from(note.title.clone()),
                    Value::from(note.body.clone()),
                    Value::from(note.color.clone()),
                    Value::from(actor.user_id()),
                ],
            ))
            .await?
            .expect("one row")
            .try_get_by_index(0)
            .expect("id");
        record_save(
            txn,
            actor,
            Save {
                label: Some(note.title.clone()),
                ..Save::new(id, Reason::Create, &note)
            },
        )
        .await?;
        Ok(id)
    }

    async fn update(&self, txn: &DatabaseTransaction, actor: Actor, id: i64) -> ModelResult<()> {
        ProbeNotes.update(txn, actor, id).await
    }

    fn changed_fields(&self) -> Vec<String> {
        ProbeNotes.changed_fields()
    }
}

#[tokio::test]
#[should_panic(expected = "writes exactly 0 new record(s), 0 revision(s)")]
async fn a_record_written_outside_the_save_transaction_fails_the_check() {
    with_app(|_router, ctx| async move {
        probe_notes::create_table(&ctx.db).await;
        let entity = RowOnThePool {
            pool: ctx.db.clone(),
        };
        assert_failed_entry_rolls_back(&ctx.db, &entity, Actor::System, None).await;
    })
    .await;
}

/// Changes only the title on update, but claims the colour too.
struct ClaimsAnUnchangedField;

impl SafetyNetEntity for ClaimsAnUnchangedField {
    probe_reads!();

    async fn create(&self, txn: &DatabaseTransaction, actor: Actor) -> ModelResult<i64> {
        ProbeNotes.create(txn, actor).await
    }

    async fn update(&self, txn: &DatabaseTransaction, actor: Actor, id: i64) -> ModelResult<()> {
        let current = snapshot_of(txn, id).await;
        let next = ProbeNoteRevision {
            color: current.color.clone(),
            ..next_note(&current)
        };
        probe_notes::update(txn, actor, id, Reason::Update, &next)
            .await
            .map(|_| ())
    }

    fn changed_fields(&self) -> Vec<String> {
        ProbeNotes.changed_fields()
    }
}

#[tokio::test]
#[should_panic(expected = "lists exactly the changed fields [\"color\", \"title\"]")]
async fn an_update_entry_with_the_wrong_field_names_fails_the_check() {
    with_app(|_router, ctx| async move {
        probe_notes::create_table(&ctx.db).await;
        let id = assert_create(&ctx.db, &ClaimsAnUnchangedField, Actor::System).await;
        assert_update(&ctx.db, &ClaimsAnUnchangedField, Actor::System, id).await;
    })
    .await;
}

/// Updates the row without its audit columns.
struct NoAuditColumns;

impl SafetyNetEntity for NoAuditColumns {
    probe_reads!();

    async fn create(&self, txn: &DatabaseTransaction, actor: Actor) -> ModelResult<i64> {
        ProbeNotes.create(txn, actor).await
    }

    async fn update(&self, txn: &DatabaseTransaction, actor: Actor, id: i64) -> ModelResult<()> {
        let next = next_note(&snapshot_of(txn, id).await);
        txn.execute_raw(Statement::from_sql_and_values(
            DbBackend::Postgres,
            "UPDATE probe_notes SET title = $1, color = $2 WHERE id = $3",
            [
                Value::from(next.title.clone()),
                Value::from(next.color.clone()),
                Value::from(id),
            ],
        ))
        .await?;
        record_save(
            txn,
            actor,
            Save {
                label: Some(next.title.clone()),
                ..Save::new(id, Reason::Update, &next)
            },
        )
        .await
        .map(|_| ())
    }

    fn changed_fields(&self) -> Vec<String> {
        ProbeNotes.changed_fields()
    }
}

#[tokio::test]
#[should_panic(expected = "updated_at of probe_notes")]
async fn an_update_that_leaves_updated_at_alone_fails_the_check() {
    with_app(|_router, ctx| async move {
        let db = &ctx.db;
        probe_notes::create_table(db).await;
        let id = assert_create(db, &NoAuditColumns, Actor::System).await;
        // A transaction of its own starts later than the create's, so the
        // instants differ.
        db.execute_unprepared("SELECT pg_sleep(0.01)")
            .await
            .expect("let the clock move");
        assert_update(db, &NoAuditColumns, Actor::System, id).await;
    })
    .await;
}

/// Writes the snapshot of the note before the update into the update's
/// revision.
struct StaleSnapshot;

impl SafetyNetEntity for StaleSnapshot {
    probe_reads!();

    async fn create(&self, txn: &DatabaseTransaction, actor: Actor) -> ModelResult<i64> {
        ProbeNotes.create(txn, actor).await
    }

    async fn update(&self, txn: &DatabaseTransaction, actor: Actor, id: i64) -> ModelResult<()> {
        let current = snapshot_of(txn, id).await;
        update_row(txn, actor, id, &next_note(&current)).await;
        record_save(
            txn,
            actor,
            Save {
                label: Some(current.title.clone()),
                ..Save::new(id, Reason::Update, &current)
            },
        )
        .await
        .map(|_| ())
    }

    fn changed_fields(&self) -> Vec<String> {
        ProbeNotes.changed_fields()
    }
}

#[tokio::test]
#[should_panic(expected = "is the record after the save")]
async fn a_revision_of_the_record_before_the_save_fails_the_check() {
    with_app(|_router, ctx| async move {
        probe_notes::create_table(&ctx.db).await;
        let id = assert_create(&ctx.db, &StaleSnapshot, Actor::System).await;
        assert_update(&ctx.db, &StaleSnapshot, Actor::System, id).await;
    })
    .await;
}
