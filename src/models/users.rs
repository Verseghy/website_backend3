//! The admin accounts (ADR 0006): finders, password hashing and the stored
//! TOTP secret on top of the generated `users` entity.
//!
//! The TOTP secret is encrypted with the account's `id` as associated data
//! ([`crate::domain::totp_secret`]); [`Model::totp_secret`] decrypts it with
//! the key the caller passes in, and the code check itself is
//! [`crate::domain::totp`].
//!
//! Passwords are hashed and verified through `loco_rs::hash`, which uses
//! Argon2id with the argon2 crate's default parameters (ADR 0006 §4). The
//! one exception is the legacy bcrypt hash that the importer copies from
//! Laravel (`$2y$10$...`, ADR 0017): loco-rs has no bcrypt verifier, so
//! [`check_stored_password`] verifies it with the `bcrypt` crate, and a
//! sign-in that it lets in stores an Argon2id hash of the same password in
//! its place, on the sign-in's own `UPDATE` ([`AcceptedSignIn`],
//! [`ActiveModel::record_sign_in`]). The pure decision of which verifier a
//! stored value needs is [`crate::domain::password_hash`]; this module
//! performs the verification.
//!
//! A failed check that has no hash to verify (an account without a
//! password, pending or anonymised; a value of an unknown kind; a bcrypt
//! value that the bcrypt crate cannot read; an e-mail address that matches
//! no account) verifies the constant [`DUMMY_PASSWORD_HASH`] instead. That
//! costs one Argon2id verification with the default parameters, as a wrong
//! password does for a hash that `hash_password` wrote (ADR 0006 §4, "Same
//! answer for every failure"). A stored Argon2id value is verified with its own
//! parameters, and backend3 stores no Argon2id value other than
//! `hash_password` output. A wrong password for a legacy bcrypt hash costs
//! that hash's own bcrypt verification, at the cost the hash carries (10
//! for every imported hash).
//!
//! Account events (sign-in, rehash, `token_version` and the two-factor
//! columns) are not content updates: they write no revision and leave
//! `updated_at` and `updated_by` alone (ADR 0011 §4). That is why the
//! `before_save` hook below does not stamp `updated_at` the way Loco's
//! generated model does; the content write paths set it explicitly.
//!
//! Revocation is [`ActiveModel::revoke_sessions`]: it increments
//! `token_version` in one `UPDATE`, which ends every session of the account
//! on its next request (ADR 0006 §2). Logout ([`ActiveModel::log_out`]) uses
//! it, and so will the other events of that section.
//!
//! Sign-in ([`Model::authenticate`]) and the session token
//! ([`Model::generate_session_token`]) live here too, with the activity
//! entries that the sign-in and the logout write (ADR 0011 §5): a completed
//! sign-in sets `last_login_at`, replaces a legacy bcrypt hash, and writes
//! `login` ([`ActiveModel::record_sign_in`]); the rehash writes no entry of
//! its own, because ADR 0011 §5 has no such action and the `login` entry
//! already records the sign-in that made it. A failed sign-in of a known
//! account writes `login_failed` ([`record_failed_sign_in`]), and a logout
//! writes `logout` ([`ActiveModel::log_out`]). Each entry is about the account
//! (`entity` `users/<id>`, never a label: ADR 0011 §5 keeps names of people
//! out of the log) and holds the request id and the client address of the
//! request ([`RequestContext`]). The entries of a completed sign-in and of a
//! logout record a change of the account and are written on its
//! transaction; a failed sign-in changes no data, and its entry is written
//! on a connection of its own (ADR 0011 §5). The [`Authenticable`]
//! impl lets Loco's `JWTWithUser` extractor load the account a token names,
//! and [`Model::session_state`] hands the admin session check
//! (`open_session` in `controllers::admin::session`, which the admin route
//! table's guard calls) the columns that decide whether the session still
//! holds (ADR 0006 §1). [`Model::grants`] reads the roles and extra grants
//! that the account holds, which that check loads on every request
//! (ADR 0007 §5, "Fresh data").
//! [`Model::seed_account`] creates the invented account of the development,
//! test and E2E seed (`crate::domain::seed`) with its roles.
//!
//! The entity derives no serde (`[package.metadata.db.entity]` in
//! `Cargo.toml`), so a [`Model`] can be neither a response nor a request
//! body: accounts go out only through allow-listed DTOs, without the password
//! hash or the TOTP secret (ADR 0005, ADR 0011 §4). It still derives `Debug`,
//! which Sea-ORM requires, so a `Model` is never logged with `?`, which would
//! print both.

