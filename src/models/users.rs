//! The admin accounts (ADR 0006): finders, password hashing and the stored
//! TOTP secret on top of the generated `users` entity.
//!
//! The TOTP secret is encrypted with the account's `id` as associated data
//! ([`crate::domain::totp_secret`]); [`Model::totp_secret`] decrypts it with
//! the key the caller passes in, and the code check itself is
//! [`crate::domain::totp`].
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
//! Revocation is [`ActiveModel::revoke_sessions`]: it increments
//! `token_version` in one `UPDATE`, which ends every session of the account
//! on its next request (ADR 0006 §2). Logout uses it, and so will the other
//! events of that section.
//!
//! Sign-in ([`Model::authenticate`]) and the session token
//! ([`Model::generate_session_token`]) live here too. The [`Authenticable`]
//! impl lets Loco's `JWTWithUser` extractor load the account a token names,
//! and [`Model::session_state`] hands the admin extractor the columns that
//! decide whether the session still holds (ADR 0006 §1).
//! [`Model::seed_account`] creates the invented account of the development,
//! test and E2E seed (`crate::domain::seed`).
//!
//! The entity derives no serde (`[package.metadata.db.entity]` in
//! `Cargo.toml`), so a [`Model`] can be neither a response nor a request
//! body: accounts go out only through allow-listed DTOs, without the password
//! hash or the TOTP secret (ADR 0005, ADR 0011 §4). It still derives `Debug`,
//! which Sea-ORM requires, so a `Model` is never logged with `?`, which would
//! print both.

use chrono::{DateTime, Utc};
use loco_rs::{auth::jwt::JWT, hash, prelude::*};
use sea_orm::sea_query::Func;

pub use super::_entities::users::{self, ActiveModel, Column, Entity, Model};
use crate::domain::{
    email_address,
    password_hash::{self, DUMMY_PASSWORD_HASH, PasswordHashKind},
    seed::SeedAccount,
    session::{AUTH_TIME_CLAIM, AccountState, SCOPE_CLAIM, SessionScope, TOKEN_VERSION_CLAIM},
    totp::TotpSecret,
    totp_secret::{self, DecryptError, TotpEncryptionKey},
};

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

/// `token_version + 1`: the new value of a revocation, computed by
/// PostgreSQL from the stored value ([`ActiveModel::revoke_sessions`]).
///
/// `ExprTrait` is imported here only, for the reason
/// [`lower_email_equals`] gives.
fn token_version_plus_one() -> Expr {
    use sea_orm::sea_query::ExprTrait as _;
    Expr::col(Column::TokenVersion).add(1)
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

/// Why [`ActiveModel::revoke_sessions`] refused an account before it wrote
/// anything. Both are programming errors of the caller, never of the request.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum RevokeSessionsError {
    /// The active model has no `id`, so no row can be addressed.
    #[error(
        "revoke_sessions needs the account's id: build the active model from a stored row \
         (`into_active_model`)"
    )]
    NoId,
    /// The caller set `token_version` itself.
    #[error(
        "revoke_sessions increments token_version itself; leave it unset on the active model \
         (ADR 0006 §2)"
    )]
    TokenVersionSet,
}

