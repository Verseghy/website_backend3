//! Labels, „címkék” (parity LBL-01), on top of the generated `labels`
//! entity: the live-row finders that every read of labels goes through
//! (ADR 0011 §3).
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
//! The writes (create, update, delete, restore, purge) come with their
//! slices, through the safety net of ADR 0011 §7. The audit instants are
//! PostgreSQL's (`CURRENT_TIMESTAMP` defaults, and `Expr::current_timestamp()`
//! in an UPDATE; research: DECISIONS R15, D90), never the app clock's, so
//! the hook below stamps nothing.
//!
//! [`Model::seed_labels`] is the one write so far: the development, test and
//! E2E seed's invented labels (`crate::domain::seed::SEED_LABELS`), which the
//! E2E suite lists. Like the seed account, it writes no revision and no
//! activity entry: only local databases ever hold those rows (ADR 0040).
//!
//! The entity derives no serde (`[package.metadata.db.entity]` in
//! `Cargo.toml`): a label goes out only through
//! `crate::dtos::admin::labels::LabelResponse`.

use loco_rs::prelude::*;
use sea_orm::{DatabaseConnection, Select, TransactionSession, sea_query::Func};

pub use super::_entities::labels::{ActiveModel, Column, Entity, Model};
use super::list_page::{self, Page};
use crate::domain::{list_query::ListQuery, seed::SeedLabel};

/// The `labels` entity, under the name Loco's generated models use.
pub type Labels = Entity;

/// `lower(name) = lower($1)`: whether a label's name equals `name` but for
/// case, the comparison of the live labels' unique index on `lower(name)`
/// (the labels migration).
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
}
