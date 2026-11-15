//! Labels, „címkék” (parity LBL-01, LBL-02), on top of the generated
//! `labels` entity: the live-row finders that every read of labels goes
//! through (ADR 0011 §3), and the create and the edit, which write a label
//! under the safety net (ADR 0011 §7).
//!
//! A label in the trash has `deleted_at` set. [`Model::find_live`] is the one
//! place that leaves such rows out, and every other finder here starts from
//! it, so no read of live labels can forget the filter. Its name, like every
//! finder built on it, has the word `live`: the architecture test lets the
//! public API call only such finders, and fails on a finder of that name
//! that does not filter the trash (`tests/architecture.rs`; recipe 2 step 4
//! of the project skill). The admin list reads live rows too; the trash
//! filter comes with the trash (ADR 0011 §8).
//!
//! **The writes** ([`Model::create`], [`ActiveModel::edit`]) take the
//! transaction, the actor and a checked
//! [`crate::domain::labels::LabelInput`], the only value they write, so a
//! save can set the name and the colour and nothing else (parity SHR-10).
//! Each writes the row, then the label's next revision
//! ([`LabelRevision`], built from the stored row) and the `create` or
//! `update` entry that names it and, for an update, its changed fields,
//! through `revisions::record_save`, in the caller's transaction (ADR 0011
//! §4, §5, §7): a failure of any part fails the save, and the caller's
//! rollback leaves nothing behind. Delete, restore and purge come with the
//! trash.
//!
//! **The audit columns** (ADR 0011 §2): `created_by` and `updated_by` are
//! the actor's user id, NULL for the system; the instants are PostgreSQL's
//! (research: DECISIONS R15, D90): an insert leaves `created_at` and
//! `updated_at` to their `CURRENT_TIMESTAMP` defaults, and an edit sets
//! `updated_at` to `Expr::current_timestamp()`, so the row, its revision and
//! its entry carry one instant, the start of the transaction. The hook below
//! therefore stamps nothing.
//!
//! **A name that a live label already has** is refused by the partial
//! unique index [`LIVE_NAME_INDEX`] on `lower(name)` among live rows
//! (ADR 0009, "Constraints"; ADR 0011 §3), never by a lookup that a
//! concurrent save could race: the write's refusal becomes
//! [`ModelError::EntityAlreadyExists`] (`super::unique_index`), which the
//! handler answers `409 label.name_taken`. A trashed label's name is free,
//! and a label may change the case of its own name.
//!
//! **The seed's labels** come from [`Model::seed_labels`], the one write
//! outside the safety net: the development, test and E2E seed's invented
//! labels (`crate::domain::seed::SEED_LABELS`), which the E2E suite lists.
//! Like the seed account, it writes no revision and no activity entry: only
//! local databases ever hold those rows (ADR 0040).
//!
//! The entity derives no serde (`[package.metadata.db.entity]` in
//! `Cargo.toml`): a label goes out only through
//! `crate::dtos::admin::labels::LabelResponse`.

use loco_rs::prelude::*;
use sea_orm::{
    DatabaseConnection, DatabaseTransaction, Select, TransactionSession, sea_query::Func,
};

pub use super::_entities::labels::{ActiveModel, Column, Entity, Model};
use super::{
    list_page::{self, Page},
    revisions::{self, Save},
    unique_index,
};
use crate::domain::{
    activity::Actor,
    labels::{LabelInput, LabelRevision},
    list_query::ListQuery,
    revision::Reason,
    seed::SeedLabel,
};

/// The `labels` entity, under the name Loco's generated models use.
pub type Labels = Entity;

/// The partial unique index that keeps the names of live labels apart,
/// compared without regard to case: `lower(name)` among the rows with
/// `deleted_at IS NULL` (the migration `m20261115_090000_labels`). Its
/// refusal of a write is the `409` of a label save.
pub const LIVE_NAME_INDEX: &str = "labels_live_lower_name_key";

