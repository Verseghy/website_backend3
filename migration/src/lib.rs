//! Sea-ORM migrations of backend3.
//!
//! Every schema change is a migration in this crate (ADR 0009). Loco runs them
//! through `backend3-cli db migrate`, or at boot when `database.auto_migrate`
//! is on. `cargo loco generate migration` (and `generate model`) adds the new
//! file and edits this one in two places, so both anchors below must stay:
//! the `mod` line goes above the first line that mentions the struct
//! declaration (the comment in front of its doc comment), and the
//! `Box::new(...)` line goes above the marker comment inside
//! [`Migrator::migrations`]. loco-gen 1.2.0 inserts before the *first*
//! matching line of the file, so no other line may contain either anchor
//! text.
//!
//! Migrations run in the order of [`Migrator::migrations`]. Their file names
//! start with a UTC timestamp, which keeps that order stable across branches.

pub use sea_orm_migration::prelude::*;

mod m20261024_000001_search_support;
mod m20261025_044606_users;
// Generated `mod` lines go above the first line naming `pub struct Migrator`,
// which is this one, so the doc comment below stays attached to the struct
// (the `missing_docs` lint requires that).

/// The migrator that Loco's `create_app::<App, Migrator>` and the `db`
/// subcommands drive.
pub struct Migrator;

#[async_trait::async_trait]
impl MigratorTrait for Migrator {
    fn migrations() -> Vec<Box<dyn MigrationTrait>> {
        vec![
            Box::new(m20261024_000001_search_support::Migration),
            Box::new(m20261025_044606_users::Migration),
            // inject-above (do not remove this comment)
        ]
    }
}