impl ActiveModel {
    /// Ends every session of this account: increments `token_version` by one
    /// and returns the account as stored afterwards (ADR 0006 §2).
    ///
    /// - `db`: the connection or transaction to write to.
    ///
    /// Every token carries the version it was issued at (`tv`), and the
    /// admin session extractor refuses one that differs from the stored
    /// value, so after this call every token issued before it fails on its
    /// next request: cookies and Bearer tokens alike, on every device. A
    /// token issued from the returned row's `token_version` holds.
    ///
    /// The increment is one statement, `token_version = token_version + 1`,
    /// not a read and a write: PostgreSQL locks the row for it, so two
    /// concurrent revocations move the version twice, and the one that
    /// commits second moves it past the value the first returned. A token
    /// that an event issues from its returned value (the new cookie of a
    /// password change) therefore ends with every later revocation, and a
    /// read-then-write could have let it survive one.
    ///
    /// The changes that this active model holds, if any, are written in the
    /// same `UPDATE`, so an account event that also changes a column (a
    /// deactivation, a new password) revokes in the statement that makes the
    /// change. Logout holds none.
    ///
    /// It writes nothing else: no revision, and neither `updated_at` nor
    /// `updated_by` unless the caller set them, because a revocation is an
    /// account event, not a content update (ADR 0011 §4). The `UPDATE` goes
    /// past the model's save hooks; [`ActiveModelBehavior::before_save`] of
    /// this module acts on inserts only. The column is an `integer`: after
    /// 2,147,483,647 increments PostgreSQL refuses the next one, which a
    /// school's accounts never reach.
    ///
    /// # Errors
    ///
    /// [`ModelError::Any`] with [`RevokeSessionsError::NoId`] when the active
    /// model has no `id`, and with [`RevokeSessionsError::TokenVersionSet`]
    /// when the caller set `token_version`; both before any query.
    /// [`ModelError::EntityNotFound`] when no row has the id (accounts are
    /// anonymised, never deleted, so a stored row's id always matches).
    /// [`ModelError::DbErr`] when the statement fails.
    pub async fn revoke_sessions<C>(mut self, db: &C) -> ModelResult<Model>
    where
        C: ConnectionTrait,
    {
        let id = match self.id {
            ActiveValue::Set(id) | ActiveValue::Unchanged(id) => id,
            ActiveValue::NotSet => return Err(ModelError::wrap(RevokeSessionsError::NoId)),
        };
        if self.token_version.is_set() {
            return Err(ModelError::wrap(RevokeSessionsError::TokenVersionSet));
        }
        // The id addresses the row; it is never part of the `SET` list.
        self.id = ActiveValue::NotSet;
        Entity::update_many()
            .set(self)
            .col_expr(Column::TokenVersion, token_version_plus_one())
            .filter(Column::Id.eq(id))
            .exec_with_returning(db)
            .await?
            .into_iter()
            .next()
            .ok_or(ModelError::EntityNotFound)
    }
}