use chrono::{DateTime, Utc};
use loco_rs::{auth::jwt::JWT, hash, prelude::*};
use sea_orm::{
    DatabaseTransaction, DbBackend, FromQueryResult, Statement, TransactionSession, sea_query::Func,
};

pub use super::_entities::users::{self, ActiveModel, Column, Entity, Model};
use super::{
    activity_log::{self, EntityRef, Entry, RequestContext},
    user_roles,
};
use crate::domain::{
    activity::{Action, Actor},
    authz::AccountGrants,
    email_address,
    password_hash::{self, DUMMY_PASSWORD_HASH, PasswordHashKind, legacy_password_must_change},
    seed::SeedAccount,
    session::{AUTH_TIME_CLAIM, AccountState, SCOPE_CLAIM, SessionScope, TOKEN_VERSION_CLAIM},
    totp::TotpSecret,
    totp_secret::{self, DecryptError, TotpEncryptionKey},
};

/// The roles and the extra grants of the account `$1`, in one statement:
/// each row carries either a `user_roles.role_id` or a
/// `user_permissions.permission`, never both.
///
/// One statement on purpose: PostgreSQL gives a statement one snapshot, so
/// the two lists always come from the same committed state. Two separate
/// queries could read the roles from before a concurrent role change and the
/// grants from after it, a mixture that neither state held, which a guard
/// such as "grant only what you hold" (ADR 0007 §6) must never see. The
/// primary keys' indexes, whose leading column is `user_id`, serve both
/// halves.
const GRANTS_QUERY: &str = "SELECT role_id, NULL::text AS permission FROM user_roles \
                            WHERE user_id = $1 \
                            UNION ALL \
                            SELECT NULL::bigint AS role_id, permission FROM user_permissions \
                            WHERE user_id = $1";

/// A row of [`GRANTS_QUERY`]: a role id or a permission identifier.
#[derive(Debug, FromQueryResult)]
struct StoredGrant {
    /// A `user_roles.role_id`, or `None` on a grant's row.
    role_id: Option<i64>,
    /// A `user_permissions.permission`, or `None` on a role's row.
    permission: Option<String>,
}

/// The resource name of an account, the `entity_type` of the activity
/// entries about one (ADR 0011 §4, §5).
pub const ENTITY_TYPE: &str = "users";

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

/// `GREATEST(last_login_at, CURRENT_TIMESTAMP)`: the new value of
/// `last_login_at` on a sign-in ([`ActiveModel::record_sign_in`] says why it
/// never moves back). PostgreSQL's `GREATEST` ignores a NULL, so an account
/// that never signed in gets the transaction's start.
fn latest_login_at() -> Expr {
    Func::greatest([Expr::col(Column::LastLoginAt), Expr::current_timestamp()]).into()
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

/// What [`check_stored_password`] found for a password and a stored hash.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PasswordCheck {
    /// The password matches the stored Argon2id hash.
    Matches,
    /// The password matches the stored legacy bcrypt hash (`$2y$`, `$2a$`
    /// or `$2b$`). A sign-in that this lets in replaces the hash with an
    /// Argon2id hash of the same password (ADR 0006 §4).
    MatchesLegacy,
    /// The password matches nothing that is stored: it is wrong, the
    /// account has no password, or the stored value is not a hash that
    /// backend3 verifies.
    Fails,
}

impl PasswordCheck {
    /// Whether the password matched, whatever kind of hash it matched.
    #[must_use]
    pub fn matched(self) -> bool {
        self != Self::Fails
    }
}

