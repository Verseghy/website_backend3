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
//! The entity derives no serde (`[package.metadata.db.entity]` in
//! `Cargo.toml`): a label goes out only through
//! `crate::dtos::admin::labels::LabelResponse`.

use loco_rs::prelude::*;
use sea_orm::{DatabaseConnection, Select};

pub use super::_entities::labels::{ActiveModel, Column, Entity, Model};
use super::list_page::{self, Page};
use crate::domain::list_query::ListQuery;

/// The `labels` entity, under the name Loco's generated models use.
pub type Labels = Entity;

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
}
