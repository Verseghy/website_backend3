//! The safety-net checks that every admin-editable entity runs on its own
//! write methods (ADR 0011 §1, §4, §5, §7 and its "What must be true").
//!
//! An entity slice implements [`SafetyNetEntity`] for its model in its
//! model tests and calls [`assert_safety_net`]; `tests/models/safety_net.rs`
//! proves the helper on the test-only [`super::probe_notes`] entity, and
//! that it fails for each mistake it is there to catch. The checks:
//!
//! - **Create** writes revision 1 (reason `create`, the type's current
//!   `schema_version`, the actor's user id, a snapshot of exactly the
//!   allow-listed fields that equals the stored record) and one `create`
//!   entry that names the record and that revision.
//! - **Update** writes the next revision (reason `update`) and one `update`
//!   entry whose `details` list exactly the changed field names.
//! - **One transaction, one instant**: the revision's `created_at`, the
//!   entry's `occurred_at` and the record's audit columns are the same
//!   instant, the start of the save's transaction on PostgreSQL's clock
//!   (research: DECISIONS R15, D90); `created_by` and `updated_by` are the
//!   actor's (ADR 0011 §2), and an update leaves the `created_*` pair alone.
//! - **A failed activity entry rolls the save back** (ADR 0011, "A failure
//!   while inserting the activity entry rolls back the record change"): with
//!   a trigger that refuses every activity insert of the save's transaction,
//!   the write fails, and after its rollback the record, its revisions and
//!   the log are as they were. The trigger is committed before the save and
//!   dropped after it, and it refuses only inside a transaction that set the
//!   test's own setting [`REFUSE_ENTRIES_FLAG`] with `SET LOCAL`, so an entry
//!   written on another connection gets through, and the check sees it.
//!   (Creating the trigger inside the save's transaction would not do: the
//!   `CREATE TRIGGER` locks `activity_log` until that transaction ends, so a
//!   write on another connection would wait for it forever.)
//!
//! The helper writes and commits through the entity's own methods on the
//! pool it is given (each check opens and commits its own transaction), and
//! reads only through SQL and the revision and activity-log models.

use std::future::Future;

use backend3::{
    domain::{
        activity::{Action, Actor, CHANGED_FIELDS_KEY},
        revision::{Reason, RevisionSnapshot, snapshot_object},
    },
    models::{activity_log, revisions},
};
use loco_rs::model::ModelResult;
use sea_orm::{
    ColumnTrait, ConnectionTrait, DatabaseConnection, DatabaseTransaction, DbBackend, EntityTrait,
    QueryFilter, QueryOrder, Statement, TransactionTrait, prelude::DateTimeWithTimeZone,
};
use serde_json::Value;

/// An admin-editable entity under test: how to create and update one of its
/// records through the entity's own write methods.
pub trait SafetyNetEntity {
    /// The entity's revision snapshot type; its `ENTITY_TYPE` is the
    /// entity's table.
    type Snapshot: RevisionSnapshot + PartialEq + std::fmt::Debug;

    /// Creates one record on `txn` as `actor` through the entity's create
    /// method, and returns its id. The helper commits or rolls back `txn`.
    ///
    /// - `txn`: the save's transaction.
    /// - `actor`: who creates it.
    fn create(
        &self,
        txn: &DatabaseTransaction,
        actor: Actor,
    ) -> impl Future<Output = ModelResult<i64>>;

    /// Updates record `id` on `txn` as `actor` through the entity's update
    /// method, so that exactly the fields of [`Self::changed_fields`] get new
    /// values, on every call.
    ///
    /// - `txn`: the save's transaction.
    /// - `actor`: who saves it.
    /// - `id`: the record.
    fn update(
        &self,
        txn: &DatabaseTransaction,
        actor: Actor,
        id: i64,
    ) -> impl Future<Output = ModelResult<()>>;

    /// The fields that [`Self::update`] changes, as its `update` entry must
    /// list them.
    fn changed_fields(&self) -> Vec<String>;

    /// The snapshot of record `id` as stored now, built the way the entity
    /// builds its revisions.
    ///
    /// - `db`: the test's pool.
    /// - `id`: the record.
    fn snapshot(&self, db: &DatabaseConnection, id: i64) -> impl Future<Output = Self::Snapshot>;

    /// The label the record's entries carry: its title or name for a
    /// content record, `None` for people (ADR 0011 §5).
    ///
    /// - `snapshot`: the record after the save.
    fn label(&self, snapshot: &Self::Snapshot) -> Option<String>;
}