/// Checks `password` against the stored hash `stored`, with the verifier
/// that the hash's kind needs ([`password_hash::classify`]).
///
/// - `stored`: the account's `users.password_hash`, or `None` for an account
///   that has no password (pending, or anonymised).
/// - `password`: the plain-text password to check.
///
/// - An Argon2id hash is verified with `loco_rs::hash` and the parameters
///   it carries: [`PasswordCheck::Matches`] or [`PasswordCheck::Fails`].
/// - A legacy bcrypt hash is verified with the `bcrypt` crate and the cost
///   it carries: [`PasswordCheck::MatchesLegacy`] or
///   [`PasswordCheck::Fails`]. bcrypt reads at most the first 72 bytes of a
///   password, and the crate's `verify` truncates a longer one there, as
///   Laravel's `password_verify` did when the hash was made, so a legacy
///   password longer than 72 bytes still verifies (ADR 0006 §4: the people
///   who sign in today keep their password).
///
/// Every other case is [`PasswordCheck::Fails`] after verifying
/// [`DUMMY_PASSWORD_HASH`], so it costs what a wrong password costs for a
/// `hash_password` hash:
///
/// - `None`;
/// - a value of an unknown kind, including the other Argon2 variants, which
///   the Argon2 verifier would otherwise accept;
/// - a value with a bcrypt prefix that the bcrypt crate cannot read (a cost
///   outside 4 to 31, a salt or hash of the wrong length or alphabet).
///
/// A malformed `$argon2id$` value never matches either, but it fails without
/// the Argon2 work, because only the verifier parses the PHC string. Nothing
/// stores one: the only Argon2id values backend3 writes are
/// [`hash_password`] output.
///
/// It writes nothing: the sign-in stores the rehash of a legacy password
/// ([`Model::authenticate`], [`ActiveModel::record_sign_in`]).
#[must_use]
pub fn check_stored_password(stored: Option<&str>, password: &str) -> PasswordCheck {
    let Some(stored) = stored else {
        verify_dummy_password(password);
        return PasswordCheck::Fails;
    };
    match password_hash::classify(stored) {
        PasswordHashKind::Argon2id => {
            if hash::verify_password(password, stored) {
                PasswordCheck::Matches
            } else {
                PasswordCheck::Fails
            }
        }
        PasswordHashKind::Bcrypt => match bcrypt::verify(password, stored) {
            Ok(true) => PasswordCheck::MatchesLegacy,
            Ok(false) => PasswordCheck::Fails,
            // The crate refuses the value before any bcrypt work, so the
            // dummy pays for it, as for a value of an unknown kind.
            Err(_) => {
                verify_dummy_password(password);
                PasswordCheck::Fails
            }
        },
        PasswordHashKind::Unknown => {
            verify_dummy_password(password);
            PasswordCheck::Fails
        }
    }
}

/// Whether `password` matches the stored hash `stored`, an Argon2id hash or
/// a legacy bcrypt one: [`check_stored_password`] without the kind, with the
/// same cost on every failure.
///
/// - `stored`: the account's `users.password_hash`, or `None` for an account
///   that has no password (pending, or anonymised).
/// - `password`: the plain-text password to check.
#[must_use]
pub fn verify_stored_password(stored: Option<&str>, password: &str) -> bool {
    check_stored_password(stored, password).matched()
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

/// Why [`ActiveModel::record_sign_in`] refused an account before it wrote
/// anything. Both are programming errors of the caller, never of the request.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum RecordSignInError {
    /// The active model has no `id`, so no row can be addressed.
    #[error(
        "record_sign_in needs the account's id: build the active model from a stored row \
         (`into_active_model`)"
    )]
    NoId,
    /// The caller set `last_login_at` itself.
    #[error(
        "record_sign_in sets last_login_at itself, from PostgreSQL's clock; leave it unset on \
         the active model (ADR 0006 §4; recipe 7 step 6 of the project skill)"
    )]
    LastLoginAtSet,
    /// The active model does not hold the stored `token_version` unchanged,
    /// so the write cannot check that the session it records still holds.
    #[error(
        "record_sign_in matches the token_version that the new session token carries: build the \
         active model from the row the sign-in checked (`into_active_model`) and leave \
         token_version unchanged (ADR 0006 §2)"
    )]
    TokenVersionNotStored,
}

/// What [`Model::authenticate`] found for a password sign-in, the browser's
/// (`/auth/login`) or a script's (`/auth/token`) (ADR 0006 §3, §4).
///
/// Both sign-ins answer every outcome but [`SignInOutcome::Accepted`] with
/// the same `401`; the outcomes differ only in what the activity log records
/// (ADR 0011 §5): a failed sign-in of a known account writes `login_failed`,
/// and one that names no known account writes nothing (research: DECISIONS
/// R9 (D09), "Known accounts only").
// `Debug` prints the account's password hash in `Accepted`, as `Model`'s
// own does: for test assertions only, never for logging.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SignInOutcome {
    /// The password is right and the account may start a full `admin`
    /// session. Boxed: a whole account row is some 300 bytes, and the other
    /// outcomes are an id or nothing (clippy's `large_enum_variant`).
    Accepted(Box<AcceptedSignIn>),
    /// A live account has the address, but the sign-in failed: the password
    /// is wrong, the account has none yet, it is inactive, or, until the
    /// two-factor step and the forced password change of ADR 0006 §4 exist,
    /// it has two-factor on, must change its password, or signed in with a
    /// legacy password that ADR 0006 §6 sends to the password change
    /// (research: DECISIONS R15 (D74); [`legacy_password_must_change`]).
    Refused {
        /// The account's id, for its `login_failed` entry
        /// ([`record_failed_sign_in`]).
        account_id: i64,
    },
    /// No account that the sign-in can find has the address: no account at
    /// all, one that is in the trash or anonymised, or an address that no
    /// account can hold ([`Model::find_live_by_email`]). The address is
    /// never recorded: a mistyped address is personal data that the
    /// append-only log could never drop again.
    UnknownAccount,
}

