//! The admin session (ADR 0006 §1, ADR 0007 §5): how a request's session is
//! opened and checked, and how a handler learns who is signed in.
//!
//! Two halves, so that every route is checked by the route table and never
//! by the handler alone (ADR 0007 §5, "Deny by default"):
//!
//! - `open_session` opens and checks the session. The guard of the admin
//!   route table (`super::routes`) calls it for every route whose
//!   requirement needs a session, before the handler runs, and hands the
//!   result on through the request's extensions.
//! - [`AdminSession`] is the extractor a handler takes. It only receives
//!   what the guard opened; it never opens a session itself. A handler that
//!   takes it on a route the guard did not open a session for (a route
//!   mounted outside the route table, or a `Public` one) answers `500` and
//!   logs why, so such a mistake fails loudly in every test of that route
//!   instead of serving an unchecked request.
//!
//! `open_session` wraps Loco's `JWTWithUser<users::Model>`, which reads the
//! token from the `__Host-admin_session` cookie or the Bearer header
//! (`auth.jwt.location`), checks its signature and expiry, and loads the
//! account that its `pid` names. Then it applies the session rules of
//! [`crate::domain::session::check_session`]: the account is neither
//! anonymised nor in the trash, it is active, the token's `tv` is the stored
//! `token_version`, and its `scope` is one that the route accepts. Every
//! refusal answers the same `401 unauthorized` in the admin envelope, so a
//! client learns nothing about an account beyond "sign in again".
//!
//! The account is read from the database on every request, never from a
//! cache (ADR 0016): revocation through `token_version`, deactivation and the
//! trash work because each request sees the current row (ADR 0006 §2). Once
//! the session holds, `open_session` also reads the roles and extra grants
//! that the account holds ([`users::Model::grants`]), so a role or grant
//! change applies on the account's next request without ending its sessions
//! (ADR 0007 §5, "Fresh data"; ADR 0006 §2). A refused session costs no such
//! read. The route table's permission check and `/auth/me` work from
//! [`AdminSession::grants`].
//!
//! A route says which scopes it accepts with an [`AcceptedScopes`] marker,
//! twice: in the route table (`AdminRoute::accepting`), which decides what
//! the guard opens, and in the handler's type parameter, which the extractor
//! checks again. [`AdminSession`] (the default, [`AdminScope`]) takes only a
//! full session, and `AdminSession<AdminOrPasswordChange>` also takes a
//! `password_change` session, which ADR 0006 §4 allows on `auth/me`,
//! `auth/change-password` and `auth/logout` only. When the two disagree the
//! stricter wins: a scope that either refuses answers `401`. No marker
//! accepts `mfa`: the two-factor step reads its own cookie without this
//! extractor (ADR 0006 §4). The compiler holds that rule. [`AcceptedScopes`]
//! is sealed, so only this module defines markers, and [`accepted_scopes`],
//! through which both places read a marker, fails to compile for a marker
//! whose scopes hold `mfa`.
//!
//! The sliding refresh (ADR 0006 §1) is built here too. [`open_session`]
//! notes where the token came from ([`TokenCarrier`]: the session cookie or
//! the Bearer header, which is never refreshed, ADR 0006 §3). The guard asks
//! `passive_refresh_cookie` for the passive refresh of every session it
//! opens and adds the cookie to the answer with `with_passive_refresh`,
//! unless the handler set the session cookie itself; the explicit refresh,
//! `POST /api/admin/auth/refresh` (`super::auth::refresh`), builds its
//! cookie with [`renewed_session_cookie`] as well. Both read the token's
//! `auth_time` and `exp` with [`session_times`], "now" from the app clock
//! and the absolute limit from [`session_limits`], and leave the arithmetic
//! to [`crate::domain::session`].
//!
//! An admin handler that needs a signed-in account takes this extractor and
//! never Loco's `JWT` or `JWTWithUser` directly: those check neither the
//! account's state nor the version nor the scope, and no permission.
//!
//! Loco answers its own refusals (no token, a token that fails validation,
//! an unknown `pid`) with `loco_rs::Error::Unauthorized`; the guard answers
//! them, and the session rules' refusals, through `session_refusal_response`
//! with the admin envelope instead of Loco's body, and without Loco's
//! ERROR-level `controller_error` log line, because an expired session is
//! routine. Loco still logs a token that fails validation itself, at
//! ERROR level (loco-rs 1.2.0 `validate_jwt`). Any other error stays Loco's
//! `500`: a database failure (also while reading the roles and grants), a
//! stored role or grant that the code does not know (which the schema rules
//! out), and an `auth` or `auth.jwt` section that is
//! missing. An outage must not answer 401, which signs the admin UI out
//! (ADR 0006, follow-up work). A configured `auth.jwt.secret` that is not
//! base64 is not one of them: loco-rs 1.2.0 decodes the key inside the
//! token validation and reports the failure as a token that is not valid,
//! so every session request answers `401`. ADR 0034 §3 places a boot check
//! of the JWT secret (base64 of at least 64 bytes) beside the refusal of the
//! published keys, so that such a configuration stops at boot instead.