/// What [`Model::seed_account`] found.
#[derive(Debug)]
pub enum SeedOutcome {
    /// No account had the seed account's address, so it was created.
    Created(Model),
    /// An account already has the address, in whatever state; the seed left
    /// it as it is.
    AlreadyPresent(Model),
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
    /// An address that no account can hold ([`email_address::can_be_stored`])
    /// is not sent to the database, where it would fail the whole query
    /// instead of matching nothing.
    ///
    /// # Errors
    ///
    /// [`ModelError::EntityNotFound`] when no live account has the address,
    /// including when only a trashed or anonymised one has it and when no
    /// account can hold it;
    /// [`ModelError::DbErr`] when the query fails.
    pub async fn find_live_by_email<C>(db: &C, email: &str) -> ModelResult<Self>
    where
        C: ConnectionTrait,
    {
        if !email_address::can_be_stored(email) {
            return Err(ModelError::EntityNotFound);
        }
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

    /// Checks a browser sign-in and returns the account that may start an
    /// admin session, or `None` when the sign-in fails (ADR 0006 §4).
    ///
    /// - `db`: the connection or transaction to query.
    /// - `email`: the address as the user typed it, matched without regard
    ///   to case ([`Model::find_live_by_email`]).
    /// - `password`: the password as typed.
    ///
    /// Every failure costs one Argon2id verification, so the time does not
    /// tell them apart ("Same answer for every failure"): an unknown, trashed
    /// or anonymised address, or one that no account can hold, verifies
    /// [`DUMMY_PASSWORD_HASH`], and the account's state is checked only after
    /// its own password. A sign-in fails when the password is wrong, when the
    /// account has none, when it is inactive, and, until the two-factor step
    /// and the forced password change of ADR 0006 §4 exist, when it has
    /// two-factor turned on or must change its password: those accounts must
    /// never get a full `admin` session from the password alone, and they get
    /// the same failure as a wrong password until then (the maintainer's
    /// answer of 2026-10-25, research: DECISIONS R15 (D74)).
    ///
    /// The Argon2 work runs on the request's task, as in Loco's own sign-in
    /// handler.
    ///
    /// # Errors
    ///
    /// [`ModelError::DbErr`] when the query fails. A failed sign-in is
    /// `Ok(None)`, never an error.
    pub async fn authenticate<C>(db: &C, email: &str, password: &str) -> ModelResult<Option<Self>>
    where
        C: ConnectionTrait,
    {
        let user = match Self::find_live_by_email(db, email).await {
            Ok(user) => user,
            Err(ModelError::EntityNotFound) => {
                verify_dummy_password(password);
                return Ok(None);
            }
            Err(error) => return Err(error),
        };
        // The password first, before any state check could end the
        // evaluation, so an account that may not sign in costs what a wrong
        // password costs.
        let password_matches = user.verify_password(password);
        let may_sign_in =
            user.active && user.totp_enabled_at.is_none() && !user.must_change_password;
        Ok((password_matches && may_sign_in).then_some(user))
    }

    /// Creates the seed account `account` unless an account already has its
    /// address, for `Hooks::seed` (`crate::domain::seed`).
    ///
    /// - `db`: the connection or transaction to write to.
    /// - `account`: the account to create; its password is hashed here, with
    ///   [`hash_password`] and a fresh salt, so no hash of it is ever
    ///   committed.
    ///
    /// The address is compared as sign-in compares it (`lower(email)`), over
    /// every row, trashed ones included, because the unique index
    /// `users_lower_email_key` covers them too. An account that has it is
    /// left exactly as it is, whatever its state, so seeding twice gives the
    /// result of seeding once, as `find_or_create_by` does in a Rails seed.
    /// To start over, recreate the database (`AGENTS.md`, "Commands").
    ///
    /// The new account is active, has a password and no two-factor, and needs
    /// no password change, so the browser sign-in gives it a full session.
    /// `created_at` and `updated_at` come from PostgreSQL's clock, the
    /// columns' default (research: DECISIONS R15 (D90)), and `created_by`
    /// stays NULL: the system created it. No revision or activity entry is
    /// written: the seed fills only local development, test and E2E
    /// databases (`crate::domain::seed::check_environment`).
    ///
    /// The caller checks the environment first; this method does not see it.
    ///
    /// # Errors
    ///
    /// [`ModelError::Any`] when hashing fails; [`ModelError::DbErr`] when a
    /// query fails, including the unique violation of a concurrent seed that
    /// inserted the same address first.
    pub async fn seed_account<C>(db: &C, account: &SeedAccount) -> ModelResult<SeedOutcome>
    where
        C: ConnectionTrait,
    {
        if let Some(existing) = Entity::find()
            .filter(lower_email_equals(account.email))
            .one(db)
            .await?
        {
            return Ok(SeedOutcome::AlreadyPresent(existing));
        }
        let created = ActiveModel {
            email: ActiveValue::Set(Some(account.email.to_owned())),
            name: ActiveValue::Set(account.name.to_owned()),
            password_hash: ActiveValue::Set(Some(hash_password(account.password)?)),
            ..Default::default()
        }
        .insert(db)
        .await?;
        Ok(SeedOutcome::Created(created))
    }

    /// Signs a full admin session token for this account (ADR 0006 §1).
    ///
    /// - `secret`: the base64 JWT secret, `auth.jwt.secret`.
    /// - `expiration`: the token's lifetime in seconds, `auth.jwt.expiration`.
    /// - `auth_time`: when the password step succeeded, the app clock's "now"
    ///   that the caller read (ADR 0010 point 7). The claim holds it in whole
    ///   seconds since the Unix epoch, a JWT `NumericDate` like `exp`.
    ///
    /// The token is Loco's HS512 JWT: Loco's `pid` claim (this account's
    /// UUID) and `exp` (the current time plus `expiration`, which Loco
    /// computes), plus [`TOKEN_VERSION_CLAIM`] with `token_version`,
    /// [`SCOPE_CLAIM`] with [`SessionScope::Admin`] and [`AUTH_TIME_CLAIM`]
    /// (the claim names of [`crate::domain::session`], which checks them).
    ///
    /// # Errors
    ///
    /// [`ModelError::Jwt`] when `secret` is not valid base64 or signing fails.
    pub fn generate_session_token(
        &self,
        secret: &str,
        expiration: u64,
        auth_time: DateTime<Utc>,
    ) -> ModelResult<String> {
        let mut claims = serde_json::Map::new();
        claims.insert(TOKEN_VERSION_CLAIM.to_owned(), self.token_version.into());
        claims.insert(SCOPE_CLAIM.to_owned(), SessionScope::Admin.claim().into());
        claims.insert(AUTH_TIME_CLAIM.to_owned(), auth_time.timestamp().into());
        Ok(JWT::new(secret).generate_token(expiration, self.pid.to_string(), claims)?)
    }

    /// The columns of this row that decide whether its sessions still hold,
    /// for [`crate::domain::session::check_session`] (ADR 0006 §1).
    ///
    /// The admin extractor calls it on the row it has just read for the
    /// request, so the check always sees the stored state: a deactivation, a
    /// trash or a `token_version` increment applies on the next request
    /// (ADR 0006 §2, ADR 0016).
    #[must_use]
    pub fn session_state(&self) -> AccountState {
        AccountState {
            trashed: self.deleted_at.is_some(),
            anonymised: self.anonymised_at.is_some(),
            active: self.active,
            token_version: self.token_version,
        }
    }

    /// Decrypts this account's stored TOTP secret
    /// (`users.totp_secret_encrypted`, ADR 0006 §6).
    ///
    /// - `key`: the derived key from `settings.admin_auth`, passed in by the
    ///   caller (ADR 0008 rule 2).
    ///
    /// The value is authenticated with this row's `id`, so bytes that were
    /// copied here from another account's row do not decrypt. Whether
    /// two-factor is on is `totp_enabled_at`, not this column: during
    /// enrolment the secret is stored before the first code confirms it.
    ///
    /// # Errors
    ///
    /// [`DecryptError::Length`] or [`DecryptError::Authentication`] when the
    /// stored value is not this row's secret under `key`. Either means the
    /// row was changed outside backend3 or the key is not the one the
    /// secret was stored with; the caller refuses the code.
    pub fn totp_secret(&self, key: &TotpEncryptionKey) -> Result<Option<TotpSecret>, DecryptError> {
        self.totp_secret_encrypted
            .as_deref()
            .map(|stored| totp_secret::decrypt(key, self.id, stored))
            .transpose()
    }
}

/// Lets Loco's `JWTWithUser<users::Model>` extractor load the account that a
/// session token names (ADR 0006 §1).
#[async_trait::async_trait]
impl Authenticable for Model {
    /// backend3 has no API keys: the starter's `api_key` column is not
    /// created (ADR 0006 §4), and scripts use Bearer session tokens, so no
    /// key ever matches.
    ///
    /// - `_db`: unused.
    /// - `_api_key`: unused.
    async fn find_by_api_key(_db: &DatabaseConnection, _api_key: &str) -> ModelResult<Self> {
        Err(ModelError::EntityNotFound)
    }

    /// Loads the account whose `pid` the token's `pid` claim holds, whatever
    /// its state: the admin session check rejects trashed, anonymised and
    /// inactive accounts with the reason (ADR 0006 §1).
    ///
    /// - `db`: the application's connection.
    /// - `claims_key`: the token's `pid` claim.
    async fn find_by_claims_key(db: &DatabaseConnection, claims_key: &str) -> ModelResult<Self> {
        Self::find_by_pid(db, claims_key).await
    }
}
