//! Revisions (ADR 0011 §4): [`append`] writes the next revision of a record,
//! and [`record_save`] writes a save's revision together with the activity
//! entry that records it, the pair that every create, update and revert of
//! an admin-editable entity writes in its own transaction (ADR 0011 §7).
//! [`delete_for_record`] removes a record's revisions in a permanent delete
//! (ADR 0011 §6).
//!
//! The table is append-only. Its triggers refuse every `UPDATE` and
//! `TRUNCATE`, and every `DELETE` outside a permanent-delete transaction
//! (the migration `m20261101_012003_revisions` has the details), so nothing
//! here updates a revision, and only [`delete_for_record`] deletes any. A
//! revision could never be corrected, so every rule it must meet is checked
//! before the insert, by the pure functions of [`crate::domain::revision`].
//!
//! - **The snapshot is the entity's revision snapshot type**, an allow-list
//!   ([`RevisionSnapshot`]), never the entity's model. Its constants give
//!   the record's table (`entity_type`) and the `schema_version`.
//! - **The actor is an argument** (ADR 0011 §2): a user, the system, or the
//!   importer, the only actor of the `import` and `import_original`
//!   revisions. `created_by` is the user's id, NULL for the other two.
//! - **The same transaction as the record's write.** Both functions take a
//!   transaction, never a pool: the revision commits or rolls back with the
//!   save, and the row lock below lasts until the save ends.
//! - **The next number is taken under the record's row lock.** Before it
//!   reads the latest number, [`append`] locks the record's own row with
//!   `SELECT ... FOR NO KEY UPDATE`, the lock that the save's own `UPDATE`
//!   of the row takes, and takes again for nothing when the save already
//!   holds it. A concurrent save of the same record waits for it until this
//!   transaction ends. Its next statement then sees the committed revision
//!   (READ COMMITTED, the isolation level backend3 runs at, takes a new
//!   snapshot for every statement), so it takes the number after it. Under
//!   REPEATABLE READ or SERIALIZABLE the waiting save fails instead. When
//!   the first save updated the row, the lock fails with a serialization
//!   error. When the first save only locked it (an `append` without an
//!   `UPDATE`, as the importer writes), the waiting save gets the lock,
//!   reads the latest number from its older snapshot, and its insert is
//!   refused by the unique index on `(entity_type, entity_id, revision_no
//!   DESC)`, which refuses a repeated number in any case. The model test
//!   `a_waiting_save_under_repeatable_read_fails_instead_of_repeating_a_number`
//!   pins both. A row lock lives in the row itself, so a transaction that
//!   writes many records (the importer) holds no entry of PostgreSQL's
//!   shared lock table for them, as an advisory lock per record would; and
//!   the lock proves that the record exists, which no foreign key can,
//!   because one table holds the revisions of every entity.
//! - **A permanent delete takes the same lock before it deletes.** A save
//!   that is still running may have inserted a revision that no other
//!   transaction can see yet, so a `DELETE` of the record's revisions on
//!   its own would miss it, and that revision would survive the purge or the
//!   anonymisation. [`delete_for_record`] waits for the row lock first.
//! - **The instant comes from PostgreSQL's clock** (research: DECISIONS R15,
//!   D90): `created_at` is left to its `CURRENT_TIMESTAMP` default, the start
//!   of the transaction, which the save's audit columns and its activity
//!   entry get too.

use loco_rs::model::{ModelError, ModelResult};
use sea_orm::{
    ActiveModelTrait, ActiveValue, ColumnTrait, ConnectionTrait, DatabaseTransaction, DbBackend,
    EntityTrait, QueryFilter, QueryOrder, QuerySelect, Statement,
    entity::prelude::ActiveModelBehavior,
};
use serde_json::{Map, Value};

pub use super::_entities::revisions::{self, ActiveModel, Column, Entity, Model};
use super::activity_log::{self, EntityRef, Entry};
use crate::domain::{
    activity::Actor,
    revision::{
        Reason, RevisionError, RevisionSnapshot, SnapshotObject, UnknownReason, UpcastError,
        check_revision, check_snapshot_type, comparable_snapshot, next_revision_no, read_snapshot,
        save_entry_details, snapshot_object,
    },
};

/// No save hooks: a revision is only ever inserted, by [`append`] or
/// [`record_save`], and the database fills in its id and instant. The table
/// has no `updated_at`, so the `before_save` that `cargo loco db entities`
/// generated only returned the model unchanged, and is left out.
impl ActiveModelBehavior for ActiveModel {}

