//! The four roles (ADR 0007 §2, §3) on top of the generated `roles` entity.
//!
//! The rows are seeded by the `roles_and_grants` migration with backend1's
//! ids, and nothing in backend3 writes them: a new role is a migration plus
//! a change to `crate::domain::authz::Role` in the same PR. What a role may
//! do is not stored at all; it is `crate::domain::authz::ROLE_PERMISSIONS`.
//! The table exists so that `user_roles.role_id` has a foreign key, and so
//! that the importer can map backend1's role names to ids (ADR 0007 §9).
//!
//! The entity derives no serde (`[package.metadata.db.entity]` in
//! `Cargo.toml`); roles go out only through the DTOs of
//! `crate::dtos::admin::authz`.

use sea_orm::{QueryOrder, entity::prelude::*};

pub use super::_entities::roles::{ActiveModel, Entity, Model};

/// The `roles` entity, under the name Loco's generated models use.
pub type Roles = Entity;

impl Model {
    /// Every role row, in id order.
    ///
    /// - `db`: the connection or transaction to read on.
    ///
    /// The importer compares them with `crate::domain::authz::Role::ALL`
    /// before it maps the legacy role names to them (data-migration.md
    /// §6.13).
    ///
    /// # Errors
    ///
    /// A failed query.
    pub async fn list_in_id_order<C>(db: &C) -> std::result::Result<Vec<Self>, DbErr>
    where
        C: ConnectionTrait,
    {
        Entity::find()
            .order_by_asc(super::_entities::roles::Column::Id)
            .all(db)
            .await
    }
}

#[async_trait::async_trait]
impl ActiveModelBehavior for ActiveModel {
    /// Saves the row unchanged: `roles` has no columns for a hook to fill.
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