impl SignInOutcome {
    /// The account that may start a session, if the sign-in succeeded.
    #[must_use]
    pub fn accepted(self) -> Option<Model> {
        match self {
            Self::Accepted(sign_in) => Some(sign_in.user),
            Self::Refused { .. } | Self::UnknownAccount => None,
        }
    }
}

/// A sign-in that [`Model::authenticate`] accepted: the account as the
/// sign-in checked it, and the new hash that replaces a legacy one.
// `Debug` prints both hashes, as `Model`'s own does: for test assertions
// only, never for logging.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AcceptedSignIn {
    /// The account, as read and checked by the sign-in. Its
    /// `token_version` is the `tv` of the token that the sign-in issues.
    pub user: Model,
    /// For an account that still had its legacy bcrypt hash, the Argon2id
    /// hash of the password that matched it ([`hash_password`], a fresh
    /// salt), which the sign-in stores in its place (ADR 0006 §4); `None`
    /// for an account that already has an Argon2id hash.
    pub rehashed_password: Option<String>,
}

impl IntoActiveModel<ActiveModel> for AcceptedSignIn {
    /// The active model that [`ActiveModel::record_sign_in`] takes for this
    /// sign-in: the checked account, every column unchanged but
    /// `password_hash`, which holds the rehash of a legacy password.
    ///
    /// `record_sign_in` writes the rehash in its one `UPDATE`, which matches
    /// the row only while it still has the `token_version` that the sign-in
    /// checked. A password that an `edit users` holder replaced in the
    /// meantime ended every session, and so moved the version (ADR 0006 §2):
    /// such a sign-in writes nothing and fails, and the new password is never
    /// overwritten with the old one's rehash.
    fn into_active_model(self) -> ActiveModel {
        let mut active = self.user.into_active_model();
        if let Some(rehashed) = self.rehashed_password {
            active.password_hash = ActiveValue::Set(Some(rehashed));
        }
        active
    }
}

/// The activity entry of the account event `action` about the account
/// `account_id`, written for `request` (ADR 0011 §5).
///
/// - `action`: the account action; [`activity_log::record`] refuses the
///   address on any other.
/// - `account_id`: the account the event is about, the entry's record.
/// - `request`: the request id and the client address.
///
/// The entry names the account as its record (`users/<id>`) and carries no
/// label: an account is a person, and the log keeps only the id
/// (ADR 0011 §5).
fn account_entry(action: Action, account_id: i64, request: RequestContext) -> Entry {
    Entry {
        entity: Some(EntityRef::record(ENTITY_TYPE, account_id)),
        request_id: request.request_id,
        ip: request.ip,
        ..Entry::new(action)
    }
}

/// Records a failed sign-in of a known account: one `login_failed` entry
/// about the account, and returns it (ADR 0011 §5).
///
/// - `db`: the application's connection pool. Never the request's
///   transaction: a failed sign-in changes no data, so its entry is written
///   on a connection of its own and commits at once, and a rollback of the
///   request's other work cannot take it back (ADR 0011 §5). The type takes
///   a pool only, so a transaction cannot be passed by mistake.
/// - `account_id`: the account the sign-in named, from
///   [`SignInOutcome::Refused`].
/// - `request`: the request id and the client address of the attempt.
///
/// The actor is [`Actor::System`]: the caller has not signed in, so no
/// account is the one who acted, and the entry names the account it is
/// about as its record instead. The anonymisation of an account finds these
/// entries by their record (ADR 0011 §6, "`login_failed` entries for the
/// account"), beside the entries whose actor the account is.
///
/// ```no_run
/// # async fn sign_in(db: &sea_orm::DatabaseConnection) -> loco_rs::model::ModelResult<()> {
/// use backend3::models::{activity_log::RequestContext, users::record_failed_sign_in};
///
/// record_failed_sign_in(db, 7, RequestContext::default()).await?;
/// # Ok(())
/// # }
/// ```
///
/// A transaction does not compile:
///
/// ```compile_fail
/// # async fn sign_in(txn: &sea_orm::DatabaseTransaction) -> loco_rs::model::ModelResult<()> {
/// use backend3::models::{activity_log::RequestContext, users::record_failed_sign_in};
///
/// record_failed_sign_in(txn, 7, RequestContext::default()).await?;
/// # Ok(())
/// # }
/// ```
///
/// # Errors
///
/// [`ModelError::DbErr`] when the insert fails. `entity_id` has no foreign
/// key (one column serves every table), so nothing checks that the id names
/// an account: pass only the id that [`Model::authenticate`] found.
pub async fn record_failed_sign_in(
    db: &DatabaseConnection,
    account_id: i64,
    request: RequestContext,
) -> ModelResult<activity_log::Model> {
    activity_log::record(
        db,
        Actor::System,
        account_entry(Action::LoginFailed, account_id, request),
    )
    .await
}

