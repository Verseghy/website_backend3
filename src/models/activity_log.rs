//! The activity log (ADR 0011 §5): [`record`] appends one entry, and
//! [`with_permanent_delete_flag`] marks the one kind of transaction in which
//! an entry may change at all.
//!
//! The table is append-only. Its triggers refuse every `DELETE` and
//! `TRUNCATE`, and every `UPDATE` except one that only sets `ip` to NULL
//! inside a permanent-delete transaction (the migration
//! `m20261031_223930_activity_log` has the details). So nothing here updates
//! or deletes an entry, and [`record`] checks an entry against the rules of
//! [`crate::domain::activity::check_entry`] before it inserts it: a mistake
//! that reached the table could never be corrected.
//!
//! - **The actor is an argument** (ADR 0011 §2): a user id, or the system or
//!   the importer. There is no "current user" anywhere (ADR 0008).
//! - **The same transaction as the change** (ADR 0011 §5): an entry for a
//!   data change is recorded on the transaction that makes the change, so a
//!   failed insert of the entry rolls the change back, and a rolled-back
//!   change leaves no entry. A failed sign-in changes no data and is recorded
//!   on a connection of its own. [`record`] takes any connection for that
//!   reason.
//! - **The instant comes from PostgreSQL's clock** (research: DECISIONS R15,
//!   D90): [`record`] leaves `occurred_at` to its `CURRENT_TIMESTAMP`
//!   default, the start of the transaction, which is also the instant that
//!   the change's own audit columns get from their defaults and from
//!   `Expr::current_timestamp()`.
//! - **The request id is passed in**, in [`Entry::request_id`]: the controller
//!   takes it from Loco's `LocoRequestId` request extension, so the entries
//!   of one request (a week-editor save, a bulk action) can be grouped.
//!
//! It holds the client address as `ipnetwork::IpNetwork`, the type Sea-ORM's
//! `with-ipnetwork` feature maps `inet` to; callers pass a plain
//! [`IpAddr`].

use std::net::IpAddr;

use loco_rs::model::{ModelError, ModelResult};
use sea_orm::{
    ActiveModelTrait, ActiveValue, ConnectionTrait, DatabaseTransaction,
    entity::prelude::{ActiveModelBehavior, IpNetwork},
};
use serde_json::{Map, Value};

pub use super::_entities::activity_log::{self, ActiveModel, Column, Entity, Model};
use crate::domain::activity::{
    Action, Actor, EntryShape, StoredActorError, UnknownAction, check_entry,
};

/// The transaction-local setting that marks a permanent-delete transaction,
/// which [`with_permanent_delete_flag`] sets to `on` and the table's `UPDATE`
/// trigger reads (ADR 0011 §5). The migration spells the same name in its
/// trigger function; the model tests fail if the two differ.
pub const PERMANENT_DELETE_FLAG: &str = "backend3.permanent_delete";

/// No save hooks: an entry is only ever inserted, by [`record`], and the
/// database fills in its id and instant. The table has no `updated_at`, so
/// the `before_save` that `cargo loco db entities` generated only returned
/// the model unchanged, and is left out.
impl ActiveModelBehavior for ActiveModel {}

/// The record or table an entry concerns (`entity_type`, `entity_id`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EntityRef {
    /// The resource name, the record's table, for example `posts`
    /// (ADR 0011 §4). A constant of the calling code, never client input.
    pub entity_type: &'static str,
    /// The record's id, or `None` for an entry about a whole table, such as
    /// the importer's one entry per table (ADR 0011 §5).
    pub entity_id: Option<i64>,
}

impl EntityRef {
    /// One record: `entity_type` and its `id`.
    ///
    /// - `entity_type`: the resource name, for example `posts`.
    /// - `id`: the record's id.
    #[must_use]
    pub const fn record(entity_type: &'static str, id: i64) -> Self {
        Self {
            entity_type,
            entity_id: Some(id),
        }
    }

    /// A whole table, without a record id.
    ///
    /// - `entity_type`: the resource name, for example `labels`.
    #[must_use]
    pub const fn table(entity_type: &'static str) -> Self {
        Self {
            entity_type,
            entity_id: None,
        }
    }
}