/// A revision of a record that does not exist, or that this transaction
/// cannot see: a record is written before its revision (ADR 0011 §7).
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error(
    "there is no {entity_type} row with id {entity_id} to write a revision of; write the record \
     first, in the same transaction (ADR 0011 §7)"
)]
pub struct MissingRecord {
    /// The record's table.
    pub entity_type: &'static str,
    /// The id that names no row.
    pub entity_id: i64,
}

/// Why a stored revision cannot be read as a snapshot type.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ReadSnapshotError {
    /// The revision belongs to another entity than the snapshot type.
    #[error(
        "revision {id} belongs to {stored}, not to {expected}, the entity of the snapshot type"
    )]
    WrongEntityType {
        /// The revision's id.
        id: i64,
        /// Its stored `entity_type`.
        stored: String,
        /// The snapshot type's entity type.
        expected: &'static str,
    },
    /// The stored snapshot cannot be upcast or read.
    #[error(transparent)]
    Upcast(#[from] UpcastError),
}

/// Appends the next revision of record `entity_id` and returns the stored
/// row.
///
/// - `txn`: the transaction that wrote the record (ADR 0011 §7). The
///   record's row is locked until it ends, so concurrent saves of one record
///   take consecutive numbers (see the module documentation).
/// - `actor`: who saved it; an import reason needs [`Actor::Import`], and
///   [`Actor::User`] the id of an existing account.
/// - `entity_id`: the record's id in the table that `S::ENTITY_TYPE` names.
/// - `reason`: why the revision is written.
/// - `snapshot`: the record after the write, as its snapshot type.
/// - `request_id`: Loco's request id, to group a request's writes; `None`
///   outside a request.
///
/// Use it on its own for the importer's revisions, which the importer's
/// one entry per table records (ADR 0011 §5). A save that the activity log
/// records per record goes through [`record_save`] instead.
///
/// # Errors
///
/// - [`ModelError::Any`] holding a
///   [`crate::domain::revision::SnapshotError`] for a snapshot type or value
///   that breaks a rule of [`RevisionSnapshot`];
/// - [`ModelError::Any`] holding a [`MissingRecord`] when the record's row
///   does not exist;
/// - [`ModelError::Any`] holding a [`RevisionError`] when the revision
///   breaks a rule of [`check_revision`] or the record has run out of
///   numbers;
/// - [`ModelError::DbErr`] when a statement fails, for example for a user id
///   that names no account (`revisions_created_by_fkey`).
///
/// Nothing is written then; a database error leaves `txn` for the caller to
/// roll back.
pub async fn append<S: RevisionSnapshot>(
    txn: &DatabaseTransaction,
    actor: Actor,
    entity_id: i64,
    reason: Reason,
    snapshot: &S,
    request_id: Option<&str>,
) -> ModelResult<Model> {
    let object = snapshot_object(snapshot).map_err(ModelError::wrap)?;
    lock_record::<S>(txn, entity_id).await?;
    let latest = latest_number_and_reason::<S>(txn, entity_id).await?;
    insert_next::<S>(txn, actor, entity_id, reason, object, latest, request_id).await
}

/// One save of a record under the safety net (ADR 0011 §4, §5, §7): what
/// [`record_save`] writes as the revision and as its activity entry.
///
/// Build it from [`Save::new`] with struct-update syntax, setting only the
/// parts the save has.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Save<'a, S> {
    /// The saved record's id, in the table that `S::ENTITY_TYPE` names.
    pub entity_id: i64,
    /// Why: [`Reason::Create`], [`Reason::Update`] or [`Reason::Revert`].
    pub reason: Reason,
    /// The record after the write, as its snapshot type.
    pub snapshot: &'a S,
    /// The record's display label for the activity entry (its title or
    /// name), only for a content record: never for `users`, `authors` or
    /// `colleagues`, which are people (ADR 0011 §5, recipe 8 of the project
    /// skill).
    pub label: Option<String>,
    /// Facts about the save beyond its changed field names, as a JSON
    /// object, for example the source page of a clone (ADR 0011 §5). Never
    /// under the key `changed_fields`, which the entry's changed field names
    /// take.
    pub details: Option<Map<String, Value>>,
    /// Loco's request id, written on both the revision and the entry; `None`
    /// outside a request.
    pub request_id: Option<String>,
}

impl<'a, S> Save<'a, S> {
    /// A save of `reason` of record `entity_id`, with no label, no extra
    /// details and no request id.
    ///
    /// - `entity_id`: the saved record's id.
    /// - `reason`: why it was saved.
    /// - `snapshot`: the record after the write.
    #[must_use]
    pub const fn new(entity_id: i64, reason: Reason, snapshot: &'a S) -> Self {
        Self {
            entity_id,
            reason,
            snapshot,
            label: None,
            details: None,
            request_id: None,
        }
    }
}

