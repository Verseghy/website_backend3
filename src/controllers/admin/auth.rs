//! The browser sign-in, `POST /api/admin/auth/login` (ADR 0006 §4), the
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
//! effective permissions and admin flag that the extractor read for the
//! request (ADR 0007 §7). It is one of the
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
//! Without a valid session it answers the extractor's `401 unauthorized`
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
//! - a sign-in that issues the full `admin` session sets `last_login_at` and
//!   writes `login`, on one transaction with nothing else in it
//!   ([`users::ActiveModel::record_sign_in`]); if either write fails, both
//!   roll back and the answer is a `500` without the cookie. An account that
//!   was deactivated, trashed or had its sessions ended while its password
//!   was being verified gets no session: the sign-in fails like any other;
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

use axum::{
    Json,
    extract::rejection::JsonRejection,
    http::{StatusCode, header::SET_COOKIE},
};
use loco_rs::prelude::*;

use super::{
    client_context::ClientContext,
    error_response, json_rejection_response,
    session::{AdminOrPasswordChange, AdminSession},
};
use crate::{
    app::clock,
    domain::session_cookie::{cleared_session_cookie, session_cookie},
    dtos::admin::{
        auth::{LoginRequest, LoginResponse, LoginStep, MeResponse},
        error::ErrorCode,
    },
    models::{
        activity_log::RequestContext,
        users::{self, SignInOutcome},
    },
};

/// `POST /api/admin/auth/login`: signs in with an e-mail address and a
/// password.
///
/// - `ctx`: the application context; the handler takes the database, the app
///   clock and the `auth.jwt` configuration from it.
/// - `client`: the request id and the client address, for the activity
///   entry.
/// - `body`: the JSON body, or why it could not be read.
///
/// A body that is not a JSON `{email, password}` answers `422 invalid_body`,
/// and one above the request body limit `413 body_too_large`, before any
/// lookup (the module documentation says why). A credential failure answers
/// `401 invalid_credentials`, after the `login_failed` entry of a known
/// account ([`refuse_known_account`]). A success sets `last_login_at` and
/// writes the `login` entry before it answers.
///
/// # Errors
///
/// An internal error (`500`) when the sign-in succeeds but its writes fail
/// (a refused `login` entry included), when the account lookup fails, when
/// the context has no app clock or no `auth.jwt` section, or when the token
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
    let user = match users::Model::authenticate(&ctx.db, &request.email, &request.password).await? {
        SignInOutcome::Accepted(user) => *user,
        SignInOutcome::Refused { account_id } => {
            return Ok(refuse_known_account(&ctx.db, account_id, client).await);
        }
        SignInOutcome::UnknownAccount => return Ok(error_response(ErrorCode::InvalidCredentials)),
    };
    let account_id = user.id;
    // `auth_time` is the app clock's "now" (ADR 0010 point 7). Loco's `exp`
    // still reads the system time (recipe 7 step 4 of the project skill); in
    // production the app clock is the system clock, so the two agree.
    let now = clock(&ctx)?.now();
    let jwt = ctx.config.get_jwt_config()?;
    let token = user.generate_session_token(&jwt.secret, jwt.expiration, now)?;
    let cookie = session_cookie(&token, jwt.expiration).map_err(Error::wrap)?;
    // The token and the cookie are built first, so that a sign-in is
    // recorded only when its answer can be sent. Every session that this
    // handler issues is a full `admin` one, the only kind that sets
    // `last_login_at` (ADR 0006 §4). A failed write drops `txn`, which rolls
    // both writes back, and the `500` carries no cookie.
    let txn = ctx.db.begin().await?;
    let recorded = user
        .into_active_model()
        .record_sign_in(&txn, client.clone())
        .await?;
    if recorded.is_none() {
        // The account was deactivated, trashed or had its sessions ended
        // between the password check and this write, so the token above is
        // already void: the sign-in failed, and is answered and recorded as
        // a failure. Nothing was written on `txn`.
        txn.rollback().await?;
        return Ok(refuse_known_account(&ctx.db, account_id, client).await);
    }
    txn.commit().await?;
    format::render()
        .header(SET_COOKIE, cookie)
        .json(LoginResponse {
            step: LoginStep::Done,
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
/// session's scope, and the account's roles, effective permissions and admin
/// flag (ADR 0007 §7).
///
/// - `session`: the checked admin session; an `admin` or a `password_change`
///   one (ADR 0006 §4). Without one the extractor has already answered
///   `401 unauthorized`.
///
/// # Errors
///
/// An internal error (`500`) when the response cannot be built: an account
/// without an e-mail address or an `mfa` scope, both of which the session
/// check rules out ([`MeResponse::new`]). A failed lookup of the account or
/// of its roles and grants is the extractor's `500`.
// `state` is named because the handler takes no `State`, from which the
// macro would otherwise infer it; the extractor reads the `AppContext`.
#[debug_handler(state = AppContext)]
pub async fn me(session: AdminSession<AdminOrPasswordChange>) -> Result<Response> {
    // The model-to-DTO conversion happens here, at the controller boundary
    // (ADR 0005, ADR 0008 rule 6): only the allow-listed columns leave the
    // row, so the DTO never sees the password hash or the TOTP secret.
    // The roles and grants are the ones the extractor read for this
    // request, so the answer shows a role change at once (ADR 0007 §5).
    let user = session.user;
    let me = MeResponse::new(
        user.pid,
        user.name,
        user.email,
        session.scope,
        &session.grants,
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
///   `password_change` one (ADR 0006 §4). Without one the extractor has
///   already answered `401 unauthorized`.
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
/// lookup is the extractor's `500`.
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
