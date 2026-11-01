//! The admin session extractor (ADR 0006 §1, ADR 0007 §5).
//!
//! [`AdminSession`] is how an admin handler learns who is signed in. It wraps
//! Loco's `JWTWithUser<users::Model>`, which reads the token from the
//! `__Host-admin_session` cookie or the Bearer header (`auth.jwt.location`),
//! checks its signature and expiry, and loads the account that its `pid`
//! names. Then it applies the session rules of
//! [`crate::domain::session::check_session`]: the account is neither
//! anonymised nor in the trash, it is active, the token's `tv` is the stored
//! `token_version`, and its `scope` is one that the route accepts. Every
//! refusal answers the same `401 unauthorized` in the admin envelope, so a
//! client learns nothing about an account beyond "sign in again".
//!
//! The account is read from the database on every request, never from a
//! cache (ADR 0016): revocation through `token_version`, deactivation and the
//! trash work because each request sees the current row (ADR 0006 §2).
//!
//! A route says which scopes it accepts with the type parameter, an
//! [`AcceptedScopes`] marker: [`AdminSession`] (the default, [`AdminScope`])
//! takes only a full session, and `AdminSession<AdminOrPasswordChange>` also
//! takes a `password_change` session, which ADR 0006 §4 allows on
//! `auth/me`, `auth/change-password` and `auth/logout` only. No marker
//! accepts `mfa`: the two-factor step reads its own cookie without this
//! extractor (ADR 0006 §4). The compiler holds that rule. [`AcceptedScopes`]
//! is sealed, so only this module defines markers, and the extractor fails
//! to compile for a marker whose scopes hold `mfa`.
//!
//! An admin handler that needs a signed-in account takes this extractor and
//! never Loco's `JWT` or `JWTWithUser` directly: those check neither the
//! account's state nor the version nor the scope.
//!
//! Loco answers its own refusals (no token, a token that fails validation,
//! an unknown `pid`) with `loco_rs::Error::Unauthorized`; this extractor
//! answers them with the admin envelope instead of Loco's body, and without
//! Loco's ERROR-level `controller_error` log line, because an expired session
//! is routine. Loco still logs a token that fails validation itself, at
//! ERROR level (loco-rs 1.2.0 `validate_jwt`). Any other error stays Loco's
//! `500`: a database failure, and an `auth` or `auth.jwt` section that is
//! missing. An outage must not answer 401, which signs the admin UI out
//! (ADR 0006, follow-up work). A configured `auth.jwt.secret` that is not
//! base64 is not one of them: loco-rs 1.2.0 decodes the key inside the
//! token validation and reports the failure as a token that is not valid,
//! so every session request answers `401`. ADR 0034 §3 places a boot check
//! of the JWT secret (base64 of at least 64 bytes) beside the refusal of the
//! published keys, so that such a configuration stops at boot instead.

use std::marker::PhantomData;

use axum::{
    extract::{FromRef, FromRequestParts},
    http::request::Parts,
};
use loco_rs::{auth::jwt::UserClaims, prelude::*};

use super::error_response;
use crate::{
    domain::session::{SessionScope, check_session},
    dtos::admin::error::ErrorCode,
    models::users,
};

/// The seal of [`AcceptedScopes`].
///
/// `Sealed` is public in a private module: code outside this module can
/// name `AcceptedScopes` as a bound, but cannot implement `Sealed`, so it
/// cannot add a marker either. The set of markers, and so the scopes any
/// admin route can accept, is the list in this file.
mod sealed {
    /// Implemented by the markers of this module, and by nothing else.
    pub trait Sealed {}
}

/// The session scopes that a route accepts (ADR 0006 §4); implemented by
/// the uninhabited marker types below, which [`AdminSession`] takes as its
/// type parameter. The trait is sealed: no other module can implement it.
pub trait AcceptedScopes: sealed::Sealed {
    /// The accepted scopes. Never [`SessionScope::Mfa`]: only the two-factor
    /// step accepts an `mfa` token, and it does not use this extractor.
    /// [`AdminSession`] does not compile for a marker that breaks the rule.
    const SCOPES: &'static [SessionScope];
}

/// A full `admin` session only: the scope of every admin route except the
/// three own-account routes of [`AdminOrPasswordChange`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AdminScope {}

impl sealed::Sealed for AdminScope {}

impl AcceptedScopes for AdminScope {
    const SCOPES: &'static [SessionScope] = &[SessionScope::Admin];
}

/// An `admin` or a `password_change` session: the scopes of
/// `GET /api/admin/auth/me`, `POST /api/admin/auth/change-password` and
/// `POST /api/admin/auth/logout`, and of no other route (ADR 0006 §4).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AdminOrPasswordChange {}

impl sealed::Sealed for AdminOrPasswordChange {}

impl AcceptedScopes for AdminOrPasswordChange {
    const SCOPES: &'static [SessionScope] = &[SessionScope::Admin, SessionScope::PasswordChange];
}