/// What [`record_save`] wrote.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Saved {
    /// The new revision.
    pub revision: Model,
    /// The activity entry that records the save; its `revision_id` is the
    /// new revision's id.
    pub entry: activity_log::Model,
}

/// Writes the revision of a save and the activity entry that records it, in
/// `txn`, and returns both (ADR 0011 §4, §5, §7). Every create, update and
/// revert of an admin-editable entity calls it in the transaction that
/// writes the record.
///
/// - `txn`: the transaction that wrote the record.
/// - `actor`: who saved it: a user, or the system ([`Actor::Import`] writes
///   no saves).
/// - `save`: the record, the reason and what the entry holds.
///
/// The entry's action is the reason's ([`Reason::save_action`]); it names
/// the record and the new revision's id. A `create` entry holds the extra
/// details. An `update` or `revert` entry holds the names of the fields that
/// changed since the record's latest revision, read in the current shape
/// ([`comparable_snapshot`]), beside the extra details: the names, never the
/// values ([`save_entry_details`]).
///
/// When the entry cannot be written, the error is returned and `txn` is
/// left with the revision and the record's write in it: the caller rolls it
/// back, so a failed entry fails the whole save (ADR 0011, "A failure while
/// inserting the activity entry rolls back the record change").
///
/// **A latest revision that cannot be upcast fails the save too**, on
/// purpose, although the changed field names are only details of the
/// entry. The two cases differ from the newer snapshot that
/// [`comparable_snapshot`] compares as stored. A newer version is an
/// expected state after a rollback to an older release. A failed upcast is a
/// mistake in this release's code: an upcaster step that refuses a shape
/// its version had, or a snapshot type whose shape changed without a new
/// `SCHEMA_VERSION`. Falling back to "every field changed" would write an
/// entry with a false list of changed fields into a table that can never
/// correct it. It would also hide the mistake, which breaks the history
/// panel and the revert of that revision as well. The save fails with the
/// step's error instead, the editor's change is not stored, and the error
/// names the entity, the version and the field. The fix is a corrected step
/// in a release; the stored revision stays as it is. Saving the record
/// works again from that release on.
///
/// # Errors
///
/// - every error of [`append`];
/// - [`ModelError::Any`] holding a [`RevisionError::NotASave`] for an
///   import reason, or [`RevisionError::ReservedDetailKey`] for extra
///   details under `changed_fields`;
/// - [`ModelError::Any`] holding an [`UpcastError`] when the latest earlier
///   revision cannot be upcast (above);
/// - every error of [`activity_log::record`], for example a label on an
///   entry about a person.
pub async fn record_save<S: RevisionSnapshot>(
    txn: &DatabaseTransaction,
    actor: Actor,
    save: Save<'_, S>,
) -> ModelResult<Saved> {
    let Save {
        entity_id,
        reason,
        snapshot,
        label,
        details,
        request_id,
    } = save;
    let Some(action) = reason.save_action() else {
        return Err(ModelError::wrap(RevisionError::NotASave { reason }));
    };
    let after = snapshot_object(snapshot).map_err(ModelError::wrap)?;
    lock_record::<S>(txn, entity_id).await?;
    let latest = Entity::find()
        .filter(Column::EntityType.eq(S::ENTITY_TYPE))
        .filter(Column::EntityId.eq(entity_id))
        .order_by_desc(Column::RevisionNo)
        .one(txn)
        .await?;
    let before = latest
        .as_ref()
        .map(|previous| {
            comparable_snapshot::<S>(previous.schema_version, previous.snapshot.clone())
        })
        .transpose()
        .map_err(ModelError::wrap)?;
    let details =
        save_entry_details(reason, before.as_ref(), &after, details).map_err(ModelError::wrap)?;
    let latest = latest
        .map(|previous| Ok::<_, ModelError>((previous.revision_no, stored_reason(&previous)?)))
        .transpose()?;
    let revision = insert_next::<S>(
        txn,
        actor,
        entity_id,
        reason,
        after,
        latest,
        request_id.as_deref(),
    )
    .await?;
    let entry = activity_log::record(
        txn,
        actor,
        Entry {
            entity: Some(EntityRef::record(S::ENTITY_TYPE, entity_id)),
            revision_id: Some(revision.id),
            label,
            details,
            request_id,
            ..Entry::new(action)
        },
    )
    .await?;
    Ok(Saved { revision, entry })
}

