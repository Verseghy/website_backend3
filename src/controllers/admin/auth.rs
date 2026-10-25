//! The browser sign-in, `POST /api/admin/auth/login` (ADR 0006 §4).
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

use axum::{Json, extract::rejection::JsonRejection, http::header::SET_COOKIE};
use loco_rs::prelude::*;

use super::{error_response, json_rejection_response};
use crate::{
    app::clock,
    domain::session_cookie::session_cookie,
    dtos::admin::{
        auth::{LoginRequest, LoginResponse, LoginStep},
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