/// A request's admin session: the signed-in account, checked for this
/// request (the module documentation lists the checks).
///
/// - `A`: the scopes the route accepts; [`AdminScope`] unless the route is
///   one that ADR 0006 §4 opens to a `password_change` session.
// No `Debug`: `users::Model` derives it and would print the password hash
// and the TOTP secret (the `users` module documentation says why it has to).
pub struct AdminSession<A: AcceptedScopes = AdminScope> {
    /// The signed-in account, as read from the database for this request.
    pub user: users::Model,
    /// The session's scope, one of `A::SCOPES`.
    pub scope: SessionScope,
    /// The token's claims as Loco validated them: `pid`, and in its
    /// `claims` map the custom claims, such as the `auth_time` that the
    /// session refresh needs. loco-rs 1.2.0 keeps `exp` private and has no
    /// getter for it, so code that needs the expiry reads it by serialising
    /// this value (`UserClaims` derives `Serialize`) or decodes the token
    /// itself.
    pub claims: UserClaims,
    /// The accepted scopes, in the type only. `fn() -> A` keeps the struct
    /// `Send` and `Sync` whatever `A` is.
    accepts: PhantomData<fn() -> A>,
}

impl<S, A> FromRequestParts<S> for AdminSession<A>
where
    AppContext: FromRef<S>,
    S: Send + Sync,
    A: AcceptedScopes,
{
    type Rejection = Response;

    /// Loads the account through Loco's `JWTWithUser` and checks the
    /// session.
    ///
    /// - `parts`: the request's head, where Loco finds the token.
    /// - `state`: the router's state, which holds the [`AppContext`].
    ///
    /// # Errors
    ///
    /// `401 unauthorized` in the admin envelope for every refusal, and for
    /// an `auth.jwt.secret` that is not base64 (the module documentation
    /// says why); Loco's `500` when the database fails or the `auth.jwt`
    /// section is missing.
    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        // Evaluated when the compiler instantiates this extractor for `A`:
        // a marker whose scopes hold `mfa` is a compile error, not a route
        // that opens to the two-factor token (ADR 0006 §4).
        const {
            assert!(
                !SessionScope::Mfa.is_in(A::SCOPES),
                "an AcceptedScopes marker must never accept an mfa token (ADR 0006 §4)"
            );
        };
        let auth::JWTWithUser { claims, user } =
            auth::JWTWithUser::<users::Model>::from_request_parts(parts, state)
                .await
                .map_err(loco_refusal_response)?;
        match check_session(user.session_state(), &claims.claims, A::SCOPES) {
            Ok(scope) => Ok(Self {
                user,
                scope,
                claims,
                accepts: PhantomData,
            }),
            Err(refusal) => {
                // DEBUG: an expired or revoked session is routine, and every
                // open admin tab of a signed-out account sends one.
                tracing::debug!(user_id = user.id, %refusal, "admin session refused");
                Err(error_response(ErrorCode::Unauthorized))
            }
        }
    }
}

/// The answer to a request that Loco's `JWTWithUser` refused.
///
/// - `error`: Loco's refusal.
///
/// `Unauthorized` (no token, a token that fails validation, an account that
/// the `pid` does not name) becomes the admin `401 unauthorized` envelope.
/// Everything else, a database failure above all, keeps Loco's own answer
/// and its log line, a `500`.
fn loco_refusal_response(error: Error) -> Response {
    match error {
        Error::Unauthorized(reason) => {
            tracing::debug!(reason, "admin session refused by Loco's extractor");
            error_response(ErrorCode::Unauthorized)
        }
        other => other.into_response(),
    }
}

#[cfg(test)]
mod tests {
    use axum::{body::to_bytes, http::StatusCode};

    use super::*;
    use crate::dtos::admin::error::ErrorEnvelope;

    #[test]
    fn no_marker_accepts_an_mfa_token() {
        assert_eq!(AdminScope::SCOPES, &[SessionScope::Admin]);
        assert_eq!(
            AdminOrPasswordChange::SCOPES,
            &[SessionScope::Admin, SessionScope::PasswordChange]
        );
        // The compile-time assertion of `from_request_parts` checks the same
        // with `is_in`; this pins each marker's whole list.
        for scopes in [AdminScope::SCOPES, AdminOrPasswordChange::SCOPES] {
            assert!(!scopes.contains(&SessionScope::Mfa), "{scopes:?}");
            assert!(!SessionScope::Mfa.is_in(scopes), "{scopes:?}");
        }
    }

    #[tokio::test]
    async fn loco_s_unauthorized_becomes_the_admin_envelope() {
        for reason in ["token is not found", "token is not valid", "not found"] {
            let response = loco_refusal_response(Error::Unauthorized(reason.to_owned()));
            assert_eq!(response.status(), StatusCode::UNAUTHORIZED, "{reason}");
            let body = to_bytes(response.into_body(), 4096).await.expect("body");
            assert_eq!(
                body,
                serde_json::to_vec(&ErrorEnvelope::new(ErrorCode::Unauthorized))
                    .expect("serializes"),
                "{reason}"
            );
        }
    }

    #[tokio::test]
    async fn every_other_loco_error_keeps_loco_s_answer() {
        // A database failure (`map_auth_model_error`) and a missing `auth.jwt`
        // section (`get_jwt_from_config`) are server faults, never a 401.
        for error in [
            Error::InternalServerError,
            Error::string("JWT token not configured"),
        ] {
            let response = loco_refusal_response(error);
            assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
            let body = to_bytes(response.into_body(), 4096).await.expect("body");
            let body: serde_json::Value = serde_json::from_slice(&body).expect("Loco's JSON");
            assert_ne!(body["error"], "unauthorized", "{body}");
        }
    }
}