impl ActiveModel {
    /// Records a completed sign-in on `txn`: sets `last_login_at`, stores the
    /// rehash of a legacy password, and writes the `login` entry, and returns
    /// the account as stored afterwards, or
    /// `None` when the account no longer may start the session that the
    /// sign-in issues (ADR 0006 §1, §2, §4, §7; ADR 0011 §5).
    ///
    /// - `txn`: the transaction of the sign-in's writes, so that the new
    ///   `last_login_at` and its entry commit or roll back together
    ///   (ADR 0011 §5). The caller commits it.
    /// - `request`: the request id and the client address of the sign-in.
    ///
    /// Call it only when the full `admin`-scope session is issued
    /// (ADR 0006 §4, "`last_login_at`"), by the browser's sign-in or by a
    /// script's (`/auth/token`, ADR 0006 §3): a session of another scope is
    /// not a completed sign-in. Build the active model from the
    /// [`AcceptedSignIn`] (`into_active_model`), whose `token_version` is the
    /// `tv` of the token the sign-in issues.
    ///
    /// **The account must still be the one that was checked.** The `UPDATE`
    /// matches the row only while it is live (neither in the trash nor
    /// anonymised), active, and still at that `token_version`, the checks
    /// that the admin session check (`open_session` in
    /// `controllers::admin::session`, which the admin route table's guard
    /// calls) makes on every request (ADR 0006 §1). Every event that ends
    /// sessions increments `token_version` (ADR 0006 §2), so an account that
    /// an `edit users` holder deactivated, trashed or gave a new password
    /// while its password was being verified matches nothing: the method
    /// then writes nothing and returns `None`, and the sign-in fails like any
    /// other. Without the check it would record a sign-in, for good, whose
    /// cookie the next request refuses.
    ///
    /// `last_login_at` comes from PostgreSQL's clock: `GREATEST(last_login_at,
    /// CURRENT_TIMESTAMP)`. `CURRENT_TIMESTAMP` is the start of `txn`, the
    /// instant that the `login` entry records as `occurred_at` (recipe 7
    /// step 6 of the project skill; this reading extends research: DECISIONS
    /// R15 (D90), which settles the audit columns and the log's instants,
    /// to `last_login_at`, so that the column and the entry agree).
    /// `GREATEST` keeps a later instant that is already stored (PostgreSQL's
    /// `GREATEST` ignores a NULL, so the first sign-in stores the instant):
    /// two sign-ins of one account may commit in the other order than they
    /// began, and the second `UPDATE`, which waits for the first one's row
    /// lock and then reads its committed value, must not move the column
    /// back. The column is therefore always the instant of the account's
    /// newest `login` entry, the one that the users list shows as "Utolsó
    /// belépés".
    ///
    /// The changes that this active model holds, if any, are written in the
    /// same `UPDATE`, as [`ActiveModel::revoke_sessions`] does: the Argon2id
    /// hash that replaces a legacy bcrypt hash (ADR 0006 §4), which the
    /// active model of an [`AcceptedSignIn`] holds, so the rehash is stored
    /// exactly when the sign-in is, and only for the account version that was
    /// checked. It writes nothing else: no revision, and neither `updated_at`
    /// nor `updated_by` unless the caller set them, because a sign-in and its
    /// rehash are account events, not a content update (ADR 0011 §4). The
    /// `UPDATE` goes past the model's save hooks.
    ///
    /// The entry's actor is the account itself, which has just signed in,
    /// and its record is the account (`users/<id>`).
    ///
    /// # Errors
    ///
    /// [`ModelError::Any`] with [`RecordSignInError::NoId`] when the active
    /// model has no `id`, with [`RecordSignInError::LastLoginAtSet`] when
    /// the caller set `last_login_at`, and with
    /// [`RecordSignInError::TokenVersionNotStored`] when `token_version` is
    /// not the stored value, unchanged; all before any query.
    /// [`ModelError::DbErr`] when a statement fails, and the error of
    /// [`activity_log::record`] when the entry is refused; the caller then
    /// drops `txn`, which rolls `last_login_at` back.
    pub async fn record_sign_in(
        mut self,
        txn: &DatabaseTransaction,
        request: RequestContext,
    ) -> ModelResult<Option<Model>> {
        let id = match self.id {
            ActiveValue::Set(id) | ActiveValue::Unchanged(id) => id,
            ActiveValue::NotSet => return Err(ModelError::wrap(RecordSignInError::NoId)),
        };
        if self.last_login_at.is_set() {
            return Err(ModelError::wrap(RecordSignInError::LastLoginAtSet));
        }
        // `Unchanged` only: a value the caller set, or left out, is not the
        // stored version that the issued token carries.
        let ActiveValue::Unchanged(token_version) = self.token_version else {
            return Err(ModelError::wrap(RecordSignInError::TokenVersionNotStored));
        };
        // The id addresses the row; it is never part of the `SET` list. The
        // `token_version` is `Unchanged`, so it is not part of it either.
        self.id = ActiveValue::NotSet;
        let Some(user) = Entity::update_many()
            .set(self)
            .col_expr(Column::LastLoginAt, latest_login_at())
            .filter(Column::Id.eq(id))
            .filter(Column::DeletedAt.is_null())
            .filter(Column::AnonymisedAt.is_null())
            .filter(Column::Active.eq(true))
            .filter(Column::TokenVersion.eq(token_version))
            .exec_with_returning(txn)
            .await?
            .into_iter()
            .next()
        else {
            return Ok(None);
        };
        activity_log::record(
            txn,
            Actor::User(id),
            account_entry(Action::Login, id, request),
        )
        .await?;
        Ok(Some(user))
    }