use std::{
    future::{Future, ready},
    marker::PhantomData,
    sync::Arc,
};

use axum::{
    extract::FromRequestParts,
    http::{HeaderValue, header::SET_COOKIE, request::Parts},
};
use chrono::DateTime;
use loco_rs::{
    auth::jwt::UserClaims, controller::extractor::auth::extract_token_from_cookie, prelude::*,
};

use super::error_response;
use crate::{
    app::clock,
    domain::{
        authz::AccountGrants,
        session::{
            AbsoluteLimit, Renewal, SessionScope, SessionTimes, TokenCarrier, check_session,
            passive_renewal,
        },
        session_cookie::{SESSION_COOKIE_NAME, names_session_cookie, session_cookie},
    },
    dtos::admin::error::ErrorCode,
    models::users,
    settings::Settings,
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
/// type parameter and the route table reads through [`accepted_scopes`].
/// The trait is sealed: no other module can implement it.
pub trait AcceptedScopes: sealed::Sealed {
    /// The accepted scopes. Never [`SessionScope::Mfa`]: only the two-factor
    /// step accepts an `mfa` token, and it does not use this extractor.
    /// [`accepted_scopes`] does not compile for a marker that breaks the
    /// rule.
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

/// The scopes of the marker `A`, checked when the compiler instantiates this
/// function for `A`: a marker whose scopes hold `mfa` is a compile error,
/// not a route that opens to the two-factor token (ADR 0006 §4).
///
/// The route table and the extractor read a marker's scopes only through
/// this function, so both are held to the rule.
#[must_use]
pub const fn accepted_scopes<A: AcceptedScopes>() -> &'static [SessionScope] {
    const {
        assert!(
            !SessionScope::Mfa.is_in(A::SCOPES),
            "an AcceptedScopes marker must never accept an mfa token (ADR 0006 §4)"
        );
    };
    A::SCOPES
}

/// A session that [`open_session`] opened and checked: what the route
/// table's guard hands to the handler's [`AdminSession`].
///
/// It travels in the request's extensions inside an [`Arc`], because
/// `http::Extensions` holds only `Clone` values and Loco's `UserClaims` is
/// not `Clone`. The type is visible to this module's parent only, so no
/// other code can put one into a request.
// No `Debug`: `users::Model` derives it and would print the password hash
// and the TOTP secret (the `users` module documentation says why it has to).
pub(super) struct OpenedSession {
    /// The signed-in account, as read from the database for this request.
    pub(super) user: users::Model,
    /// The session's scope, one of the scopes the route table accepts.
    pub(super) scope: SessionScope,
    /// The roles and extra grants that the account holds.
    pub(super) grants: AccountGrants,
    /// The token's claims as Loco validated them.
    pub(super) claims: UserClaims,
    /// Where the request carried the token.
    pub(super) carrier: TokenCarrier,
}

impl OpenedSession {
    /// Puts the session into the request's extensions, for the handler's
    /// [`AdminSession`] to take.
    ///
    /// - `parts`: the request's head, which the handler receives next.
    pub(super) fn hand_over(self, parts: &mut Parts) {
        parts.extensions.insert(Arc::new(self));
    }
}

/// Opens and checks the session of a request to a route that accepts
/// `scopes`, then reads the account's roles and extra grants.
///
/// - `parts`: the request's head, where Loco finds the token.
/// - `ctx`: the application context; Loco reads the `auth.jwt`
///   configuration and the database from it.
/// - `scopes`: the scopes the route accepts, as [`accepted_scopes`] gives
///   them for a marker.
///
/// # Errors
///
/// `Error::Unauthorized` for every refusal, and for an `auth.jwt.secret`
/// that is not base64 (the module documentation says why); any other Loco
/// error when the database fails, when a stored role or grant is one the
/// code does not know, or when the `auth.jwt` section is missing.
/// [`session_refusal_response`] turns either into the answer.
pub(super) async fn open_session(
    parts: &mut Parts,
    ctx: &AppContext,
    scopes: &'static [SessionScope],
) -> Result<OpenedSession, Error> {
    let carrier = token_carrier(parts);
    let auth::JWTWithUser { claims, user } =
        auth::JWTWithUser::<users::Model>::from_request_parts(parts, ctx)
            .await
            .inspect_err(|error| {
                if let Error::Unauthorized(reason) = error {
                    // DEBUG: an expired session is routine.
                    tracing::debug!(reason, "admin session refused by Loco's extractor");
                }
            })?;
    let scope = match check_session(user.session_state(), &claims.claims, scopes) {
        Ok(scope) => scope,
        Err(refusal) => {
            // DEBUG: an expired or revoked session is routine, and every
            // open admin tab of a signed-out account sends one.
            tracing::debug!(user_id = user.id, %refusal, "admin session refused");
            return Err(Error::Unauthorized(refusal.to_string()));
        }
    };
    // After the check, so a refused session costs no query. A failure here
    // is a server fault, never a 401, which would sign the admin UI out:
    // Loco's own answer logs it at ERROR and sends its 500. The model never
    // answers `EntityNotFound` (Loco's 404) for an account without roles.
    let grants = user.grants(&ctx.db).await.map_err(Error::from)?;
    Ok(OpenedSession {
        user,
        scope,
        grants,
        claims,
        carrier,
    })
}

/// Where a request carries its session token, as Loco reads it.
///
/// - `parts`: the request's head.
///
/// `auth.jwt.location` lists the session cookie first, then the Bearer
/// header (checked at boot, `crate::domain::session_jwt::check_locations`),
/// and Loco takes the first location that holds a token, with the same
/// cookie parser as here (loco-rs 1.2.0 `extract_token`). So a request that
/// sends the session cookie, even beside a Bearer header, is authenticated
/// by the cookie, and one without it by the Bearer header.
fn token_carrier(parts: &Parts) -> TokenCarrier {
    if extract_token_from_cookie(SESSION_COOKIE_NAME, parts).is_ok() {
        TokenCarrier::Cookie
    } else {
        TokenCarrier::Bearer
    }
}

/// What the session refresh takes from the settings: the absolute limit of
/// ADR 0006 §8.
///
/// `App::after_context` builds it once from the validated settings and puts
/// it into the shared store; the guard and the refresh handler read it with
/// [`session_limits`] and pass the limit to the pure arithmetic of
/// `crate::domain::session` (ADR 0008 rules 2 and 3).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SessionLimits {
    /// `settings.admin_auth.session_absolute_max_secs`.
    pub absolute: AbsoluteLimit,
}