/// `lower(name) = lower($1)`: whether a label's name equals `name` but for
/// case, the comparison of the live labels' unique index on `lower(name)`
/// ([`LIVE_NAME_INDEX`]).
///
/// - `name`: the name to compare with, bound as a parameter.
///
/// sea-query's `ExprTrait` is imported only inside this function, for the
/// reason `users::lower_email_equals` gives: in a wider scope its `eq`
/// would compete with `ColumnTrait::eq` and `PartialEq::eq`.
fn lower_name_equals(name: &str) -> Expr {
    use sea_orm::sea_query::ExprTrait as _;
    Func::lower(Expr::col(Column::Name)).eq(Func::lower(name))
}

/// Why [`ActiveModel::edit`] refused a label before it wrote anything. A
/// programming error of the caller, never of the request.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum EditLabelError {
    /// The active model has no `id`, so no row can be addressed.
    #[error(
        "edit needs the label's id: build the active model from a stored row \
         (`into_active_model`)"
    )]
    NoId,
}

#[async_trait::async_trait]
impl ActiveModelBehavior for ActiveModel {
    /// Saves the row unchanged.
    ///
    /// - `_db`: unused; the hook needs no query.
    /// - `_insert`: unused.
    ///
    /// Loco's generated hook stamps `updated_at` from the system time on an
    /// update. backend3 takes stored instants from PostgreSQL's clock
    /// (recipe 7 steps 5 and 6 of the project skill), and the write paths
    /// set the audit columns themselves (ADR 0011 §2, §7), so the hook only
    /// passes the row on.
    async fn before_save<C>(self, _db: &C, _insert: bool) -> std::result::Result<Self, DbErr>
    where
        C: ConnectionTrait,
    {
        Ok(self)
    }
}

impl Model {
    /// The select of every live label, those not in the trash: the
    /// live-row helper of ADR 0011 §3, which every other finder of labels
    /// starts from.
    #[must_use]
    pub fn find_live() -> Select<Entity> {
        Entity::find().filter(Column::DeletedAt.is_null())
    }

    /// The live label with `id`.
    ///
    /// - `db`: the connection or transaction to read with.
    /// - `id`: the label's id.
    ///
    /// # Errors
    ///
    /// [`ModelError::EntityNotFound`] when no label has the id or it is in
    /// the trash, so a trashed label reads like a missing one; a database
    /// error otherwise.
    pub async fn find_live_by_id<C>(db: &C, id: i64) -> ModelResult<Self>
    where
        C: ConnectionTrait,
    {
        Self::find_live()
            .filter(Column::Id.eq(id))
            .one(db)
            .await?
            .ok_or(ModelError::EntityNotFound)
    }

    /// One page of the live labels and their total, as a checked list
    /// request asks (the shared admin list contract,
    /// `crate::domain::list_query`; the request was checked against
    /// `crate::domain::labels::LIST`).
    ///
    /// - `db`: the pool; the page and the total are read in one snapshot
    ///   ([`list_page::fetch`]).
    /// - `query`: the checked request.
    ///
    /// # Errors
    ///
    /// A database error.
    pub async fn list_live_page(
        db: &DatabaseConnection,
        query: &ListQuery,
    ) -> ModelResult<Page<Self>> {
        list_page::fetch(db, Self::find_live(), query).await
    }

