//! The extra grants of each account, „Egyéni jogosultságok” (ADR 0007 §3),
//! on top of the generated `user_permissions` entity: one row per account
//! and permission, `(user_id, permission)`.
//!
//! `permission` holds a permission identifier, such as `edit events`; the
//! table's CHECK `user_permissions_permission_known` admits only the sixteen
//! names of `crate::domain::authz::Permission`. The rows are read together
//! with the account's roles by `users::Model::grants`, on every admin
//! request (ADR 0007 §5, "Fresh data"), and written by the user editor and
//! the importer in later slices. A grant change is recorded in the account's
//! revision and activity log (ADR 0011 §1, the join-table row).

use sea_orm::entity::prelude::*;

pub use super::_entities::user_permissions::{ActiveModel, Entity, Model};

/// The `user_permissions` entity, under the name Loco's generated models
/// use.
pub type UserPermissions = Entity;

#[async_trait::async_trait]
impl ActiveModelBehavior for ActiveModel {
    /// Saves the row unchanged: a grant has no columns for a hook to fill.
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