impl SessionLimits {
    /// The limits of `settings`.
    ///
    /// - `settings`: the validated settings of the process.
    #[must_use]
    pub fn new(settings: &Settings) -> Self {
        Self {
            absolute: settings.admin_auth.session_absolute_max_secs,
        }
    }
}

/// The session limits of this app context.
///
/// - `ctx`: the request's app context.
///
/// # Errors
///
/// [`Error::Message`] when the context carries none: every context Loco
/// builds has them (`App::after_context`), one built by hand does not.
pub fn session_limits(ctx: &AppContext) -> Result<SessionLimits> {
    ctx.shared_store.get::<SessionLimits>().ok_or_else(|| {
        Error::string(
            "the session limits are missing from the shared store; App::after_context registers \
             them, so this context was not built by App::boot",
        )
    })
}

/// The `auth_time` and `exp` of a validated session token.
///
/// - `claims`: the claims Loco validated.
///
/// loco-rs 1.2.0 keeps `UserClaims::exp` private and has no getter for it,
/// but `UserClaims` derives `Serialize`: serialised, it writes `pid`, `exp`
/// and the custom claims side by side, which
/// [`SessionTimes::from_claims`] reads.
///
/// # Errors
///
/// An internal error when the claims hold no integer `auth_time` or `exp`.
/// Every token backend3 signs carries both, and only backend3 holds the
/// secret, so either is a defect in the code that signed it.
pub fn session_times(claims: &UserClaims) -> Result<SessionTimes> {
    match serde_json::to_value(claims)? {
        serde_json::Value::Object(all) => SessionTimes::from_claims(&all).map_err(Error::wrap),
        other => Err(Error::string(&format!(
            "Loco's UserClaims serialised to {other}, not to an object"
        ))),
    }
}