/// Deletes every revision of record `entity_id` in the table of `S`, as
/// part of a permanent delete (ADR 0011 §6), and returns how many it
/// deleted.
///
/// - `txn`: the outermost transaction of the permanent delete, which has
///   called [`activity_log::with_permanent_delete_flag`]. Without the flag
///   the table's trigger refuses the `DELETE`.
/// - `entity_id`: the record's id in the table that `S::ENTITY_TYPE` names.
///
/// The two permanent deletes that remove revisions call it. A purge removes
/// the revisions of the purged record. The anonymisation of a user removes
/// those of the account record, because they hold the name and e-mail
/// address (`S` is then the users snapshot type).
///
/// **It locks the record's row before it deletes**, with the lock that
/// [`append`] takes. A save of the record that is still running holds that
/// lock, and its revision may already be inserted but not yet visible to
/// this transaction. A `DELETE` alone would not see that revision and would
/// not wait for it, so the revision would outlive the purge, or keep the
/// personal data that the anonymisation removes. Waiting for the lock lets
/// that save end first. The `DELETE` then runs as a new statement, so at
/// READ COMMITTED it sees the save's revision. From then on no save of the
/// record can add a revision until this transaction ends.
///
/// The row may already be gone. This transaction may have deleted it
/// itself, as a purge does, and then holds its lock already. Or another
/// transaction deleted it and committed: no save can add a revision of it
/// after that, because [`append`] refuses a row that does not exist. In
/// both cases nothing is locked and the revisions are deleted.
///
/// Call it after the permanent delete has written the record (the purge's
/// `DELETE` of the row, the anonymisation's `UPDATE` of the account). Only
/// that write tells whether the record may be removed at all (for example
/// that it is still in the trash after a wait), and it takes the same lock.
///
/// # Errors
///
/// - [`ModelError::Any`] holding a
///   [`crate::domain::revision::SnapshotTypeError`] for a snapshot type that
///   breaks a rule of [`RevisionSnapshot`]. Nothing is locked or deleted
///   then.
/// - [`ModelError::DbErr`] when a statement fails, for example outside a
///   permanent-delete transaction (`restrict_violation`, "DELETE is allowed
///   only in a permanent-delete transaction"). That leaves `txn` for the
///   caller to roll back.
pub async fn delete_for_record<S: RevisionSnapshot>(
    txn: &DatabaseTransaction,
    entity_id: i64,
) -> ModelResult<u64> {
    // The table name goes into the lock's SQL, so its check comes first.
    check_snapshot_type::<S>().map_err(ModelError::wrap)?;
    // Whether the row is still there does not matter here (above).
    lock_row::<S>(txn, entity_id).await?;
    let deleted = Entity::delete_many()
        .filter(Column::EntityType.eq(S::ENTITY_TYPE))
        .filter(Column::EntityId.eq(entity_id))
        .exec(txn)
        .await?;
    Ok(deleted.rows_affected)
}

/// Locks the row of record `entity_id` in the table of `S` for the rest of
/// `txn`, or refuses a record that does not exist ([`lock_row`]).
async fn lock_record<S: RevisionSnapshot>(
    txn: &DatabaseTransaction,
    entity_id: i64,
) -> ModelResult<()> {
    if !lock_row::<S>(txn, entity_id).await? {
        return Err(ModelError::wrap(MissingRecord {
            entity_type: S::ENTITY_TYPE,
            entity_id,
        }));
    }
    Ok(())
}

/// Locks the row of record `entity_id` in the table of `S` for the rest of
/// `txn`, with the lock an `UPDATE` of its non-key columns takes, and says
/// whether this transaction sees the row. It waits while another
/// transaction holds a conflicting lock, and then reads the row as that
/// transaction left it.
///
/// `S::ENTITY_TYPE` is written into the statement as the table name. It is
/// a constant of the snapshot type, and every caller has checked that it is
/// `snake_case` (lowercase ASCII letters, digits and underscores) first,
/// through [`snapshot_object`] or [`check_snapshot_type`], so it cannot
/// carry SQL.
async fn lock_row<S: RevisionSnapshot>(
    txn: &DatabaseTransaction,
    entity_id: i64,
) -> ModelResult<bool> {
    let sql = format!(
        "SELECT 1 FROM {} WHERE id = $1 FOR NO KEY UPDATE",
        S::ENTITY_TYPE
    );
    let statement = Statement::from_sql_and_values(DbBackend::Postgres, sql, [entity_id.into()]);
    Ok(txn.query_one_raw(statement).await?.is_some())
}