    /// Logs the account out on `txn`: ends every session of the account
    /// ([`ActiveModel::revoke_sessions`]) and writes the `logout` entry, and
    /// returns the account as stored afterwards (ADR 0006 §2; ADR 0011 §5).
    ///
    /// - `txn`: the transaction of the logout's writes, so that the
    ///   revocation and its entry commit or roll back together (ADR 0011
    ///   §5). The caller commits it.
    /// - `request`: the request id and the client address of the logout.
    ///
    /// Like the revocation, it writes no revision and leaves `updated_at`
    /// and `updated_by` alone (ADR 0011 §4). The entry's actor is the
    /// account that logs out, and its record is the account (`users/<id>`).
    ///
    /// # Errors
    ///
    /// The errors of [`ActiveModel::revoke_sessions`], before or instead of
    /// any entry, and the error of [`activity_log::record`] when the entry
    /// is refused; the caller then drops `txn`, which rolls the revocation
    /// back, so the sessions hold.
    pub async fn log_out(
        self,
        txn: &DatabaseTransaction,
        request: RequestContext,
    ) -> ModelResult<Model> {
        let user = self.revoke_sessions(txn).await?;
        activity_log::record(
            txn,
            Actor::User(user.id),
            account_entry(Action::Logout, user.id, request),
        )
        .await?;
        Ok(user)
    }

    /// Ends every session of this account: increments `token_version` by one
    /// and returns the account as stored afterwards (ADR 0006 §2).
    ///
    /// - `db`: the connection or transaction to write to.
    ///
    /// Every token carries the version it was issued at (`tv`), and the
    /// admin session check refuses one that differs from the stored
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

    /// Whether `password` matches this account's stored password, an
    /// Argon2id hash or a legacy bcrypt one, with the equal-cost failure
    /// paths of [`check_stored_password`].
    ///
    /// - `password`: the plain-text password to check.
    ///
    /// The account's state (active, trashed) is not checked here; sign-in
    /// checks it after the password, so a disabled account costs the same.
    /// Nothing is rehashed: only a sign-in replaces a legacy hash.
    #[must_use]
    pub fn verify_password(&self, password: &str) -> bool {
        verify_stored_password(self.password_hash.as_deref(), password)
    }

