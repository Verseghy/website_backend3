//! The browser sign-in, `POST /api/admin/auth/login` (ADR 0006 §4), the
//! scripts' sign-in, `POST /api/admin/auth/token` (ADR 0006 §3), the
//! signed-in account, `GET /api/admin/auth/me` (ADR 0007 §7), and logout,
//! `POST /api/admin/auth/logout` (ADR 0006 §2).
//!
//! A correct e-mail address and password answer `200 {"step": "done"}` and
//! set the `__Host-admin_session` cookie with a full `admin` session token
//! ([`crate::domain::session_cookie`]). The token is never in the body
//! (ADR 0006 §3). Every credential failure answers the same
//! `401 invalid_credentials` envelope, sets no cookie and costs one password
//! verification ([`users::Model::authenticate`]): a wrong password, an
//! unknown address, an account that may not sign in.
//!
//! A script signs in at `/auth/token` with the same `{email, password}` and
//! gets `200 {token, expires_at}`: a full `admin` session token for its
//! `Authorization: Bearer` header, which it never gets from the browser
//! sign-in, and when that token expires. No cookie is set. The token is the
//! session token of the browser sign-in in every other respect: the same
//! two-hour lifetime, the same `tv` check, the same permission checks, and
//! never a `password_change` or `mfa` token. It is never refreshed, so the
//! script signs in again (ADR 0006 §3; the explicit refresh answers it `204`
//! without a cookie, research: DECISIONS R35 (D154)). Its refusals are the
//! browser sign-in's, byte for byte: the one `401` for every credential
//! failure, the `422` and the `413`. Requests with the Bearer token are
//! exempt from the `Origin` check of cookie writes, as long as they carry no
//! credential cookie (ADR 0044 §2 and its reading 1, research: DECISIONS R35
//! (D151)), so a script needs no `Origin` header. An account
//! that has two-factor on or must change its password is refused like a wrong
//! password until the token's TOTP code and the `password_change_required`
//! answer of ADR 0006 §3 exist (research: DECISIONS R15 (D74)).
//!
//! Both sign-ins check the password the same way. A legacy bcrypt hash
//! (`$2y$`, `$2a$`, `$2b$`), which the importer copies unchanged, is
//! verified with the bcrypt crate and, when the sign-in succeeds, replaced
//! by an Argon2id hash of the same password in the sign-in's own `UPDATE`:
//! an account event, so no revision and no change of `updated_at`
//! (ADR 0006 §4, ADR 0011 §4). The next sign-in verifies the Argon2id hash.
//! A legacy password shorter than 12 characters opens no session while
//! `admin_auth.force_change_short_legacy_password` is on, and keeps its
//! hash (ADR 0006 §6; [`users::Model::authenticate`] says why). The setting
//! comes from [`SignInRules`], which `App::after_context` stores.
//!
//! A body that is not a JSON `{email, password}` is not a credential
//! failure: it answers `422 invalid_body`, or `413 body_too_large` above the
//! request body limit ([`super::json_rejection_response`]; the maintainer's
//! answer of 2026-10-25, research: DECISIONS R15 (D74)). It is refused before
//! any lookup, so its answer is the same for every address and tells nothing
//! about the accounts. ADR 0006 §4's "same answer for every failure" covers
//! the credential failures.
//!
//! `auth/me` answers the account of the request's admin session
//! ([`super::session::AdminSession`]) as a [`MeResponse`], with the roles,
//! effective permissions and admin flag that the session read for the
//! request, and the access map computed from them, from which the admin's
//! `authProvider.canAccess` answers (ADR 0007 §7). It is one of the
//! three routes that a `password_change` session may reach (ADR 0006 §4), so
//! the admin UI can tell such a session to set a new password. Without a
//! valid session it answers `401 unauthorized`.
//!
//! Logout ends every session of the account, not only the one it is sent
//! with (ADR 0006 §2, "Logout therefore signs the user out on every
//! device"): it increments `token_version`
//! ([`users::ActiveModel::revoke_sessions`]), so every token issued before
//! it fails on its next request, and answers `204 No Content` with a
//! `Set-Cookie` that deletes the session cookie in this browser
//! ([`crate::domain::session_cookie::cleared_session_cookie`]). It is
//! `POST` only: a `GET` would let a link or an image sign the user out, so
//! every other method answers axum's `405` with `Allow: POST`. It is the
//! third route that a `password_change` session may reach (ADR 0006 §4).
//! Without a valid session it answers the route table's `401 unauthorized`
//! (ADR 0006 §1 refuses every such session with 401, and ADR 0007 §5 gives
//! "No valid session: 401" for every admin route), sets no cookie and
//! touches no account; the admin UI treats that answer as already signed
//! out. A request body, if any, is not read.
//!
//! Sign-in and logout write the account entries of the activity log
//! (ADR 0011 §5), each with the request id and the client address
//! ([`ClientContext`]), about the account, and without touching `updated_at`
//! or writing a revision (ADR 0011 §4):
//!
//! - a sign-in that issues the full `admin` session, the browser's or a
//!   script's, sets `last_login_at`, stores the rehash of a legacy password
//!   and writes `login`, on one transaction with nothing else in it
//!   ([`users::ActiveModel::record_sign_in`]); if a write fails, all roll
//!   back and the answer is a `500` without the cookie or the token. A
//!   script's sign-in is a sign-in like the browser's: it shows in
//!   `last_login_at`, by which stale accounts are found (ADR 0006 §7). An
//!   account that was deactivated, trashed or had its sessions ended while
//!   its password was being verified gets no session and no rehash: the
//!   sign-in fails like any other;
//! - a failed sign-in of an account that the sign-in finds writes
//!   `login_failed` ([`users::record_failed_sign_in`]) on a connection of its
//!   own, never on a transaction of the request, so that no rollback of the
//!   request's work can take it back; a failed sign-in that names no such
//!   account, and a body that is refused before any lookup, write nothing
//!   (research: DECISIONS R9 (D09)). The answer stays the one `401`, also
//!   when the entry cannot be written: a `500` there, and the `401` of an
//!   unknown address beside it, would tell whoever can make the database
//!   refuse writes (a read-only database, a full disk) which addresses
//!   belong to an account (ADR 0006 §4, "Same answer for every failure";
//!   ADR 0023 §3). The lost entry is logged as an error instead, with the
//!   account's id and the request id, never the typed address;
//! - a logout writes `logout` on the transaction of its revocation
//!   ([`users::ActiveModel::log_out`]), so a failed entry keeps every session
//!   and the cookie.
//!
//! The explicit sliding refresh, `POST /api/admin/auth/refresh`
//! (ADR 0006 §1), renews the full `admin` session of the session cookie:
//! `204 No Content` with a new `__Host-admin_session` cookie that expires
//! two hours later, whatever was left of the old one, but never later than
//! the session's `auth_time` plus `admin_auth.session_absolute_max_secs`
//! (12 hours). The new token keeps the `auth_time`, so the limit counts
//! from the sign-in. The admin UI calls it on typing or clicking at most
//! every 10 minutes (`admin/src/auth/activity-refresh.ts`). At the absolute
//! limit, and for a Bearer token, which is never refreshed (ADR 0006 §3),
//! it answers `204` without a cookie: the session keeps the token it has
//! and ends with it. The maintainer kept both answers as built on
//! 2026-11-15, "Keep as built: 204 without a cookie" for a Bearer token and
//! "Keep 204 without a cookie (as built)" at the absolute limit (research:
//! DECISIONS R35 (D154, D155); ADR 0006 §1 quotes the questions). A
//! `password_change` session gets the route table's
//! `401`, since ADR 0006 §4 opens only three routes to it. The passive
//! refresh, which every other admin route gets from the route table's
//! guard, is the same renewal; its arithmetic for both is
//! [`crate::domain::session`]. A request body, if any, is not read.