/// The `Set-Cookie` value that renews a full `admin` session (ADR 0006 §1):
/// a new token for `user`, with the session's `auth_time`, that expires as
/// `renewed` says.
///
/// - `user`: the account, as read for this request; the new token carries
///   its current `token_version`, which the session check has just found
///   equal to the token's.
/// - `times`: the renewed token's times; its `auth_time` is kept, so the
///   absolute limit still counts from the sign-in.
/// - `renewed`: the renewal from `crate::domain::session`.
/// - `ctx`: the app context, for the `auth.jwt` configuration.
///
/// Loco's `JWT::generate_token` sets `exp` to the system time plus the
/// lifetime it is given (recipe 7 step 4 of the project skill), so the
/// lifetime of `renewed` goes to Loco and to the cookie's `Max-Age` alike.
/// In production the app clock is the system clock, read a few microseconds
/// before Loco reads it, so `exp` is `renewed.expires_at`, or one second
/// later when the system clock's second turns between the two reads. Either
/// stays within `auth_time` plus the absolute limit: a capped renewal counts
/// its lifetime from "now" rounded up (`crate::domain::session::renewal`).
///
/// # Errors
///
/// An internal error when the context has no `auth.jwt` section, when the
/// `auth_time` is outside chrono's range, or when the token or its cookie
/// cannot be built (a secret that is not base64, which the boot check rules
/// out).
pub fn renewed_session_cookie(
    user: &users::Model,
    times: SessionTimes,
    renewed: Renewal,
    ctx: &AppContext,
) -> Result<String> {
    let auth_time = DateTime::from_timestamp(times.auth_time, 0).ok_or_else(|| {
        Error::string(&format!(
            "the session's auth_time {} is outside chrono's range",
            times.auth_time
        ))
    })?;
    let jwt = ctx.config.get_jwt_config()?;
    let token = user.generate_session_token(&jwt.secret, renewed.lifetime_secs, auth_time)?;
    session_cookie(&token, renewed.lifetime_secs).map_err(Error::wrap)
}

/// The cookie of the passive refresh for an opened session, or `None` when
/// the session gets none (ADR 0006 §1, "Sliding refresh, passive").
///
/// - `session`: the session the guard has just opened.
/// - `ctx`: the app context, for the app clock, the session limits and the
///   `auth.jwt` configuration.
///
/// [`passive_renewal`] decides: a full `admin` session that the session
/// cookie carries, with less than 3,600 s left, and a renewal that ends
/// later than its token. The guard reads "now" here, once per request.
///
/// # Errors
///
/// The errors of [`renewed_session_cookie`], and an internal error when the
/// context has no app clock or no session limits. A token without an
/// integer `auth_time` or `exp` is not one: it gets no refresh, and its
/// request goes on (an ERROR line names the defect), because an optional
/// renewal must not fail a request that its session admits.
pub(super) fn passive_refresh_cookie(
    session: &OpenedSession,
    ctx: &AppContext,
) -> Result<Option<HeaderValue>> {
    // The two checks that need no claim come first, so a Bearer request or
    // a restricted session reads neither the clock nor the claims.
    if session.scope != SessionScope::Admin || session.carrier != TokenCarrier::Cookie {
        return Ok(None);
    }
    let times = match session_times(&session.claims) {
        Ok(times) => times,
        Err(error) => {
            tracing::error!(
                user_id = session.user.id,
                %error,
                "a session token without auth_time or exp: no passive refresh"
            );
            return Ok(None);
        }
    };
    let now = clock(ctx)?.now();
    let limit = session_limits(ctx)?.absolute;
    let Some(renewed) = passive_renewal(session.scope, session.carrier, now, times, limit) else {
        return Ok(None);
    };
    tracing::debug!(
        user_id = session.user.id,
        expires_at = renewed.expires_at,
        "passive session refresh"
    );
    let cookie = renewed_session_cookie(&session.user, times, renewed, ctx)?;
    HeaderValue::from_str(&cookie)
        .map(Some)
        .map_err(Error::wrap)
}