/// One activity entry before it is written: everything except the actor,
/// which [`record`] takes on its own, and the instant and id, which the
/// database sets.
///
/// Build it from [`Entry::new`] with struct-update syntax, setting only the
/// parts the action has:
///
/// ```
/// use backend3::{
///     domain::activity::{Action, changed_fields_details},
///     models::activity_log::{EntityRef, Entry},
/// };
///
/// let entry = Entry {
///     entity: Some(EntityRef::record("labels", 7)),
///     label: Some("Hírek".to_owned()),
///     details: Some(changed_fields_details(&["color".to_owned()])),
///     request_id: Some("4bf92f35-request".to_owned()),
///     ..Entry::new(Action::Update)
/// };
/// assert_eq!(entry.action, Action::Update);
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    /// What happened.
    pub action: Action,
    /// The record or table concerned, if any.
    pub entity: Option<EntityRef>,
    /// The revision the action wrote, if any (ADR 0011 §4); it needs a
    /// record in [`Entry::entity`].
    pub revision_id: Option<i64>,
    /// A display label for a content record (a post or page title, a label
    /// or dish name, ...), so the log stays readable after a purge. Set only
    /// with [`Entry::entity`], never on a sign-in or account entry, never
    /// for people (users, authors, colleagues), whose entries keep only the
    /// id, and never blank (ADR 0011 §5). A media operation names its paths
    /// in [`Entry::details`] instead.
    pub label: Option<String>,
    /// Facts about the action as a JSON object: the changed field names of an
    /// update ([`crate::domain::activity::changed_fields_details`]), the old
    /// and new path and the size of a media operation, the counts of a batch
    /// action (ADR 0011 §5). Never the changed values.
    pub details: Option<Map<String, Value>>,
    /// The request id that groups the entries of one request: Loco's
    /// `LocoRequestId`, 1 to 255 characters. `None` outside a request (tasks,
    /// queue jobs, the importer). Loco keeps an incoming `x-request-id`
    /// header as the id (without the characters outside letters, digits,
    /// `_`, `-` and `@`), so the text is the caller's own; recipe 8 of the
    /// project skill says what that means for an entry written before the
    /// caller has signed in.
    pub request_id: Option<String>,
    /// The client address, only for sign-in and account events (ADR 0011
    /// §5): the address Loco's `remote_ip` middleware resolved.
    pub ip: Option<IpAddr>,
}

impl Entry {
    /// An entry of `action` with no other part set.
    ///
    /// - `action`: what happened.
    #[must_use]
    pub const fn new(action: Action) -> Self {
        Self {
            action,
            entity: None,
            revision_id: None,
            label: None,
            details: None,
            request_id: None,
            ip: None,
        }
    }

    /// The parts of the entry that [`check_entry`] reads.
    fn shape(&self) -> EntryShape<'_> {
        EntryShape {
            action: self.action,
            entity_type: self.entity.map(|entity| entity.entity_type),
            has_entity_id: self.entity.is_some_and(|entity| entity.entity_id.is_some()),
            has_revision_id: self.revision_id.is_some(),
            label: self.label.as_deref(),
            request_id: self.request_id.as_deref(),
            has_client_address: self.ip.is_some(),
        }
    }
}