    /// Creates the seed's labels that no live label names yet: the invented
    /// labels of the development, test and E2E seed
    /// (`crate::domain::seed::SEED_LABELS`), which `Hooks::seed` passes.
    ///
    /// - `db`: the connection to write with; the labels are written in one
    ///   transaction of their own.
    /// - `labels`: the labels to create, in order, so their ids grow in
    ///   that order.
    ///
    /// A label whose name a live label already has, but for case (the
    /// unique index's comparison), is left out, and the live label is left
    /// exactly as it is, so seeding twice gives the result of seeding once,
    /// as `find_or_create_by` does in a Rails seed. A trashed label with the
    /// name does not count: the index leaves trashed rows out too.
    /// `created_at` and `updated_at` come from PostgreSQL's clock, the
    /// columns' defaults, and `created_by` and `updated_by` stay NULL: the
    /// system created them (research: DECISIONS R15 (D90)). No revision or
    /// activity entry is written: like the seed account
    /// (`users::Model::seed_account`), these rows fill only local databases,
    /// and the caller checks the environment first
    /// (`crate::domain::seed::check_environment`); this method does not see
    /// it.
    ///
    /// Returns how many labels it created.
    ///
    /// # Errors
    ///
    /// [`ModelError::DbErr`] when a query fails, including the unique
    /// violation of a concurrent seed that created one of the names first.
    /// Nothing is written then.
    pub async fn seed_labels<C>(db: &C, labels: &[SeedLabel]) -> ModelResult<u64>
    where
        C: ConnectionTrait + TransactionTrait,
    {
        let txn = db.begin().await?;
        let mut created = 0;
        for label in labels {
            let taken = Self::find_live()
                .filter(lower_name_equals(label.name))
                .one(&txn)
                .await?
                .is_some();
            if taken {
                continue;
            }
            ActiveModel {
                name: ActiveValue::Set(label.name.to_owned()),
                color: ActiveValue::Set(label.color.to_owned()),
                ..Default::default()
            }
            .insert(&txn)
            .await?;
            created += 1;
        }
        txn.commit().await?;
        Ok(created)
    }

    /// Creates a label as `actor`: the row, then its revision 1 and its
    /// `create` entry, in `txn` (ADR 0011 §7), and returns the stored row.
    ///
    /// - `txn`: the save's transaction; the caller commits it, or rolls it
    ///   back on any error.
    /// - `actor`: who creates it: the signed-in account
    ///   ([`Actor::User`]), whose id becomes `created_by` and `updated_by`,
    ///   or [`Actor::System`], which leaves both NULL (ADR 0011 §2).
    /// - `input`: the checked name and colour.
    /// - `request_id`: Loco's request id, written on the revision and the
    ///   entry; `None` outside a request.
    ///
    /// `id`, `created_at` and `updated_at` come from the database: the
    /// identity and the `CURRENT_TIMESTAMP` defaults, the start of `txn`. The
    /// entry is labelled with the name, so the log stays readable after a
    /// purge (ADR 0011 §5).
    ///
    /// # Errors
    ///
    /// - [`ModelError::EntityAlreadyExists`] when a live label already has
    ///   the name, compared without regard to case ([`LIVE_NAME_INDEX`]);
    ///   `txn` is aborted then, and only a rollback is left to do.
    /// - every error of `revisions::record_save`, among them a refused
    ///   activity entry;
    /// - [`ModelError::DbErr`] when another statement fails.
    pub async fn create(
        txn: &DatabaseTransaction,
        actor: Actor,
        input: &LabelInput,
        request_id: Option<String>,
    ) -> ModelResult<Self> {
        let user_id = actor.user_id();
        let label = ActiveModel {
            name: ActiveValue::Set(input.name().to_owned()),
            color: ActiveValue::Set(input.color().to_owned()),
            created_by: ActiveValue::Set(user_id),
            updated_by: ActiveValue::Set(user_id),
            // `id`, the instants and the trash columns stay unset: the
            // INSERT leaves them out, and the database fills them in.
            ..Default::default()
        }
        .insert(txn)
        .await
        .map_err(|error| unique_index::conflict_or_error(error, LIVE_NAME_INDEX))?;
        record_save(txn, actor, &label, Reason::Create, request_id).await?;
        Ok(label)
    }

    /// The label's revision snapshot: its editable fields as stored
    /// (ADR 0011 §4).
    #[must_use]
    pub fn revision_snapshot(&self) -> LabelRevision {
        LabelRevision {
            name: self.name.clone(),
            color: self.color.clone(),
        }
    }
}