/// `response` with the passive refresh's `cookie` added, unless the
/// response sets, replaces or deletes the session cookie itself.
///
/// - `response`: the answer to a request whose session the guard opened:
///   the handler's, or the guard's own `403`.
/// - `cookie`: the `Set-Cookie` value of [`passive_refresh_cookie`], or
///   `None`.
///
/// A browser keeps the last of two cookies with one name, so a renewal
/// beside the handler's own session cookie would override it: the logout's
/// deletion would leave a session behind (ADR 0006 §2), and the explicit
/// refresh's cookie would be replaced by an equal one. The handler's
/// cookie therefore wins, and is the answer's only session cookie.
pub(super) fn with_passive_refresh(
    mut response: Response,
    cookie: Option<HeaderValue>,
) -> Response {
    let Some(cookie) = cookie else {
        return response;
    };
    let sets_its_own = response
        .headers()
        .get_all(SET_COOKIE)
        .iter()
        .any(|value| names_session_cookie(value.as_bytes()));
    if !sets_its_own {
        response.headers_mut().append(SET_COOKIE, cookie);
    }
    response
}

/// A request's admin session: the signed-in account, opened and checked
/// for this request by the route table's guard (the module documentation
/// lists the checks).
///
/// - `A`: the scopes the handler accepts; [`AdminScope`] unless the route is
///   one that ADR 0006 §4 opens to a `password_change` session.
// No `Debug`: `users::Model` derives it and would print the password hash
// and the TOTP secret (the `users` module documentation says why it has to).
pub struct AdminSession<A: AcceptedScopes = AdminScope> {
    /// The signed-in account, as read from the database for this request.
    pub user: users::Model,
    /// The session's scope, one of `A::SCOPES`.
    pub scope: SessionScope,
    /// The roles and extra grants that the account holds, read for this
    /// request after the session check (ADR 0007 §3, §5).
    pub grants: AccountGrants,
    /// The token's claims as Loco validated them: `pid`, and in its
    /// `claims` map the custom claims, such as the `auth_time` that the
    /// session refresh needs. loco-rs 1.2.0 keeps `exp` private and has no
    /// getter for it; [`session_times`] reads it with `auth_time`.
    pub claims: UserClaims,
    /// Where the request carried the token: the session cookie of a
    /// browser, or the Bearer header of a script, which is never refreshed
    /// (ADR 0006 §3).
    pub carrier: TokenCarrier,
    /// The accepted scopes, in the type only. `fn() -> A` keeps the struct
    /// `Send` and `Sync` whatever `A` is.
    accepts: PhantomData<fn() -> A>,
}

impl<S, A> FromRequestParts<S> for AdminSession<A>
where
    S: Send + Sync,
    A: AcceptedScopes,
{
    type Rejection = Response;

    /// Takes the session that the route table's guard opened for this
    /// request, and checks that its scope is one the handler accepts.
    ///
    /// - `parts`: the request's head, whose extensions carry the session.
    /// - `_state`: the router's state; not read, because the guard has
    ///   already used it.
    ///
    /// # Errors
    ///
    /// `401 unauthorized` in the admin envelope when the session's scope is
    /// not in `A::SCOPES` (the route table accepted a scope that the handler
    /// does not). Loco's `500` when the guard opened no session: the route
    /// is mounted outside the route table, or the table declares it
    /// `Public`, or a second extractor already took the session. Each is a
    /// mistake in the code, never something a request can cause.
    // Not `async fn`: nothing here waits, the guard has done the I/O.
    fn from_request_parts(
        parts: &mut Parts,
        _state: &S,
    ) -> impl Future<Output = Result<Self, Self::Rejection>> + Send {
        let scopes = accepted_scopes::<A>();
        // `into_inner` gives the session back when this is the only handle,
        // which it is: the guard dropped its own when it handed it over.
        let Some(opened) = parts
            .extensions
            .remove::<Arc<OpenedSession>>()
            .and_then(Arc::into_inner)
        else {
            tracing::error!(
                method = %parts.method,
                path = parts.uri.path(),
                "an admin handler takes AdminSession, but the route table's guard opened no \
                 session for this request: mount the route through controllers::admin::routes \
                 with a requirement that needs a session (ADR 0007 §5)"
            );
            return ready(Err(Error::InternalServerError.into_response()));
        };
        if !opened.scope.is_in(scopes) {
            tracing::debug!(
                user_id = opened.user.id,
                scope = opened.scope.claim(),
                "admin session refused: the handler does not accept the scope that the route \
                 table accepted"
            );
            return ready(Err(error_response(ErrorCode::Unauthorized)));
        }
        ready(Ok(Self {
            user: opened.user,
            scope: opened.scope,
            grants: opened.grants,
            claims: opened.claims,
            carrier: opened.carrier,
            accepts: PhantomData,
        }))
    }
}