use axum::{
    Json,
    extract::rejection::JsonRejection,
    http::{StatusCode, header::SET_COOKIE},
};
use chrono::{DateTime, Utc};
use loco_rs::{auth::jwt::JWT, config::JWT as JwtConfig, prelude::*};

use super::{
    client_context::ClientContext,
    error_response, json_rejection_response,
    session::{
        AdminOrPasswordChange, AdminSession, renewed_session_cookie, session_limits, session_times,
    },
};
use crate::{
    app::clock,
    domain::{
        authz::RESOURCE_ACCESS,
        session::explicit_renewal,
        session_cookie::{cleared_session_cookie, session_cookie},
    },
    dtos::admin::{
        auth::{LoginRequest, LoginResponse, LoginStep, MeResponse, TokenRequest, TokenResponse},
        error::ErrorCode,
    },
    models::{
        activity_log::RequestContext,
        users::{self, SignInOutcome},
    },
    settings::Settings,
};

/// What the password sign-ins take from the settings: the legacy password
/// rule of ADR 0006 §6, §8.
///
/// `App::after_context` builds it once from the validated settings and puts
/// it into the shared store; the sign-ins read it with [`sign_in_rules`] and
/// pass the value to [`users::Model::authenticate`] (ADR 0008 rules 2 and
/// 3).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SignInRules {
    /// `settings.admin_auth.force_change_short_legacy_password`: a legacy
    /// password shorter than 12 characters opens no full session.
    pub force_change_short_legacy_password: bool,
}

