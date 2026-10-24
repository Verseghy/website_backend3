//! The database objects that Hungarian search and ordering need (ADR 0020 §1).
//!
//! - `unaccent` folds accents for case- and accent-insensitive matching:
//!   `unaccent(lower(x COLLATE hu_icu))`.
//! - `hu_icu` is an ICU collation for the Hungarian alphabet (`cs` after `c`,
//!   `zs` after `z`), used through explicit `COLLATE hu_icu` clauses. The
//!   database's default collation stays deterministic (ADR 0002).
//!
//! The collation has its own name instead of relying on the predefined
//! `hu-x-icu`, so the schema does not depend on which ICU locales `initdb`
//! imported. Both statements fail loudly on an image without the `unaccent`
//! contrib files or without ICU support; there is deliberately no fallback
//! (ADR 0020 §1). `unaccent` is a trusted extension, so the database owner
//! role that runs migrations in production can create it without superuser
//! rights (ADR 0002).

use sea_orm_migration::prelude::*;

/// Creates the `unaccent` extension and the `hu_icu` collation.
#[derive(DeriveMigrationName)]
pub struct Migration;

/// Statements of [`Migration::up`], in order. `IF NOT EXISTS` makes the
/// migration safe on a database where an operator already created either
/// object by hand.
const UP: &[&str] = &[
    "CREATE EXTENSION IF NOT EXISTS unaccent",
    "CREATE COLLATION IF NOT EXISTS hu_icu (provider = icu, locale = 'hu')",
];

/// Statements of [`Migration::down`]: the reverse order of [`UP`]. Later
/// migrations that use either object are rolled back before this one, so
/// nothing depends on them by the time these run.
const DOWN: &[&str] = &[
    "DROP COLLATION IF EXISTS hu_icu",
    "DROP EXTENSION IF EXISTS unaccent",
];

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        run_all(manager, UP).await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        run_all(manager, DOWN).await
    }
}

/// Runs each statement on the migration's connection, stopping at the first
/// error so a missing extension or collation provider surfaces as-is.
async fn run_all(manager: &SchemaManager<'_>, statements: &[&str]) -> Result<(), DbErr> {
    let db = manager.get_connection();
    for statement in statements {
        db.execute_unprepared(statement).await?;
    }
    Ok(())
}