/// The test's own transaction-local setting that switches the refusing
/// trigger on: [`assert_failed_entry_rolls_back`] sets it with `SET LOCAL`
/// in the save's transaction only.
pub const REFUSE_ENTRIES_FLAG: &str = "backend3_test.refuse_entries";

/// The message of the trigger that [`assert_failed_entry_rolls_back`]
/// installs.
const REFUSED_ENTRY: &str = "safety-net check: this activity entry is refused";

/// The function and trigger that refuse every activity insert of a
/// transaction that set [`REFUSE_ENTRIES_FLAG`] to `on`, committed before
/// the save under test.
const INSTALL_REFUSAL: &str = "CREATE FUNCTION safety_net_refuse_entry() RETURNS trigger \
     LANGUAGE plpgsql AS $$ BEGIN \
     IF current_setting('backend3_test.refuse_entries', true) = 'on' THEN \
     RAISE EXCEPTION 'safety-net check: this activity entry is refused'; END IF; \
     RETURN NEW; END $$; \
     CREATE TRIGGER safety_net_refuse_entry BEFORE INSERT ON activity_log \
     FOR EACH ROW EXECUTE FUNCTION safety_net_refuse_entry()";

/// Removes what [`INSTALL_REFUSAL`] created, after the save.
const REMOVE_REFUSAL: &str = "DROP TRIGGER safety_net_refuse_entry ON activity_log; \
     DROP FUNCTION safety_net_refuse_entry()";

/// Runs every check of the module documentation for `entity`, as `actor`:
/// a create, two updates, and a failed create and a failed update.
///
/// - `db`: the test's pool (`ctx.db`); every check commits on it.
/// - `entity`: the entity under test.
/// - `actor`: who saves: [`Actor::User`] of an existing account, or
///   [`Actor::System`].
///
/// Returns the id of the record it created and updated.
pub async fn assert_safety_net<E: SafetyNetEntity>(
    db: &DatabaseConnection,
    entity: &E,
    actor: Actor,
) -> i64 {
    let id = assert_create(db, entity, actor).await;
    assert_update(db, entity, actor, id).await;
    assert_update(db, entity, actor, id).await;
    assert_failed_entry_rolls_back(db, entity, actor, None).await;
    assert_failed_entry_rolls_back(db, entity, actor, Some(id)).await;
    id
}

/// Creates a record and checks that it got revision 1 and one `create`
/// entry, in one transaction; returns the record's id.
///
/// - `db`: the test's pool.
/// - `entity`: the entity under test.
/// - `actor`: who creates it.
pub async fn assert_create<E: SafetyNetEntity>(
    db: &DatabaseConnection,
    entity: &E,
    actor: Actor,
) -> i64 {
    let entity_type = E::Snapshot::ENTITY_TYPE;
    let totals = Totals::read::<E>(db).await;
    let txn = db.begin().await.expect("begin the create");
    let id = entity
        .create(&txn, actor)
        .await
        .unwrap_or_else(|error| panic!("create a record of {entity_type}: {error}"));
    txn.commit().await.expect("commit the create");

    let revisions = revisions_of::<E>(db, id).await;
    assert_eq!(
        revisions.len(),
        1,
        "a new record of {entity_type} has exactly one revision, revision 1 (ADR 0011 §4)"
    );
    let revision = &revisions[0];
    assert_eq!(revision.revision_no, 1, "the create's revision number");
    assert_revision(db, entity, actor, revision, Reason::Create).await;
    let entry = only_new_entry::<E>(db, id, 0).await;
    assert_entry(
        entity,
        actor,
        &entry,
        revision,
        Action::Create,
        &db_snapshot(db, entity, id).await,
    );
    assert!(
        entry
            .details
            .as_ref()
            .is_none_or(|details| details.get(CHANGED_FIELDS_KEY).is_none()),
        "a create entry of {entity_type} lists no changed fields: {:?}",
        entry.details
    );

    let audit = Audit::read::<E>(db, id).await;
    assert_eq!(
        (audit.created_at, audit.updated_at),
        (revision.created_at, revision.created_at),
        "created_at and updated_at of a new {entity_type} record are the instant of its \
         revision: the transaction's start (research: DECISIONS R15, D90)"
    );
    assert_eq!(
        (audit.created_by, audit.updated_by),
        (actor.user_id(), actor.user_id()),
        "created_by and updated_by of a new {entity_type} record are the actor's (ADR 0011 §2)"
    );
    totals.assert_grew::<E>(db, 1, 1).await;
    id
}