impl SignInRules {
    /// The rules of `settings`.
    ///
    /// - `settings`: the validated settings of the process.
    #[must_use]
    pub fn new(settings: &Settings) -> Self {
        Self {
            force_change_short_legacy_password: settings
                .admin_auth
                .force_change_short_legacy_password,
        }
    }
}

/// The sign-in rules of this app context.
///
/// - `ctx`: the request's app context.
///
/// # Errors
///
/// [`Error::Message`] when the context carries none: every context Loco
/// builds has them (`App::after_context`), one built by hand does not.
pub fn sign_in_rules(ctx: &AppContext) -> Result<SignInRules> {
    ctx.shared_store.get::<SignInRules>().ok_or_else(|| {
        Error::string(
            "the sign-in rules are missing from the shared store; App::after_context registers \
             them, so this context was not built by App::boot",
        )
    })
}

/// `POST /api/admin/auth/login`: signs in with an e-mail address and a
/// password.
///
/// - `ctx`: the application context; the handler takes the database, the app
///   clock, the sign-in rules and the `auth.jwt` configuration from it.
/// - `client`: the request id and the client address, for the activity
///   entry.
/// - `body`: the JSON body, or why it could not be read.
///
/// A body that is not a JSON `{email, password}` answers `422 invalid_body`,
/// and one above the request body limit `413 body_too_large`, before any
/// lookup (the module documentation says why). A credential failure answers
/// `401 invalid_credentials`, after the `login_failed` entry of a known
/// account ([`refuse_known_account`]). A success sets `last_login_at`,
/// replaces a legacy password hash and writes the `login` entry before it
/// answers `{"step": "done"}` with the session cookie
/// ([`sign_in_with_password`]).
///
/// # Errors
///
/// An internal error (`500`) when the sign-in succeeds but its writes fail
/// (a refused `login` entry included), when the account lookup fails, when
/// the context has no app clock, no sign-in rules or no `auth.jwt` section,
/// when the rehash of a legacy password cannot be made, or when the token
/// or its cookie cannot be built (a secret that is not base64). A failed
/// sign-in never answers `500` for its own entry: it answers the one `401`
/// (the module documentation says why).
#[debug_handler]
pub async fn login(
    State(ctx): State<AppContext>,
    ClientContext(client): ClientContext,
    body: Result<Json<LoginRequest>, JsonRejection>,
) -> Result<Response> {
    let request = match body {
        Ok(Json(request)) => request,
        Err(rejection) => return Ok(json_rejection_response(&rejection)),
    };
    sign_in_with_password(
        &ctx,
        client,
        &request.email,
        &request.password,
        |token, jwt| {
            // The token travels only in the `HttpOnly` cookie, never in the
            // body (ADR 0006 §3).
            let cookie = session_cookie(token, jwt.expiration).map_err(Error::wrap)?;
            format::render()
                .header(SET_COOKIE, cookie)
                .json(LoginResponse {
                    step: LoginStep::Done,
                })
        },
    )
    .await
}

