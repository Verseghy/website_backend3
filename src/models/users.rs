//! The admin accounts (ADR 0006): finders and password hashing on top of the
//! generated `users` entity.
//!
//! Passwords are hashed and verified only through `loco_rs::hash`, which uses
//! Argon2id with the argon2 crate's default parameters (ADR 0006 §4). The
//! pure decision of which verifier a stored value needs is
//! [`crate::domain::password_hash`]; this module performs the verification.
//!
//! A failed check that has no Argon2id hash to verify (an account without a
//! password, pending or anonymised; a legacy bcrypt hash; a value of an
//! unknown kind; an e-mail address that matches no account) verifies the
//! constant [`DUMMY_PASSWORD_HASH`] instead. That costs one Argon2id
//! verification with the default parameters, as a wrong password does for a
//! hash that `hash_password` wrote, so sign-in can answer every failure in
//! about the same time (ADR 0006 §4, "Same answer for every failure"). A
//! stored Argon2id value is verified with its own parameters, and backend3
//! stores no Argon2id value other than `hash_password` output.
//!
//! Account events (sign-in, rehash, `token_version` and the two-factor
//! columns) are not content updates: they write no revision and leave
//! `updated_at` and `updated_by` alone (ADR 0011 §4). That is why the
//! `before_save` hook below does not stamp `updated_at` the way Loco's
//! generated model does; the content write paths set it explicitly.
//!
//! The entity derives no serde (`[package.metadata.db.entity]` in
//! `Cargo.toml`), so a [`Model`] can be neither a response nor a request
//! body: accounts go out only through allow-listed DTOs, without the password
//! hash or the TOTP secret (ADR 0005, ADR 0011 §4). It still derives `Debug`,
//! which Sea-ORM requires, so a `Model` is never logged with `?`, which would
//! print both.

use loco_rs::{hash, prelude::*};
use sea_orm::sea_query::Func;

pub use super::_entities::users::{self, ActiveModel, Column, Entity, Model};
use crate::domain::password_hash::{self, DUMMY_PASSWORD_HASH, PasswordHashKind};

/// `lower(email) = lower($1)`: the e-mail comparison of sign-in, written as
/// the expression of the unique index `users_lower_email_key`.
///
/// - `email`: the address to compare with, bound as a parameter.
///
/// sea-query's `ExprTrait` is imported only inside this function: it is
/// implemented for every value that converts into an expression, and in a
/// wider scope its `eq` would compete with `ColumnTrait::eq` and with
/// `PartialEq::eq` (the warning in loco-rs 1.2.0's `prelude.rs`).
fn lower_email_equals(email: &str) -> Expr {
    use sea_orm::sea_query::ExprTrait as _;
    Func::lower(Expr::col(Column::Email)).eq(Func::lower(email))
}

#[async_trait::async_trait]
impl ActiveModelBehavior for ActiveModel {
    /// Gives a new account its public id.
    ///
    /// - `_db`: unused; the hook needs no query.
    /// - `insert`: whether this save inserts the row.
    ///
    /// On insert, a `pid` that the caller left unset becomes a random UUID
    /// v4, as in Loco's starter, so every account has the JWT claim key of
    /// ADR 0006 §1. A `pid` the caller set is kept.
    ///
    /// Unlike Loco's generated hook, this one never touches `updated_at`: an
    /// account event must leave it unchanged (ADR 0011 §4), and only the
    /// caller knows whether a save is one.
    async fn before_save<C>(self, _db: &C, insert: bool) -> std::result::Result<Self, DbErr>
    where
        C: ConnectionTrait,
    {
        let mut this = self;
        if insert && this.pid.is_not_set() {
            // `Uuid` is the uuid crate's type, which loco-rs builds with its
            // `v4` feature.
            this.pid = ActiveValue::Set(Uuid::new_v4());
        }
        Ok(this)
    }
}

/// Hashes a new password for `users.password_hash`: an Argon2id PHC string
/// with the argon2 crate's default parameters and a fresh random salt
/// (ADR 0006 §4).
///
/// - `password`: the plain-text password. The caller applies the password
///   rule of ADR 0006 §6 (12 to 128 characters) before hashing; this function
///   hashes whatever it is given.
///
/// # Errors
///
/// Returns [`ModelError::Any`] when the hasher fails, which with the default
/// parameters only happens if the system random number generator fails.
pub fn hash_password(password: &str) -> ModelResult<String> {
    hash::hash_password(password).map_err(ModelError::wrap)
}