    /// Checks a password sign-in, the browser's or a script's, and returns
    /// its [`SignInOutcome`]: the account that may start a full `admin`
    /// session, or why the sign-in failed (ADR 0006 §3, §4).
    ///
    /// - `db`: the connection or transaction to query.
    /// - `email`: the address as the user typed it, matched without regard
    ///   to case ([`Model::find_live_by_email`]).
    /// - `password`: the password as typed.
    /// - `force_change_short_legacy_password`:
    ///   `settings.admin_auth.force_change_short_legacy_password`
    ///   (ADR 0006 §6, §8), passed in by the caller (ADR 0008 rule 2).
    ///
    /// Every failure costs one password verification ("Same answer for every
    /// failure"): an unknown, trashed or anonymised address, or one that no
    /// account can hold, verifies [`DUMMY_PASSWORD_HASH`]
    /// ([`SignInOutcome::UnknownAccount`]), and the account's state is
    /// checked only after its own password ([`check_stored_password`]), so a
    /// refused account costs what a wrong password for its stored hash
    /// costs. A sign-in of a live account fails
    /// ([`SignInOutcome::Refused`]) when the password is wrong, when the
    /// account has none, when it is inactive, and, until the two-factor step
    /// and the forced password change of ADR 0006 §4 exist, when it has
    /// two-factor turned on, must change its password, or signed in with a
    /// legacy password that [`legacy_password_must_change`] sends to the
    /// password change (ADR 0006 §6: shorter than 12 characters while the
    /// setting is on). Those accounts must never get a full `admin` session
    /// from the password alone, and they get the same failure as a wrong
    /// password until then (the maintainer's answer of 2026-10-25, research:
    /// DECISIONS R15 (D74), which this reading extends to the short legacy
    /// password); such an account keeps its bcrypt hash, so the rule still
    /// applies at its next sign-in.
    ///
    /// An accepted sign-in of an account that still has its legacy bcrypt
    /// hash carries the Argon2id hash of the password
    /// ([`AcceptedSignIn::rehashed_password`]), made here, where the
    /// password is at hand; [`ActiveModel::record_sign_in`] stores it with
    /// the sign-in (ADR 0006 §4: verified once, "then immediately" replaced).
    ///
    /// It writes nothing: the caller records the outcome
    /// ([`ActiveModel::record_sign_in`], [`record_failed_sign_in`]).
    ///
    /// The Argon2 and bcrypt work runs on the request's task, as in Loco's
    /// own sign-in handler.
    ///
    /// # Errors
    ///
    /// [`ModelError::DbErr`] when the query fails, and [`ModelError::Any`]
    /// when the rehash of a legacy password cannot be made, which with the
    /// hasher's default parameters only happens if the system random number
    /// generator fails. A failed sign-in is an `Ok` outcome, never an error.
    pub async fn authenticate<C>(
        db: &C,
        email: &str,
        password: &str,
        force_change_short_legacy_password: bool,
    ) -> ModelResult<SignInOutcome>
    where
        C: ConnectionTrait,
    {
        let user = match Self::find_live_by_email(db, email).await {
            Ok(user) => user,
            Err(ModelError::EntityNotFound) => {
                verify_dummy_password(password);
                return Ok(SignInOutcome::UnknownAccount);
            }
            Err(error) => return Err(error),
        };
        // The password first, before any state check could end the
        // evaluation, so an account that may not sign in costs what a wrong
        // password costs.
        let check = check_stored_password(user.password_hash.as_deref(), password);
        let legacy = check == PasswordCheck::MatchesLegacy;
        let short_legacy =
            legacy && legacy_password_must_change(password, force_change_short_legacy_password);
        let may_sign_in = user.active
            && user.totp_enabled_at.is_none()
            && !user.must_change_password
            && !short_legacy;
        if !(check.matched() && may_sign_in) {
            return Ok(SignInOutcome::Refused {
                account_id: user.id,
            });
        }
        let rehashed_password = if legacy {
            Some(hash_password(password)?)
        } else {
            None
        };
        Ok(SignInOutcome::Accepted(Box::new(AcceptedSignIn {
            user,
            rehashed_password,
        })))
    }

