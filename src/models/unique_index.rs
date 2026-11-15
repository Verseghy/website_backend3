//! Which unique index refused a write: the conflicts that an admin write
//! answers `409` (the write contract; ADR 0009, "Constraints"; ADR 0011 §3;
//! ADR 0021 §4's 409 row). A model module without an entity (ADR 0008
//! rule 5), because every entity with a uniqueness rule shares it: the
//! labels' live names first, then the pages' live slugs and the canteen's
//! one live day per date.
//!
//! **The index decides, not a lookup.** A uniqueness rule over live rows is
//! a partial unique index (`... WHERE deleted_at IS NULL`), which PostgreSQL
//! checks on every insert and update. A write that first looked for a
//! holder of the value would race with a concurrent write of the same value
//! (both look, both find none, both write); the index lets the first one in
//! and refuses the second, after waiting for the first to commit. So the
//! model writes the row and turns that refusal into a conflict with
//! [`conflict_or_error`].
//!
//! **Only the named index.** A write can break other rules at the same
//! time: a primary key, a foreign key, a CHECK, another unique index. Only
//! the index that the caller names becomes a conflict
//! ([`ModelError::EntityAlreadyExists`], Loco's own "Entity already
//! exists"); everything else stays the database error it is, a `500`,
//! because the write contract's validation keeps every other rule from
//! being broken by a request (parity SHR-13). PostgreSQL names the index of
//! a unique violation in the error's constraint name (SQLSTATE 23505),
//! which sqlx reports for the Postgres driver only; backend3 runs on
//! PostgreSQL alone (ADR 0002).
//!
//! **The transaction is spent.** PostgreSQL aborts a transaction in which a
//! statement failed, so a caller that gets the conflict rolls its
//! transaction back and answers; it never goes on writing in it.

use loco_rs::model::ModelError;
use sea_orm::{DbErr, RuntimeErr};

/// Whether `error` is PostgreSQL's refusal of a row by the unique index (or
/// unique constraint) named `index`.
///
/// - `error`: what a write returned.
/// - `index`: the index's name, as its migration creates it (for example
///   `labels_live_lower_name_key`).
///
/// `false` for every other error: another index's or constraint's refusal,
/// a foreign key or CHECK violation, a connection failure, or an error that
/// did not come from the database.
#[must_use]
pub fn refused_by(error: &DbErr, index: &str) -> bool {
    // An insert or update with `RETURNING` fails as a query, one without as
    // an execution (sea-orm 2.0.4 `driver/sqlx_common.rs`).
    let (DbErr::Exec(RuntimeErr::SqlxError(error)) | DbErr::Query(RuntimeErr::SqlxError(error))) =
        error
    else {
        return false;
    };
    // `as_database_error`, `constraint()` and `is_unique_violation()` are
    // methods of the driver's error and of its `DatabaseError` trait object,
    // which the Postgres driver fills from the server's error fields. They
    // are called as methods, so this module never names the driver crate,
    // which `tests/architecture.rs` keeps to the legacy import.
    error.as_database_error().is_some_and(|database| {
        database.is_unique_violation() && database.constraint() == Some(index)
    })
}

/// The model error for a failed write: [`ModelError::EntityAlreadyExists`]
/// when the unique index `index` refused it, and the database error itself
/// otherwise.
///
/// - `error`: what the write returned.
/// - `index`: the unique index whose refusal is a conflict of the write.
///
/// The caller maps `EntityAlreadyExists` to its resource's `409` code (the
/// labels: `label.name_taken`) and rolls its transaction back.
#[must_use]
pub fn conflict_or_error(error: DbErr, index: &str) -> ModelError {
    if refused_by(&error, index) {
        ModelError::EntityAlreadyExists
    } else {
        ModelError::DbErr(error)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_error_that_did_not_come_from_the_database_is_no_conflict() {
        // A driver error needs a server to answer; those cases are in the
        // model tests (`tests/models/labels_write.rs`), which break the
        // index for real.
        for error in [
            DbErr::RecordNotFound("labels".to_owned()),
            DbErr::Custom("labels_live_lower_name_key".to_owned()),
            DbErr::Exec(RuntimeErr::Internal(
                "duplicate key value violates unique constraint \"labels_live_lower_name_key\""
                    .to_owned(),
            )),
            DbErr::Query(RuntimeErr::Internal(
                "labels_live_lower_name_key".to_owned(),
            )),
        ] {
            assert!(!refused_by(&error, "labels_live_lower_name_key"), "{error}");
            assert!(
                matches!(
                    conflict_or_error(error.clone(), "labels_live_lower_name_key"),
                    ModelError::DbErr(_)
                ),
                "{error}"
            );
        }
    }
}