/// `POST /api/admin/auth/token`: signs a script in with an e-mail address
/// and a password, and answers a Bearer token of a full `admin` session in
/// the body (ADR 0006 §3).
///
/// - `ctx`: the application context; the handler takes the database, the app
///   clock, the sign-in rules and the `auth.jwt` configuration from it.
/// - `client`: the request id and the client address, for the activity
///   entry.
/// - `body`: the JSON body, or why it could not be read.
///
/// It checks and records the sign-in exactly as [`login`] does
/// ([`sign_in_with_password`]), so every refusal is the browser sign-in's,
/// byte for byte, and a success sets `last_login_at`, replaces a legacy
/// password hash and writes the `login` entry. The answer is
/// `200 {token, expires_at}` ([`TokenResponse`]) and sets no cookie: the
/// token is for the script's `Authorization: Bearer` header, which the
/// route table's guard never refreshes (ADR 0006 §3). `expires_at` is the
/// token's own `exp`, read back from the token that Loco signed, because
/// Loco counts the lifetime from the system time (recipe 7 step 4 of the
/// project skill). The route is `Public`: a script holds no session yet,
/// and without a credential cookie the `Origin` check does not apply to it
/// (ADR 0044 §2).
///
/// # Errors
///
/// The errors of [`login`], and an internal error when the signed token
/// cannot be read back for its `exp`, which only a defect can cause.
#[debug_handler]
pub async fn token(
    State(ctx): State<AppContext>,
    ClientContext(client): ClientContext,
    body: Result<Json<TokenRequest>, JsonRejection>,
) -> Result<Response> {
    let request = match body {
        Ok(Json(request)) => request,
        Err(rejection) => return Ok(json_rejection_response(&rejection)),
    };
    sign_in_with_password(
        &ctx,
        client,
        &request.email,
        &request.password,
        |token, jwt| {
            let expires_at = token_expiry(&jwt.secret, token)?;
            format::json(TokenResponse {
                token: token.to_owned(),
                expires_at,
            })
        },
    )
    .await
}

/// Checks a password sign-in, signs a full `admin` session token, builds the
/// answer with `answer` and records the sign-in; the one path of [`login`]
/// and [`token`] (ADR 0006 §3, §4).
///
/// - `ctx`: the application context: the database, the app clock, the
///   sign-in rules and the `auth.jwt` configuration.
/// - `client`: the request id and the client address, for the activity
///   entry.
/// - `email`: the address as typed.
/// - `password`: the password as typed.
/// - `answer`: builds the success answer from the signed token and the
///   `auth.jwt` configuration it was signed with: the session cookie of the
///   browser, or the token in the body for a script.
///
/// A credential failure answers the one `401 invalid_credentials`, after the
/// `login_failed` entry of a known account ([`refuse_known_account`]). The
/// token and the answer are built before anything is written, so that a
/// sign-in is recorded only when its answer can be sent. Then
/// `last_login_at`, the rehash of a legacy password and the `login` entry
/// are written on one transaction ([`users::ActiveModel::record_sign_in`]):
/// a failed write drops it, which rolls every write back, and the `500`
/// carries neither the cookie nor the token. An account that changed
/// between the password check and the write gets the failure's `401` and
/// its `login_failed` entry, and no rehash.
///
/// # Errors
///
/// The internal errors that [`login`] lists, and those of `answer`.
async fn sign_in_with_password<F>(
    ctx: &AppContext,
    client: RequestContext,
    email: &str,
    password: &str,
    answer: F,
) -> Result<Response>
where
    F: FnOnce(&str, &JwtConfig) -> Result<Response>,
{
    let rules = sign_in_rules(ctx)?;
    let outcome = users::Model::authenticate(
        &ctx.db,
        email,
        password,
        rules.force_change_short_legacy_password,
    )
    .await?;
    let sign_in = match outcome {
        SignInOutcome::Accepted(sign_in) => *sign_in,
        SignInOutcome::Refused { account_id } => {
            return Ok(refuse_known_account(&ctx.db, account_id, client).await);
        }
        SignInOutcome::UnknownAccount => return Ok(error_response(ErrorCode::InvalidCredentials)),
    };
    let account_id = sign_in.user.id;
    // `auth_time` is the app clock's "now" (ADR 0010 point 7). Loco's `exp`
    // still reads the system time (recipe 7 step 4 of the project skill); in
    // production the app clock is the system clock, so the two agree.
    let now = clock(ctx)?.now();
    let jwt = ctx.config.get_jwt_config()?;
    let token = sign_in
        .user
        .generate_session_token(&jwt.secret, jwt.expiration, now)?;
    let response = answer(&token, jwt)?;
    // Every token that a password sign-in issues is a full `admin` one, the
    // only kind that sets `last_login_at` (ADR 0006 §4); the active model
    // carries the rehash of a legacy password, if any.
    let txn = ctx.db.begin().await?;
    let recorded = sign_in
        .into_active_model()
        .record_sign_in(&txn, client.clone())
        .await?;
    if recorded.is_none() {
        // The account was deactivated, trashed or had its sessions ended
        // (a new password included) between the password check and this
        // write, so the token above is already void: the sign-in failed,
        // and is answered and recorded as a failure. Nothing was written on
        // `txn`, the rehash included.
        txn.rollback().await?;
        return Ok(refuse_known_account(&ctx.db, account_id, client).await);
    }
    txn.commit().await?;
    Ok(response)
}