/// Updates record `id` and checks that it got the next revision and one
/// `update` entry listing exactly the changed fields, in one transaction.
///
/// - `db`: the test's pool.
/// - `entity`: the entity under test.
/// - `actor`: who saves it.
/// - `id`: a record that [`assert_create`] made.
pub async fn assert_update<E: SafetyNetEntity>(
    db: &DatabaseConnection,
    entity: &E,
    actor: Actor,
    id: i64,
) {
    let entity_type = E::Snapshot::ENTITY_TYPE;
    let totals = Totals::read::<E>(db).await;
    let before = revisions_of::<E>(db, id).await;
    let entries_before = entries_of::<E>(db, id).await.len();
    let audit_before = Audit::read::<E>(db, id).await;
    let txn = db.begin().await.expect("begin the update");
    entity
        .update(&txn, actor, id)
        .await
        .unwrap_or_else(|error| panic!("update record {id} of {entity_type}: {error}"));
    txn.commit().await.expect("commit the update");

    let after = revisions_of::<E>(db, id).await;
    assert_eq!(
        after.len(),
        before.len() + 1,
        "an update of {entity_type} {id} writes exactly one revision (ADR 0011 §4)"
    );
    assert_eq!(
        after[..before.len()],
        before[..],
        "an update leaves the earlier revisions of {entity_type} {id} as they were"
    );
    let revision = after.last().expect("the new revision");
    let previous = before.last().map_or(0, |revision| revision.revision_no);
    assert_eq!(
        revision.revision_no,
        previous + 1,
        "an update of {entity_type} {id} takes the next revision number"
    );
    assert_revision(db, entity, actor, revision, Reason::Update).await;
    let entry = only_new_entry::<E>(db, id, entries_before).await;
    assert_entry(
        entity,
        actor,
        &entry,
        revision,
        Action::Update,
        &db_snapshot(db, entity, id).await,
    );
    let mut expected = entity.changed_fields();
    expected.sort_unstable();
    let listed = entry
        .details
        .as_ref()
        .and_then(|details| details.get(CHANGED_FIELDS_KEY));
    assert_eq!(
        listed,
        Some(&Value::from(expected.clone())),
        "the update entry of {entity_type} {id} lists exactly the changed fields {expected:?} \
         (ADR 0011 §5)"
    );

    let audit = Audit::read::<E>(db, id).await;
    assert_eq!(
        audit.updated_at, revision.created_at,
        "updated_at of {entity_type} {id} is the instant of the update's revision: the \
         transaction's start (Expr::current_timestamp(); research: DECISIONS R15, D90)"
    );
    assert_eq!(
        audit.updated_by,
        actor.user_id(),
        "updated_by of {entity_type} {id} is the actor's (ADR 0011 §2)"
    );
    assert_eq!(
        (audit.created_at, audit.created_by),
        (audit_before.created_at, audit_before.created_by),
        "an update leaves created_at and created_by of {entity_type} {id} alone"
    );
    totals.assert_grew::<E>(db, 0, 1).await;
}

/// Runs a create (`id` `None`) or an update of record `id` in a transaction
/// in which every activity insert fails, and checks that the write fails and
/// that its rollback leaves the record, the revisions and the log as they
/// were (ADR 0011, "What must be true").
///
/// - `db`: the test's pool.
/// - `entity`: the entity under test.
/// - `actor`: who saves.
/// - `id`: the record to update, or `None` to create one.
pub async fn assert_failed_entry_rolls_back<E: SafetyNetEntity>(
    db: &DatabaseConnection,
    entity: &E,
    actor: Actor,
    id: Option<i64>,
) {
    let entity_type = E::Snapshot::ENTITY_TYPE;
    let totals = Totals::read::<E>(db).await;
    let record_before = match id {
        Some(id) => Some(record_text::<E>(db, id).await),
        None => None,
    };
    db.execute_unprepared(INSTALL_REFUSAL)
        .await
        .expect("install the refusing trigger");
    let txn = db.begin().await.expect("begin the failing save");
    txn.execute_unprepared(&format!("SET LOCAL {REFUSE_ENTRIES_FLAG} = 'on'"))
        .await
        .expect("switch the refusal on for the save's transaction");
    let outcome = match id {
        Some(id) => entity.update(&txn, actor, id).await,
        None => entity.create(&txn, actor).await.map(|_| ()),
    };
    // Roll back and remove the trigger before any assertion, so a failing
    // check leaves nothing behind for the rest of the test.
    txn.rollback().await.expect("roll the failed save back");
    db.execute_unprepared(REMOVE_REFUSAL)
        .await
        .expect("remove the refusing trigger");
    let error = match outcome {
        Ok(()) => panic!(
            "a save of {entity_type} succeeded although its activity entry was refused: the \
             entry must be written in the save's own transaction (ADR 0011 §5)"
        ),
        Err(error) => error,
    };
    assert!(
        error.to_string().contains(REFUSED_ENTRY),
        "the save of {entity_type} failed for another reason than its refused entry: {error}"
    );

    totals.assert_grew::<E>(db, 0, 0).await;
    if let (Some(id), Some(before)) = (id, record_before) {
        assert_eq!(
            record_text::<E>(db, id).await,
            before,
            "a failed update of {entity_type} {id} leaves the record as it was"
        );
    }
    assert_eq!(
        trigger_count(db).await,
        0,
        "the refusing trigger is gone after the check"
    );
}