/// Whether `password` matches the stored hash `stored`.
///
/// - `stored`: the account's `users.password_hash`, or `None` for an account
///   that has no password (pending, or anonymised).
/// - `password`: the plain-text password to check.
///
/// Only an Argon2id hash can match, and it is verified with the parameters
/// it carries. The other cases return `false` after verifying
/// [`DUMMY_PASSWORD_HASH`], so they cost what a wrong password costs for a
/// `hash_password` hash:
///
/// - `None`;
/// - a value of an unknown kind, including the other Argon2 variants, which
///   the verifier would otherwise accept;
/// - a legacy bcrypt hash. backend3 verifies those once, with the bcrypt
///   crate, and immediately stores an Argon2id hash instead (ADR 0006 §4);
///   that path writes to the account, so it is its own model method, added
///   with the bcrypt verifier. Until then a bcrypt hash never matches here.
///
/// A malformed `$argon2id$` value never matches either, but it fails without
/// the Argon2 work, because only the verifier parses the PHC string. Nothing
/// stores one: the only Argon2id values backend3 writes are
/// [`hash_password`] output.
#[must_use]
pub fn verify_stored_password(stored: Option<&str>, password: &str) -> bool {
    match stored {
        Some(stored) if password_hash::classify(stored) == PasswordHashKind::Argon2id => {
            hash::verify_password(password, stored)
        }
        _ => {
            verify_dummy_password(password);
            false
        }
    }
}

/// Spends one Argon2id verification of `password` against
/// [`DUMMY_PASSWORD_HASH`] and discards the result.
///
/// - `password`: the plain-text password of the failed attempt.
///
/// Sign-in calls this when no live account matches the e-mail address, so
/// the answer takes as long as for a known address with a wrong password
/// (ADR 0006 §4). The attempt has already failed; nothing depends on the
/// verification's outcome.
pub fn verify_dummy_password(password: &str) {
    // The value is discarded on purpose: only the time spent matters.
    let _ = hash::verify_password(password, DUMMY_PASSWORD_HASH);
}

impl Model {
    /// Finds the live account whose e-mail address is `email`, ignoring case,
    /// for sign-in.
    ///
    /// - `db`: the connection or transaction to query.
    /// - `email`: the address as the user typed it.
    ///
    /// "Live" means not in the trash (`deleted_at IS NULL`) and not
    /// anonymised (`anonymised_at IS NULL`). The second filter is defence in
    /// depth: the anonymisation CHECKs already keep an anonymised row trashed
    /// and without an address. An inactive account is live: the caller
    /// decides what deactivation means for it (ADR 0006 §7).
    ///
    /// Both sides are folded with PostgreSQL's `lower()`, the expression of
    /// the unique index `users_lower_email_key`, so the lookup uses that
    /// index and always agrees with it on what counts as the same address.
    /// `lower()` follows the database's default collation: under the
    /// builtin `C.UTF-8` locale that production, compose and CI initialise
    /// PostgreSQL with, it folds accented letters too (Unicode simple case
    /// mapping), under the C locale only ASCII.
    ///
    /// # Errors
    ///
    /// [`ModelError::EntityNotFound`] when no live account has the address,
    /// including when only a trashed or anonymised one has it;
    /// [`ModelError::DbErr`] when the query fails.
    pub async fn find_live_by_email<C>(db: &C, email: &str) -> ModelResult<Self>
    where
        C: ConnectionTrait,
    {
        Entity::find()
            .filter(lower_email_equals(email))
            .filter(Column::DeletedAt.is_null())
            .filter(Column::AnonymisedAt.is_null())
            .one(db)
            .await?
            .ok_or(ModelError::EntityNotFound)
    }

    /// Finds the account whose public id is `pid`, whatever its state.
    ///
    /// - `db`: the connection or transaction to query.
    /// - `pid`: the account's UUID as text, as the JWT claims carry it
    ///   (ADR 0006 §1).
    ///
    /// Trashed, anonymised and inactive accounts are returned too: the
    /// session check rejects them itself, with the reason (ADR 0006 §1).
    ///
    /// # Errors
    ///
    /// [`ModelError::EntityNotFound`] when no account has the id, and when
    /// `pid` is not a UUID, because then no account can have it;
    /// [`ModelError::DbErr`] when the query fails.
    pub async fn find_by_pid<C>(db: &C, pid: &str) -> ModelResult<Self>
    where
        C: ConnectionTrait,
    {
        let Ok(pid) = Uuid::parse_str(pid) else {
            return Err(ModelError::EntityNotFound);
        };
        Entity::find()
            .filter(Column::Pid.eq(pid))
            .one(db)
            .await?
            .ok_or(ModelError::EntityNotFound)
    }

    /// Whether `password` matches this account's stored password, with the
    /// equal-cost failure paths of [`verify_stored_password`].
    ///
    /// - `password`: the plain-text password to check.
    ///
    /// The account's state (active, trashed) is not checked here; sign-in
    /// checks it after the password, so a disabled account costs the same.
    #[must_use]
    pub fn verify_password(&self, password: &str) -> bool {
        verify_stored_password(self.password_hash.as_deref(), password)
    }
}