/// When `token` expires: its `exp` claim, read back from the token.
///
/// - `secret`: the `auth.jwt` secret that signed the token.
/// - `token`: a session token that [`users::Model::generate_session_token`]
///   has just signed.
///
/// Loco's `JWT::generate_token` adds the lifetime to the system time, which
/// it reads itself, and returns only the token (recipe 7 step 4 of the
/// project skill), so the token is the one place that holds the instant the
/// script's token stops working. Validating it again is cheap, and the
/// claims come out as the session check reads them ([`session_times`]).
///
/// # Errors
///
/// An internal error when the token does not validate with `secret`, has no
/// integer `exp` or `auth_time`, or has an `exp` outside chrono's range.
/// None of them can happen to a token that was signed a moment ago with the
/// same secret.
fn token_expiry(secret: &str, token: &str) -> Result<DateTime<Utc>> {
    let claims = JWT::new(secret)
        .validate(token)
        .map_err(Error::wrap)?
        .claims;
    let expires_at = session_times(&claims)?.expires_at;
    DateTime::from_timestamp(expires_at, 0).ok_or_else(|| {
        Error::string(&format!(
            "the signed token's exp {expires_at} is outside chrono's range"
        ))
    })
}

/// Answers a failed sign-in of the known account `account_id`: writes its
/// `login_failed` entry and returns the one `401 invalid_credentials`.
///
/// - `db`: the application's pool (`ctx.db`). The entry commits on a
///   connection of its own, outside any transaction of the request
///   (ADR 0011 §5), which [`users::record_failed_sign_in`] enforces by its
///   type.
/// - `account_id`: the account that the sign-in found.
/// - `client`: the request id and the client address of the attempt.
///
/// The answer does not depend on the entry: when the insert fails, the
/// failure is logged as an error, with the account's id, the request id
/// and the database's error but never the typed address, and the answer is
/// the same `401` that an unknown address gets (the module documentation
/// says why).
async fn refuse_known_account(
    db: &DatabaseConnection,
    account_id: i64,
    client: RequestContext,
) -> Response {
    let request_id = client.request_id.clone();
    if let Err(error) = users::record_failed_sign_in(db, account_id, client).await {
        tracing::error!(
            account_id,
            request_id = request_id.as_deref(),
            error = %error,
            "the login_failed entry of a failed sign-in could not be written"
        );
    }
    error_response(ErrorCode::InvalidCredentials)
}

/// `GET /api/admin/auth/me`: the account of the request's admin session, the
/// session's scope, and the account's roles, effective permissions, admin
/// flag and access map (ADR 0007 §7).
///
/// - `session`: the checked admin session; an `admin` or a `password_change`
///   one (ADR 0006 §4). Without one the route table's guard has already
///   answered `401 unauthorized`.
///
/// # Errors
///
/// An internal error (`500`) when the response cannot be built: an account
/// without an e-mail address or an `mfa` scope, both of which the session
/// check rules out ([`MeResponse::new`]). A failed lookup of the account or
/// of its roles and grants is the guard's `500`.
// `state` is named because the handler takes no `State`, from which the
// macro would otherwise infer it; the guard reads the `AppContext`.
#[debug_handler(state = AppContext)]
pub async fn me(session: AdminSession<AdminOrPasswordChange>) -> Result<Response> {
    // The model-to-DTO conversion happens here, at the controller boundary
    // (ADR 0005, ADR 0008 rule 6): only the allow-listed columns leave the
    // row, so the DTO never sees the password hash or the TOTP secret.
    // The roles and grants are the ones the guard read for this
    // request, so the answer shows a role change at once (ADR 0007 §5).
    let user = session.user;
    let me = MeResponse::new(
        user.pid,
        user.name,
        user.email,
        session.scope,
        &session.grants,
        RESOURCE_ACCESS,
    )
    .map_err(Error::wrap)?;
    format::json(me)
}