/// Appends one entry to the activity log and returns the stored row.
///
/// - `db`: the transaction of the change the entry records, so that both
///   commit or roll back together (ADR 0011 §5). Only an entry that records
///   no data change, such as a failed sign-in, is written on a connection of
///   its own.
/// - `actor`: who did it; [`Actor::User`] needs the id of an existing
///   account.
/// - `entry`: what happened, to what, in which request.
///
/// `occurred_at` is left to the column default, the start of the
/// transaction on PostgreSQL's clock (research: DECISIONS R15, D90); the
/// returned row holds it.
///
/// # Errors
///
/// - [`ModelError::Any`] holding a
///   [`crate::domain::activity::EntryError`] when the entry breaks a
///   rule of [`check_entry`]; nothing is written then.
/// - [`ModelError::DbErr`] when the insert fails, for example for a user id
///   that names no account (`activity_log_actor_id_fkey`). The table's
///   CHECKs repeat the rules of [`check_entry`] that keep personal data out,
///   but an entry that passes [`check_entry`] also passes them.
pub async fn record<C>(db: &C, actor: Actor, entry: Entry) -> ModelResult<Model>
where
    C: ConnectionTrait,
{
    check_entry(&entry.shape()).map_err(ModelError::wrap)?;
    let entity = entry.entity;
    let row = ActiveModel {
        actor_id: ActiveValue::Set(actor.user_id()),
        actor_kind: ActiveValue::Set(actor.kind().to_owned()),
        action: ActiveValue::Set(entry.action.as_str().to_owned()),
        entity_type: ActiveValue::Set(entity.map(|entity| entity.entity_type.to_owned())),
        entity_id: ActiveValue::Set(entity.and_then(|entity| entity.entity_id)),
        revision_id: ActiveValue::Set(entry.revision_id),
        label: ActiveValue::Set(entry.label),
        details: ActiveValue::Set(entry.details.map(Value::Object)),
        request_id: ActiveValue::Set(entry.request_id),
        // A single address becomes a host network (/32 or /128): `inet`
        // stores an address with its prefix length, and `host(ip)` gives the
        // address back.
        ip: ActiveValue::Set(entry.ip.map(IpNetwork::from)),
        // `id` and `occurred_at` stay unset, so the INSERT leaves them out
        // and the database fills them in.
        ..Default::default()
    };
    Ok(row.insert(db).await?)
}

/// Marks `txn` as a permanent-delete transaction (ADR 0011 §5, §6), the
/// only kind in which the table's trigger lets an `UPDATE` through, and then
/// only one that sets `ip` to NULL.
///
/// - `txn`: the outermost transaction of the permanent delete. A
///   transaction, not a connection: the flag is set with `SET LOCAL`, so it
///   ends with the transaction, also when it rolls back, and never stays on
///   the pooled connection. (Outside a transaction block PostgreSQL would
///   ignore `SET LOCAL` with a warning.)
///
/// **Set it on the outermost transaction.** A Sea-ORM nested transaction
/// (`txn.begin()`) is a savepoint, and PostgreSQL ties a `SET LOCAL` to the
/// whole transaction, not to the savepoint: committing the nested
/// transaction (`RELEASE SAVEPOINT`) keeps the flag on until the outermost
/// transaction ends, so the work after it in that transaction runs with the
/// flag too. Only rolling the nested transaction back undoes it. The model
/// test `a_flag_set_in_a_savepoint_lasts_until_the_outer_transaction_ends`
/// pins this.
///
/// Only the anonymisation of a user calls this (ADR 0011 §6), which clears
/// the address of that user's own sign-in and account entries. The flag
/// does not let `DELETE` or `TRUNCATE` through.
///
/// # Errors
///
/// Returns [`ModelError::DbErr`] when the statement fails.
pub async fn with_permanent_delete_flag(txn: &DatabaseTransaction) -> ModelResult<()> {
    txn.execute_unprepared(&format!("SET LOCAL {PERMANENT_DELETE_FLAG} = 'on'"))
        .await?;
    Ok(())
}

impl Model {
    /// Who did it, read back from `actor_kind` and `actor_id`.
    ///
    /// # Errors
    ///
    /// Returns [`StoredActorError`] for columns that name no actor, which
    /// the table's CHECKs never let through.
    pub fn actor(&self) -> Result<Actor, StoredActorError> {
        Actor::from_stored(&self.actor_kind, self.actor_id)
    }

    /// What happened, read back from `action`.
    ///
    /// # Errors
    ///
    /// Returns [`UnknownAction`] for a name that is not an [`Action`], which
    /// the table's CHECK never lets through.
    pub fn action(&self) -> Result<Action, UnknownAction> {
        self.action.parse()
    }

    /// The client address the entry holds, if any.
    #[must_use]
    pub fn client_address(&self) -> Option<IpAddr> {
        self.ip.map(|network| network.ip())
    }
}