    /// Creates the seed account `account`, with its roles, unless an account
    /// already has its address, for `Hooks::seed` (`crate::domain::seed`).
    ///
    /// - `db`: the connection or transaction to write to; the method runs in
    ///   a transaction of its own (a savepoint inside a caller's
    ///   transaction).
    /// - `account`: the account to create; its password is hashed here, with
    ///   [`hash_password`] and a fresh salt, so no hash of it is ever
    ///   committed.
    ///
    /// The address is compared as sign-in compares it (`lower(email)`), over
    /// every row, trashed ones included, because the unique index
    /// `users_lower_email_key` covers them too. An account that has it is
    /// left exactly as it is, whatever its state and its roles, so seeding
    /// twice gives the result of seeding once, as `find_or_create_by` does in
    /// a Rails seed. To start over, recreate the database (`AGENTS.md`,
    /// "Commands").
    ///
    /// The new account is active, has a password and no two-factor, and needs
    /// no password change, so the browser sign-in gives it a full session. It
    /// holds `account.roles` (`user_roles`) and no extra grant; the account
    /// and its roles are written in one transaction, so no account is ever
    /// left without the roles the seed promises. `created_at` and
    /// `updated_at` come from PostgreSQL's clock, the columns' default
    /// (research: DECISIONS R15 (D90)), and `created_by` stays NULL: the
    /// system created it. No revision or activity entry is written: the seed
    /// fills only local development, test and E2E databases
    /// (`crate::domain::seed::check_environment`).
    ///
    /// The caller checks the environment first; this method does not see it.
    ///
    /// # Errors
    ///
    /// [`ModelError::Any`] when hashing fails; [`ModelError::DbErr`] when a
    /// query fails, including the unique violation of a concurrent seed that
    /// inserted the same address first. Nothing is written then.
    pub async fn seed_account<C>(db: &C, account: &SeedAccount) -> ModelResult<SeedOutcome>
    where
        C: ConnectionTrait + TransactionTrait,
    {
        let txn = db.begin().await?;
        if let Some(existing) = Entity::find()
            .filter(lower_email_equals(account.email))
            .one(&txn)
            .await?
        {
            txn.commit().await?;
            return Ok(SeedOutcome::AlreadyPresent(existing));
        }
        let created = ActiveModel {
            email: ActiveValue::Set(Some(account.email.to_owned())),
            name: ActiveValue::Set(account.name.to_owned()),
            password_hash: ActiveValue::Set(Some(hash_password(account.password)?)),
            ..Default::default()
        }
        .insert(&txn)
        .await?;
        for role in account.roles {
            user_roles::ActiveModel {
                user_id: ActiveValue::Set(created.id),
                role_id: ActiveValue::Set(role.id()),
            }
            .insert(&txn)
            .await?;
        }
        txn.commit().await?;
        Ok(SeedOutcome::Created(created))
    }

    /// The roles and the extra grants that this account holds, as stored now
    /// (ADR 0007 §3).
    ///
    /// - `db`: the connection or transaction to query.
    ///
    /// The admin session check (`open_session` in
    /// `controllers::admin::session`, which the admin route table's guard
    /// calls before the handler) calls it on every request, after the
    /// session rules, so a role or grant change applies on the account's
    /// next request (ADR 0007 §5, "Fresh data"; ADR 0006 §2); nothing is
    /// cached (ADR 0016). Both lists come from one statement, and so from one
    /// committed state ([`GRANTS_QUERY`] says why that matters). The account's
    /// state (trashed, inactive) is not checked here: the session check has
    /// done that.
    ///
    /// # Errors
    ///
    /// [`ModelError::DbErr`] when the query fails, and [`ModelError::Any`]
    /// with a [`crate::domain::authz::StoredGrantError`] when a stored role id
    /// or permission is not one that `crate::domain::authz` defines, which
    /// the schema's foreign key and CHECK rule out. Never
    /// [`ModelError::EntityNotFound`], which Loco would answer with `404`: an
    /// account without roles or grants has an empty [`AccountGrants`].
    pub async fn grants<C>(&self, db: &C) -> ModelResult<AccountGrants>
    where
        C: ConnectionTrait,
    {
        let rows = StoredGrant::find_by_statement(Statement::from_sql_and_values(
            DbBackend::Postgres,
            GRANTS_QUERY,
            [self.id.into()],
        ))
        .all(db)
        .await?;
        AccountGrants::from_stored(
            rows.iter().filter_map(|row| row.role_id),
            rows.iter().filter_map(|row| row.permission.as_deref()),
        )
        .map_err(ModelError::wrap)
    }

    /// Signs a full admin session token for this account (ADR 0006 §1).
    ///
    /// - `secret`: the base64 JWT secret, `auth.jwt.secret`.
    /// - `expiration`: the token's lifetime in seconds, `auth.jwt.expiration`.
    /// - `auth_time`: when the password step succeeded: at sign-in the app
    ///   clock's "now" that the caller read (ADR 0010 point 7), and at a
    ///   sliding refresh the `auth_time` of the session it renews, so the
    ///   absolute limit counts from the sign-in (ADR 0006 §1). The claim
    ///   holds it in whole seconds since the Unix epoch, a JWT `NumericDate`
    ///   like `exp`.
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
    /// The admin session check (`open_session` in
    /// `controllers::admin::session`) calls it on the row it has just read
    /// for the request, so the check always sees the stored state: a deactivation, a
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