/// Checks a revision against the entity's stored record.
async fn assert_revision<E: SafetyNetEntity>(
    db: &DatabaseConnection,
    entity: &E,
    actor: Actor,
    revision: &revisions::Model,
    reason: Reason,
) {
    let entity_type = E::Snapshot::ENTITY_TYPE;
    assert_eq!(revision.entity_type, entity_type);
    assert_eq!(
        revision.reason(),
        Ok(reason),
        "revision {} of {entity_type}",
        revision.revision_no
    );
    assert_eq!(
        revision.schema_version,
        E::Snapshot::SCHEMA_VERSION,
        "a new revision of {entity_type} has the current schema_version"
    );
    assert_eq!(
        revision.created_by,
        actor.user_id(),
        "created_by of the revision is the actor's user id (ADR 0011 §2)"
    );
    let Value::Object(stored) = &revision.snapshot else {
        panic!("the snapshot of {entity_type} is a JSON object");
    };
    let mut keys: Vec<&str> = stored.keys().map(String::as_str).collect();
    keys.sort_unstable();
    let mut listed = E::Snapshot::FIELDS.to_vec();
    listed.sort_unstable();
    assert_eq!(
        keys, listed,
        "the snapshot of {entity_type} holds exactly the allow-listed fields (ADR 0011 §4)"
    );
    let current = db_snapshot(db, entity, revision.entity_id).await;
    assert_eq!(
        revision.read_snapshot::<E::Snapshot>().as_ref(),
        Ok(&current),
        "the revision of {entity_type} is the record after the save (ADR 0011 §4)"
    );
    assert_eq!(
        Value::Object(snapshot_object(&current).expect("a valid snapshot")),
        revision.snapshot,
        "the stored snapshot of {entity_type} is the serialised record"
    );
}

/// Checks an entry against the revision its save wrote.
fn assert_entry<E: SafetyNetEntity>(
    entity: &E,
    actor: Actor,
    entry: &activity_log::Model,
    revision: &revisions::Model,
    action: Action,
    current: &E::Snapshot,
) {
    let entity_type = E::Snapshot::ENTITY_TYPE;
    assert_eq!(
        entry.action(),
        Ok(action),
        "the entry of the {entity_type} save"
    );
    assert_eq!(
        entry.actor(),
        Ok(actor),
        "the entry of the {entity_type} save"
    );
    assert_eq!(
        entry.revision_id,
        Some(revision.id),
        "the {action} entry of {entity_type} names the revision its save wrote (ADR 0011 §5)"
    );
    assert_eq!(
        entry.label,
        entity.label(current),
        "the label of the {action} entry of {entity_type}"
    );
    assert_eq!(
        entry.request_id, revision.request_id,
        "the revision and the entry of one save carry the same request id"
    );
    assert_eq!(
        entry.occurred_at, revision.created_at,
        "the revision and the entry of one save carry the same instant: they are written in one \
         transaction (ADR 0011 §5, D90)"
    );
}

/// The stored record's snapshot, through the entity.
async fn db_snapshot<E: SafetyNetEntity>(
    db: &DatabaseConnection,
    entity: &E,
    id: i64,
) -> E::Snapshot {
    entity.snapshot(db, id).await
}

/// The revisions of record `id` of `E`, oldest first.
async fn revisions_of<E: SafetyNetEntity>(
    db: &DatabaseConnection,
    id: i64,
) -> Vec<revisions::Model> {
    revisions::Model::for_record(db, E::Snapshot::ENTITY_TYPE, id)
        .await
        .expect("read the revisions")
}