/// `POST /api/admin/auth/logout`: ends every session of the signed-in
/// account and deletes the session cookie (ADR 0006 §2).
///
/// - `ctx`: the application context; the handler takes the database from
///   it.
/// - `session`: the checked admin session; an `admin` or a
///   `password_change` one (ADR 0006 §4). Without one the route table's
///   guard has already answered `401 unauthorized`.
/// - `client`: the request id and the client address, for the activity
///   entry.
///
/// The answer is `204 No Content` with the cookie-clearing `Set-Cookie`,
/// also when the token came as a Bearer header: the browser that sent no
/// cookie ignores it. It is the answer's only `Set-Cookie`, also for a
/// token close to its expiry: the passive sliding refresh of ADR 0006 §1
/// must leave this route out, because a browser keeps the last cookie of a
/// name that an answer sets, so a refreshed token after the clearing value
/// would keep a cookie behind the logout. The request test
/// `logout_near_expiry_sets_only_the_clearing_cookie` pins it. The
/// revocation and the `logout` entry (ADR 0011 §5) are written on one
/// transaction.
///
/// # Errors
///
/// An internal error (`500`) when the database fails, including a refused
/// activity entry, never a `401`, which would sign the admin UI out while
/// the sessions still hold (ADR 0006, follow-up work). A failed account
/// lookup is the guard's `500`.
#[debug_handler]
pub async fn logout(
    State(ctx): State<AppContext>,
    session: AdminSession<AdminOrPasswordChange>,
    ClientContext(client): ClientContext,
) -> Result<Response> {
    // An account event: no revision, and `updated_at` stays as it is
    // (ADR 0011 §4). The active model holds no change, so the `UPDATE`
    // writes `token_version` alone. A failed write drops `txn`, which rolls
    // the revocation back: the sessions hold and the `500` keeps the cookie.
    let txn = ctx.db.begin().await?;
    session
        .user
        .into_active_model()
        .log_out(&txn, client)
        .await?;
    txn.commit().await?;
    format::render()
        .status(StatusCode::NO_CONTENT)
        .header(SET_COOKIE, cleared_session_cookie())
        .empty()
}

/// `POST /api/admin/auth/refresh`: renews the signed-in session's cookie
/// (ADR 0006 §1, "Sliding refresh, explicit").
///
/// - `ctx`: the application context; the handler takes the app clock, the
///   session limits and the `auth.jwt` configuration from it.
/// - `session`: the checked admin session; a full `admin` one only. Without
///   one the route table's guard has already answered `401 unauthorized`.
///
/// The answer is `204 No Content`. For the session cookie it carries a new
/// cookie and token that expire two hours after the request, or at the
/// session's absolute end, whichever comes first
/// ([`explicit_renewal`]); the `Max-Age` is the new token's lifetime. At
/// the absolute end, and for a Bearer token, it carries none (the module
/// documentation says why). The guard adds no passive refresh beside it:
/// it leaves an answer that sets the session cookie alone.
///
/// # Errors
///
/// An internal error (`500`) when the context has no app clock, no session
/// limits or no `auth.jwt` section, when the token lacks an integer
/// `auth_time` or `exp` (a defect of the code that signed it), or when the
/// token or its cookie cannot be built. Never a `401` while the session
/// still holds: on every admin route a `401` means no valid session
/// (research: DECISIONS R35 (D155)).
#[debug_handler]
pub async fn refresh(State(ctx): State<AppContext>, session: AdminSession) -> Result<Response> {
    let times = session_times(&session.claims)?;
    let now = clock(&ctx)?.now();
    let limit = session_limits(&ctx)?.absolute;
    let answer = format::render().status(StatusCode::NO_CONTENT);
    let Some(renewed) = explicit_renewal(session.carrier, now, times, limit) else {
        tracing::debug!(
            user_id = session.user.id,
            carrier = ?session.carrier,
            "explicit session refresh: nothing to renew (a Bearer token, or the absolute limit)"
        );
        return answer.empty();
    };
    let cookie = renewed_session_cookie(&session.user, times, renewed, &ctx)?;
    answer.header(SET_COOKIE, cookie).empty()
}