/// The answer to a session that [`open_session`] refused or could not open.
///
/// - `error`: what [`open_session`] returned.
///
/// `Unauthorized` (no token, a token that fails validation, an account that
/// the `pid` does not name, or one of the session rules) becomes the admin
/// `401 unauthorized` envelope; its reason was logged where it arose.
/// Everything else, a database failure above all, keeps Loco's own answer
/// and its log line, a `500`.
pub(super) fn session_refusal_response(error: Error) -> Response {
    match error {
        Error::Unauthorized(_) => error_response(ErrorCode::Unauthorized),
        other => other.into_response(),
    }
}

#[cfg(test)]
mod tests {
    use axum::{
        body::to_bytes,
        http::{Request, StatusCode},
    };

    use super::*;
    use crate::{
        domain::session_cookie::cleared_session_cookie, dtos::admin::error::ErrorEnvelope,
    };

    /// The head of a `GET` with `headers`.
    fn head_with(headers: &[(&str, &str)]) -> Parts {
        let mut request = Request::get("/api/admin/auth/me");
        for (name, value) in headers {
            request = request.header(*name, *value);
        }
        request.body(()).expect("request").into_parts().0
    }

    #[test]
    fn the_cookie_carries_the_token_whenever_the_request_sends_it() {
        let cookie = format!("theme=dark; {SESSION_COOKIE_NAME}=a.b.c");
        for (headers, carrier) in [
            (vec![("cookie", cookie.as_str())], TokenCarrier::Cookie),
            // Loco reads the cookie first, also beside a Bearer header.
            (
                vec![
                    ("cookie", cookie.as_str()),
                    ("authorization", "Bearer x.y.z"),
                ],
                TokenCarrier::Cookie,
            ),
            // An empty cookie is still the cookie Loco reads (and refuses).
            (
                vec![("cookie", "__Host-admin_session=")],
                TokenCarrier::Cookie,
            ),
            (
                vec![("authorization", "Bearer x.y.z")],
                TokenCarrier::Bearer,
            ),
            // Another cookie is not the session's.
            (
                vec![
                    ("cookie", "__Host-admin_mfa=a.b.c"),
                    ("authorization", "Bearer x"),
                ],
                TokenCarrier::Bearer,
            ),
            (vec![], TokenCarrier::Bearer),
        ] {
            assert_eq!(token_carrier(&head_with(&headers)), carrier, "{headers:?}");
        }
    }

    /// The `Set-Cookie` values of `response`.
    fn set_cookies(response: &Response) -> Vec<&str> {
        response
            .headers()
            .get_all(SET_COOKIE)
            .iter()
            .map(|value| value.to_str().expect("ASCII"))
            .collect()
    }

    #[test]
    fn the_passive_refresh_never_overrides_the_handler_s_session_cookie() {
        let renewed = HeaderValue::from_static("__Host-admin_session=new; Max-Age=7200");
        // No refresh: the answer is left as it is.
        let plain = with_passive_refresh(StatusCode::OK.into_response(), None);
        assert_eq!(set_cookies(&plain), [] as [&str; 0]);
        // A refresh joins an answer that sets no cookie, or another one.
        let joined = with_passive_refresh(StatusCode::OK.into_response(), Some(renewed.clone()));
        assert_eq!(
            set_cookies(&joined),
            ["__Host-admin_session=new; Max-Age=7200"]
        );
        let mut other = StatusCode::OK.into_response();
        other
            .headers_mut()
            .append(SET_COOKIE, HeaderValue::from_static("theme=dark"));
        let beside = with_passive_refresh(other, Some(renewed.clone()));
        assert_eq!(
            set_cookies(&beside),
            ["theme=dark", "__Host-admin_session=new; Max-Age=7200"]
        );
        // An answer that deletes or sets the session cookie keeps its own.
        for own in [
            cleared_session_cookie(),
            "__Host-admin_session=mine; Max-Age=60".to_owned(),
        ] {
            let mut response = StatusCode::NO_CONTENT.into_response();
            response.headers_mut().append(
                SET_COOKIE,
                HeaderValue::from_str(&own).expect("a header value"),
            );
            let kept = with_passive_refresh(response, Some(renewed.clone()));
            assert_eq!(set_cookies(&kept), [own.as_str()]);
        }
    }