/// The activity entries about record `id` of `E`, oldest first.
async fn entries_of<E: SafetyNetEntity>(
    db: &DatabaseConnection,
    id: i64,
) -> Vec<activity_log::Model> {
    activity_log::Entity::find()
        .filter(activity_log::Column::EntityType.eq(E::Snapshot::ENTITY_TYPE))
        .filter(activity_log::Column::EntityId.eq(id))
        .order_by_asc(activity_log::Column::Id)
        .all(db)
        .await
        .expect("read the entries")
}

/// The one entry about record `id` written since it had `before` entries.
async fn only_new_entry<E: SafetyNetEntity>(
    db: &DatabaseConnection,
    id: i64,
    before: usize,
) -> activity_log::Model {
    let entity_type = E::Snapshot::ENTITY_TYPE;
    let mut entries = entries_of::<E>(db, id).await;
    assert_eq!(
        entries.len(),
        before + 1,
        "a save of {entity_type} {id} writes exactly one activity entry about it (ADR 0011 §5)"
    );
    entries.pop().expect("the new entry")
}

/// The record's row as one text, to compare before and after a failed save.
async fn record_text<E: SafetyNetEntity>(db: &DatabaseConnection, id: i64) -> String {
    let table = E::Snapshot::ENTITY_TYPE;
    scalar(
        db,
        Statement::from_sql_and_values(
            DbBackend::Postgres,
            format!("SELECT {table}::text FROM {table} WHERE id = $1"),
            [id.into()],
        ),
    )
    .await
}

/// How many triggers named like the refusing one exist.
async fn trigger_count(db: &DatabaseConnection) -> i64 {
    scalar(
        db,
        Statement::from_string(
            DbBackend::Postgres,
            "SELECT count(*) FROM pg_trigger WHERE tgname = 'safety_net_refuse_entry'",
        ),
    )
    .await
}

/// The one value that `statement` returns.
async fn scalar<T: sea_orm::TryGetable>(db: &DatabaseConnection, statement: Statement) -> T {
    let sql = statement.sql.clone();
    db.query_one_raw(statement)
        .await
        .expect(&sql)
        .expect("one row")
        .try_get_by_index(0)
        .expect(&sql)
}

/// The audit columns of one record (ADR 0011 §2).
struct Audit {
    created_at: DateTimeWithTimeZone,
    created_by: Option<i64>,
    updated_at: DateTimeWithTimeZone,
    updated_by: Option<i64>,
}

impl Audit {
    async fn read<E: SafetyNetEntity>(db: &DatabaseConnection, id: i64) -> Self {
        let table = E::Snapshot::ENTITY_TYPE;
        let row = db
            .query_one_raw(Statement::from_sql_and_values(
                DbBackend::Postgres,
                format!(
                    "SELECT created_at, created_by, updated_at, updated_by FROM {table} \
                     WHERE id = $1"
                ),
                [id.into()],
            ))
            .await
            .expect("read the audit columns")
            .unwrap_or_else(|| panic!("{table} {id} exists"));
        Self {
            created_at: row.try_get_by_index(0).expect("created_at"),
            created_by: row.try_get_by_index(1).expect("created_by"),
            updated_at: row.try_get_by_index(2).expect("updated_at"),
            updated_by: row.try_get_by_index(3).expect("updated_by"),
        }
    }
}

/// The number of rows of the entity's table, of `revisions` and of
/// `activity_log`, to check that a save wrote exactly its own rows.
struct Totals {
    records: i64,
    revisions: i64,
    entries: i64,
}

impl Totals {
    async fn read<E: SafetyNetEntity>(db: &DatabaseConnection) -> Self {
        let count = |table: &str| {
            Statement::from_string(DbBackend::Postgres, format!("SELECT count(*) FROM {table}"))
        };
        Self {
            records: scalar(db, count(E::Snapshot::ENTITY_TYPE)).await,
            revisions: scalar(db, count("revisions")).await,
            entries: scalar(db, count("activity_log")).await,
        }
    }

    /// Checks that the entity's table grew by `records` rows, and
    /// `revisions` and `activity_log` each by `saves` rows.
    async fn assert_grew<E: SafetyNetEntity>(
        &self,
        db: &DatabaseConnection,
        records: i64,
        saves: i64,
    ) {
        let now = Self::read::<E>(db).await;
        assert_eq!(
            (
                now.records - self.records,
                now.revisions - self.revisions,
                now.entries - self.entries
            ),
            (records, saves, saves),
            "a save of {} writes exactly {records} new record(s), {saves} revision(s) and \
             {saves} activity entr(y/ies) in all (the counts: new records, revisions, entries)",
            E::Snapshot::ENTITY_TYPE
        );
    }
}
