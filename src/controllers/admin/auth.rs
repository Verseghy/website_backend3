//! The browser sign-in, `POST /api/admin/auth/login` (ADR 0006 §4), and the
//! signed-in account, `GET /api/admin/auth/me` (ADR 0007 §7).
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
//! ([`super::session::AdminSession`]) as a [`MeResponse`]. It is one of the
//! three routes that a `password_change` session may reach (ADR 0006 §4), so
//! the admin UI can tell such a session to set a new password. Without a
//! valid session it answers `401 unauthorized`.

use axum::{Json, extract::rejection::JsonRejection, http::header::SET_COOKIE};
use loco_rs::prelude::*;

use super::{
    error_response, json_rejection_response,
    session::{AdminOrPasswordChange, AdminSession},
};
use crate::{
    app::clock,
    domain::session_cookie::session_cookie,
    dtos::admin::{
        auth::{LoginRequest, LoginResponse, LoginStep, MeResponse},
        error::ErrorCode,
    },
    models::users,
};

/// `POST /api/admin/auth/login`: signs in with an e-mail address and a
/// password.
///
/// - `ctx`: the application context; the handler takes the database, the app
///   clock and the `auth.jwt` configuration from it.
/// - `body`: the JSON body, or why it could not be read.
///
/// A body that is not a JSON `{email, password}` answers `422 invalid_body`,
/// and one above the request body limit `413 body_too_large`, before any
/// lookup (the module documentation says why). A credential failure answers
/// `401 invalid_credentials`.
///
/// # Errors
///
/// An internal error (`500`) when the database fails, when the context has
/// no app clock or no `auth.jwt` section, or when the token or its cookie
/// cannot be built (a secret that is not base64).
#[debug_handler]
pub async fn login(
    State(ctx): State<AppContext>,
    body: Result<Json<LoginRequest>, JsonRejection>,
) -> Result<Response> {
    let request = match body {
        Ok(Json(request)) => request,
        Err(rejection) => return Ok(json_rejection_response(&rejection)),
    };
    let Some(user) = users::Model::authenticate(&ctx.db, &request.email, &request.password).await?
    else {
        return Ok(error_response(ErrorCode::InvalidCredentials));
    };
    // `auth_time` is the app clock's "now" (ADR 0010 point 7). Loco's `exp`
    // still reads the system time (recipe 7 step 4 of the project skill); in
    // production the app clock is the system clock, so the two agree.
    let now = clock(&ctx)?.now();
    let jwt = ctx.config.get_jwt_config()?;
    let token = user.generate_session_token(&jwt.secret, jwt.expiration, now)?;
    let cookie = session_cookie(&token, jwt.expiration).map_err(Error::wrap)?;
    format::render()
        .header(SET_COOKIE, cookie)
        .json(LoginResponse {
            step: LoginStep::Done,
        })
}

/// `GET /api/admin/auth/me`: the account of the request's admin session and
/// the session's scope.
///
/// - `session`: the checked admin session; an `admin` or a `password_change`
///   one (ADR 0006 §4). Without one the extractor has already answered
///   `401 unauthorized`.
///
/// # Errors
///
/// An internal error (`500`) when the response cannot be built: an account
/// without an e-mail address or an `mfa` scope, both of which the session
/// check rules out ([`MeResponse::new`]). A failed account lookup is the
/// extractor's `500`.
// `state` is named because the handler takes no `State`, from which the
// macro would otherwise infer it; the extractor reads the `AppContext`.
#[debug_handler(state = AppContext)]
pub async fn me(session: AdminSession<AdminOrPasswordChange>) -> Result<Response> {
    // The model-to-DTO conversion happens here, at the controller boundary
    // (ADR 0005, ADR 0008 rule 6): only the allow-listed columns leave the
    // row, so the DTO never sees the password hash or the TOTP secret.
    let user = session.user;
    let me =
        MeResponse::new(user.pid, user.name, user.email, session.scope).map_err(Error::wrap)?;
    format::json(me)
}