    #[test]
    fn session_times_read_loco_s_private_exp() {
        let claims: UserClaims = serde_json::from_value(serde_json::json!({
            "pid": "11111111-2222-3333-4444-555555555555",
            "exp": 1_793_617_200_u64,
            "tv": 0,
            "scope": "admin",
            "auth_time": 1_793_610_000,
        }))
        .expect("Loco's claims");
        assert_eq!(
            session_times(&claims).expect("both claims"),
            SessionTimes {
                auth_time: 1_793_610_000,
                expires_at: 1_793_617_200,
            }
        );
        let without_auth_time: UserClaims = serde_json::from_value(serde_json::json!({
            "pid": "p",
            "exp": 1_793_617_200_u64,
        }))
        .expect("Loco's claims");
        let error = session_times(&without_auth_time).expect_err("no auth_time");
        assert!(error.to_string().contains("auth_time"), "{error}");
    }

    #[test]
    fn the_session_limits_come_from_the_settings() {
        let settings = Settings::from_section(Some(&serde_json::json!({
            "admin_ui": { "dist_dir": "x" },
            "metrics": { "binding": "127.0.0.1", "port": 9464 },
            "admin_auth": {
                "totp_encryption_key": crate::domain::totp_secret::PUBLISHED_LOCAL_KEY,
                "session_absolute_max_secs": 9000,
                "reset_link_ttl_secs": 1,
                "temp_password_ttl_secs": 1,
                "force_change_short_legacy_password": true,
            },
            "image_optimisation": {
                "pixel_budget": 50_000_000,
                "png": { "threshold_pixels": 6_000_000, "large_preset": 0 },
            },
            "import": { "source_database_url": "" },
            "media": {
                "root": "target/media/test/public",
                "tmp_dir": "target/media/test/tmp",
                "trash_dir": "target/media/test/trash",
                "min_free_bytes": 1,
                "public_base_url": "http://localhost/storage",
                "serve_storage": false,
            },
        })))
        .expect("valid settings");
        assert_eq!(SessionLimits::new(&settings).absolute.secs(), 9000);
    }

    #[test]
    fn no_marker_accepts_an_mfa_token() {
        assert_eq!(accepted_scopes::<AdminScope>(), &[SessionScope::Admin]);
        assert_eq!(
            accepted_scopes::<AdminOrPasswordChange>(),
            &[SessionScope::Admin, SessionScope::PasswordChange]
        );
        // The compile-time assertion of `accepted_scopes` checks the same
        // with `is_in`; this pins each marker's whole list.
        for scopes in [AdminScope::SCOPES, AdminOrPasswordChange::SCOPES] {
            assert!(!scopes.contains(&SessionScope::Mfa), "{scopes:?}");
            assert!(!SessionScope::Mfa.is_in(scopes), "{scopes:?}");
        }
    }

    #[tokio::test]
    async fn an_unauthorized_refusal_becomes_the_admin_envelope() {
        // Loco's three reasons, and one of the session rules.
        for reason in [
            "token is not found",
            "token is not valid",
            "not found",
            "the token's version is stale",
        ] {
            let response = session_refusal_response(Error::Unauthorized(reason.to_owned()));
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
            let response = session_refusal_response(error);
            assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
            let body = to_bytes(response.into_body(), 4096).await.expect("body");
            let body: serde_json::Value = serde_json::from_slice(&body).expect("Loco's JSON");
            assert_ne!(body["error"], "unauthorized", "{body}");
        }
    }

    #[tokio::test]
    async fn the_extractor_never_opens_a_session_itself() {
        // A request that no guard saw, even one that carries a token: the
        // extractor answers 500 and reads nothing, so a handler mounted
        // outside the route table never serves (ADR 0007 §5).
        let (mut parts, ()) = Request::get("/api/admin/x")
            .header("authorization", "Bearer not-even-a-token")
            .body(())
            .expect("request")
            .into_parts();
        for result in [
            AdminSession::<AdminScope>::from_request_parts(&mut parts, &())
                .await
                .map(|_| ()),
            AdminSession::<AdminOrPasswordChange>::from_request_parts(&mut parts, &())
                .await
                .map(|_| ()),
        ] {
            let response = result.expect_err("no session was opened");
            assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
        }
    }
}