/// The number and reason of the latest revision of record `entity_id`,
/// without its snapshot, which [`append`] does not need.
async fn latest_number_and_reason<S: RevisionSnapshot>(
    txn: &DatabaseTransaction,
    entity_id: i64,
) -> ModelResult<Option<(i32, Reason)>> {
    let latest: Option<(i32, String)> = Entity::find()
        .select_only()
        .columns([Column::RevisionNo, Column::Reason])
        .filter(Column::EntityType.eq(S::ENTITY_TYPE))
        .filter(Column::EntityId.eq(entity_id))
        .order_by_desc(Column::RevisionNo)
        .into_tuple()
        .one(txn)
        .await?;
    latest
        .map(|(number, reason)| Ok((number, reason.parse().map_err(ModelError::wrap)?)))
        .transpose()
}

/// The reason of a stored revision, as a [`ModelError`].
fn stored_reason(revision: &Model) -> ModelResult<Reason> {
    revision.reason().map_err(ModelError::wrap)
}

/// Checks the revision against the record's latest one and inserts it with
/// the next number.
///
/// Not named `insert`: the live-row rule of `tests/architecture.rs` knows a
/// model function by its name only, so a private `insert` that takes a
/// transaction would flag every `.insert(` call on a header map or a JSON
/// object in the public API modules.
async fn insert_next<S: RevisionSnapshot>(
    txn: &DatabaseTransaction,
    actor: Actor,
    entity_id: i64,
    reason: Reason,
    snapshot: SnapshotObject,
    latest: Option<(i32, Reason)>,
    request_id: Option<&str>,
) -> ModelResult<Model> {
    check_revision(
        actor,
        reason,
        latest.map(|(_, previous)| previous),
        request_id,
    )
    .map_err(ModelError::wrap)?;
    let revision_no =
        next_revision_no(latest.map(|(number, _)| number)).map_err(ModelError::wrap)?;
    let row = ActiveModel {
        entity_type: ActiveValue::Set(S::ENTITY_TYPE.to_owned()),
        entity_id: ActiveValue::Set(entity_id),
        revision_no: ActiveValue::Set(revision_no),
        reason: ActiveValue::Set(reason.as_str().to_owned()),
        schema_version: ActiveValue::Set(S::SCHEMA_VERSION),
        snapshot: ActiveValue::Set(Value::Object(snapshot)),
        created_by: ActiveValue::Set(actor.user_id()),
        request_id: ActiveValue::Set(request_id.map(str::to_owned)),
        // `id` and `created_at` stay unset, so the INSERT leaves them out
        // and the database fills them in.
        ..Default::default()
    };
    Ok(row.insert(txn).await?)
}

impl Model {
    /// The revisions of record `entity_id` of `entity_type`, oldest first.
    ///
    /// - `db`: any connection; inside a save's transaction it sees the
    ///   save's own revisions too.
    /// - `entity_type`: the record's table, for example `posts`.
    /// - `entity_id`: the record's id.
    ///
    /// # Errors
    ///
    /// Returns [`ModelError::DbErr`] when the query fails.
    pub async fn for_record<C: ConnectionTrait>(
        db: &C,
        entity_type: &str,
        entity_id: i64,
    ) -> ModelResult<Vec<Self>> {
        Ok(Entity::find()
            .filter(Column::EntityType.eq(entity_type))
            .filter(Column::EntityId.eq(entity_id))
            .order_by_asc(Column::RevisionNo)
            .all(db)
            .await?)
    }

    /// Why the revision was written, read back from `reason`.
    ///
    /// # Errors
    ///
    /// Returns [`UnknownReason`] for a name that is not a [`Reason`], which
    /// the table's CHECK never lets through.
    pub fn reason(&self) -> Result<Reason, UnknownReason> {
        self.reason.parse()
    }

    /// The snapshot as the current snapshot type `S`, upcast from the
    /// version it was stored with ([`read_snapshot`]): what the history's
    /// revert loads into the edit form (ADR 0011 §4, "Revert").
    ///
    /// # Errors
    ///
    /// Returns [`ReadSnapshotError::WrongEntityType`] for a revision of
    /// another entity than `S::ENTITY_TYPE`, and
    /// [`ReadSnapshotError::Upcast`] when the snapshot cannot be upcast or
    /// does not fit `S`.
    pub fn read_snapshot<S: RevisionSnapshot>(&self) -> Result<S, ReadSnapshotError> {
        if self.entity_type != S::ENTITY_TYPE {
            return Err(ReadSnapshotError::WrongEntityType {
                id: self.id,
                stored: self.entity_type.clone(),
                expected: S::ENTITY_TYPE,
            });
        }
        Ok(read_snapshot::<S>(
            self.schema_version,
            self.snapshot.clone(),
        )?)
    }
}
