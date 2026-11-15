//! The roles each account holds (ADR 0007 §3) on top of the generated
//! `user_roles` entity: one row per account and role, `(user_id, role_id)`.
//!
//! The rows are read together with the account's extra grants by
//! `users::Model::grants`, on every admin request (ADR 0007 §5, "Fresh
//! data"). They are written with the account: the development seed gives
//! its account the `admin` role (`users::Model::seed_account`), and the user
//! editor and the importer write them in later slices. A role change is
//! recorded in the account's revision and activity log, never as a history
//! of its own (ADR 0011 §1, the join-table row).

use sea_orm::entity::prelude::*;

pub use super::_entities::user_roles::{ActiveModel, Entity, Model};

/// The `user_roles` entity, under the name Loco's generated models use.
pub type UserRoles = Entity;

#[async_trait::async_trait]
impl ActiveModelBehavior for ActiveModel {
    /// Saves the row unchanged: a link has no columns for a hook to fill.
    ///
    /// - `_db`: unused.
    /// - `_insert`: unused.
    async fn before_save<C>(self, _db: &C, _insert: bool) -> std::result::Result<Self, DbErr>
    where
        C: ConnectionTrait,
    {
        Ok(self)
    }
}