impl ActiveModel {
    /// Saves `input` over this live label as `actor`: the row, then its
    /// next revision and the `update` entry with the names of the fields
    /// that changed since the latest revision, in `txn` (ADR 0011 §7), and
    /// returns the stored row.
    ///
    /// - `self`: the label, from a stored row (`into_active_model`). Only
    ///   its `id` is read: the columns the edit writes come from `input`, so
    ///   a column set on `self` is never written (parity SHR-10).
    /// - `txn`: the save's transaction; the caller commits it, or rolls it
    ///   back on any error.
    /// - `actor`: who saves it; its user id becomes `updated_by`, NULL for
    ///   [`Actor::System`]. `created_at` and `created_by` stay as they are.
    /// - `input`: the checked name and colour.
    /// - `request_id`: Loco's request id, written on the revision and the
    ///   entry; `None` outside a request.
    ///
    /// One `UPDATE ... WHERE id = $id AND deleted_at IS NULL RETURNING *`
    /// writes the row, with `updated_at` set to `CURRENT_TIMESTAMP`, the
    /// start of `txn`. A label that was moved to the trash after the caller
    /// read it matches nothing. A save that changes nothing still writes a
    /// revision and an entry that lists no field: ADR 0011 §4 writes one
    /// revision per update.
    ///
    /// # Errors
    ///
    /// - [`ModelError::Any`] with [`EditLabelError::NoId`] when `self` has
    ///   no `id`, before any query;
    /// - [`ModelError::EntityNotFound`] when no live label has the id;
    /// - [`ModelError::EntityAlreadyExists`] when another live label already
    ///   has the name, compared without regard to case ([`LIVE_NAME_INDEX`]);
    ///   `txn` is aborted then, and only a rollback is left to do;
    /// - every error of `revisions::record_save`, among them a refused
    ///   activity entry;
    /// - [`ModelError::DbErr`] when another statement fails.
    pub async fn edit(
        self,
        txn: &DatabaseTransaction,
        actor: Actor,
        input: &LabelInput,
        request_id: Option<String>,
    ) -> ModelResult<Model> {
        let id = match self.id {
            ActiveValue::Set(id) | ActiveValue::Unchanged(id) => id,
            ActiveValue::NotSet => return Err(ModelError::wrap(EditLabelError::NoId)),
        };
        // The id addresses the row and is never part of the `SET` list.
        let changes = Self {
            name: ActiveValue::Set(input.name().to_owned()),
            color: ActiveValue::Set(input.color().to_owned()),
            updated_by: ActiveValue::Set(actor.user_id()),
            ..Default::default()
        };
        let label = Entity::update_many()
            .set(changes)
            .col_expr(Column::UpdatedAt, Expr::current_timestamp())
            .filter(Column::Id.eq(id))
            .filter(Column::DeletedAt.is_null())
            .exec_with_returning(txn)
            .await
            .map_err(|error| unique_index::conflict_or_error(error, LIVE_NAME_INDEX))?
            .into_iter()
            .next()
            .ok_or(ModelError::EntityNotFound)?;
        record_save(txn, actor, &label, Reason::Update, request_id).await?;
        Ok(label)
    }
}

/// Writes the revision of a save of `label` and its activity entry, labelled
/// with the name (ADR 0011 §4, §5): the second half of [`Model::create`] and
/// [`ActiveModel::edit`].
///
/// - `txn`: the save's transaction, which wrote `label`.
/// - `actor`: who saved it.
/// - `label`: the stored row after the write; the snapshot is built from
///   it, so the revision is the record as the database holds it.
/// - `reason`: [`Reason::Create`] or [`Reason::Update`].
/// - `request_id`: Loco's request id, if any.
async fn record_save(
    txn: &DatabaseTransaction,
    actor: Actor,
    label: &Model,
    reason: Reason,
    request_id: Option<String>,
) -> ModelResult<()> {
    let snapshot = label.revision_snapshot();
    revisions::record_save(
        txn,
        actor,
        Save {
            label: Some(label.name.clone()),
            request_id,
            ..Save::new(label.id, reason, &snapshot)
        },
    )
    .await?;
    Ok(())
}
