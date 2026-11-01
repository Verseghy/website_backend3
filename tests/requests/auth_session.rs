//! The admin session extractor and `GET /api/admin/auth/me` (ADR 0006 §1,
//! §2, §4; ADR 0007 §7).
//!
//! `/auth/me` answers the signed-in account to the sign-in cookie and to a
//! Bearer token, for an `admin` and a `password_change` session. Every
//! refusal answers byte for byte the same `401 unauthorized`: no token, a
//! token that is not valid, expired, signed with another key, signed with
//! another algorithm or not signed at all, an unknown account, a trashed,
//! anonymised or inactive account, a stale or missing `tv`, and a scope the
//! route does not accept. The session cookie is read before the Bearer
//! header. The account is read on every request, so an account event
//! applies to the next one (ADR 0016), and a database failure answers `500`,
//! never `401`. Every account here is invented.

use axum::{
    extract::FromRequestParts,
    http::{
        HeaderMap, Request, StatusCode,
        header::{AUTHORIZATION, COOKIE},
    },
};
use axum_test::{TestResponse, TestServer};
use backend3::{
    controllers::admin::session::{AdminOrPasswordChange, AdminScope, AdminSession},
    domain::{session::SessionScope, session_cookie::SESSION_COOKIE_NAME},
    models::users::{self, ActiveModel, Model},
};
use base64::{
    Engine as _,
    engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD},
};
use chrono::{DateTime, Utc};
use hmac::{Hmac, KeyInit as _, Mac as _};
use loco_rs::{
    app::AppContext,
    auth::jwt::JWT,
    prelude::{ActiveModelTrait, ActiveValue, IntoActiveModel},
};
use serde_json::{Map, Value, json};
use sha2::{Sha256, Sha512};

use crate::support::with_server;

const ME: &str = "/api/admin/auth/me";
/// The password of the account that signs in through the endpoint.
const PASSWORD: &str = "helyes jelszó 2026";

/// The instant the invented accounts record and their tokens' `auth_time`.
fn instant() -> DateTime<Utc> {
    DateTime::parse_from_rfc3339("2026-10-25T08:00:00Z")
        .expect("RFC 3339")
        .with_timezone(&Utc)
}

/// Inserts a live, active account with `email` and no password, after
/// `change` has adjusted it.
async fn account(ctx: &AppContext, email: &str, change: impl FnOnce(&mut ActiveModel)) -> Model {
    let mut new = ActiveModel {
        email: ActiveValue::Set(Some(email.to_owned())),
        name: ActiveValue::Set("Próba Szerkesztő".to_owned()),
        ..Default::default()
    };
    change(&mut new);
    new.insert(&ctx.db).await.expect("insert the account")
}

/// Saves `change` to the stored `user`, as an account event would.
async fn update(ctx: &AppContext, user: Model, change: impl FnOnce(&mut ActiveModel)) -> Model {
    let mut active = user.into_active_model();
    change(&mut active);
    active.update(&ctx.db).await.expect("update the account")
}

/// `auth.jwt.secret` of config/test.yaml.
fn secret(ctx: &AppContext) -> String {
    ctx.config
        .get_jwt_config()
        .expect("auth.jwt")
        .secret
        .clone()
}

/// The full `admin` session token that sign-in issues for `user`.
fn admin_token(ctx: &AppContext, user: &Model) -> String {
    user.generate_session_token(&secret(ctx), 7200, instant())
        .expect("signed")
}

/// A token signed with the configured secret through Loco, for `pid`, with
/// exactly `claims` as its custom claims.
fn token_with(ctx: &AppContext, pid: &str, claims: &Value) -> String {
    let claims: Map<String, Value> = claims.as_object().expect("an object").clone();
    JWT::new(&secret(ctx))
        .generate_token(7200, pid.to_owned(), claims)
        .expect("signed")
}

/// The custom claims of a session token at version `tv` with `scope`.
fn session_claims(tv: i32, scope: SessionScope) -> Value {
    json!({ "tv": tv, "scope": scope.claim(), "auth_time": instant().timestamp() })
}

/// How [`hand_made`] signs a token: the `alg` of its header and the
/// signature that goes with it.
#[derive(Debug, Clone, Copy)]
enum Signing {
    /// HMAC-SHA-512, the algorithm Loco signs and accepts.
    Hs512,
    /// HMAC-SHA-256 with the same key: another algorithm of the same family,
    /// which a verifier that trusted the header's `alg` would accept.
    Hs256,
    /// `alg: none` and an empty signature: an unsigned token.
    Unsigned,
}

/// A JWT over `claims`, made by hand with the base64 key `secret` as
/// jsonwebtoken (Loco's signer) makes one, signed as `signing` says. It
/// lets a test choose `exp`, which Loco always sets to the wall clock plus
/// a lifetime, and the algorithm, which Loco fixes.
fn hand_made(signing: Signing, secret: &str, claims: &Value) -> String {
    let key = STANDARD.decode(secret).expect("a base64 secret");
    let alg = match signing {
        Signing::Hs512 => "HS512",
        Signing::Hs256 => "HS256",
        Signing::Unsigned => "none",
    };
    let header = URL_SAFE_NO_PAD.encode(format!(r#"{{"typ":"JWT","alg":"{alg}"}}"#));
    let payload = URL_SAFE_NO_PAD.encode(serde_json::to_vec(claims).expect("serializes"));
    let signing_input = format!("{header}.{payload}");
    let signature = match signing {
        Signing::Hs512 => {
            let mut mac = Hmac::<Sha512>::new_from_slice(&key).expect("HMAC takes any key length");
            mac.update(signing_input.as_bytes());
            mac.finalize().into_bytes().to_vec()
        }
        Signing::Hs256 => {
            let mut mac = Hmac::<Sha256>::new_from_slice(&key).expect("HMAC takes any key length");
            mac.update(signing_input.as_bytes());
            mac.finalize().into_bytes().to_vec()
        }
        Signing::Unsigned => Vec::new(),
    };
    format!("{signing_input}.{}", URL_SAFE_NO_PAD.encode(signature))
}

/// `GET /auth/me` with `token` in the session cookie, beside another cookie
/// as a browser sends it.
async fn me_with_cookie(server: &TestServer, token: &str) -> TestResponse {
    server
        .get(ME)
        .add_header(COOKIE, format!("theme=dark; {SESSION_COOKIE_NAME}={token}"))
        .await
}

/// `GET /auth/me` with `token` as a Bearer token, as a script sends it.
async fn me_with_bearer(server: &TestServer, token: &str) -> TestResponse {
    server.get(ME).authorization_bearer(token).await
}

/// Everything a client can see of a response, except the two headers that
/// differ on every response: the `x-request-id` and the `date`.
fn observable(response: &TestResponse) -> (StatusCode, HeaderMap, Vec<u8>) {
    let mut headers = response.headers().clone();
    headers.remove("x-request-id");
    headers.remove("date");
    (
        response.status_code(),
        headers,
        response.as_bytes().to_vec(),
    )
}

/// The `/auth/me` answer for `user` with `scope`.
fn me_body(user: &Model, scope: &str) -> Value {
    json!({
        "pid": user.pid.to_string(),
        "name": user.name,
        "email": user.email,
        "scope": scope
    })
}

#[tokio::test]
async fn me_answers_the_signed_in_account_to_the_sign_in_cookie_and_to_bearer() {
    with_server(|server, ctx| async move {
        let user = account(&ctx, "szerkeszto@example.test", |new| {
            new.password_hash =
                ActiveValue::Set(Some(users::hash_password(PASSWORD).expect("hash")));
        })
        .await;
        // The whole flow: the cookie that sign-in sets opens the session.
        let login = server
            .post("/api/admin/auth/login")
            .json(&json!({ "email": "szerkeszto@example.test", "password": PASSWORD }))
            .await;
        assert_eq!(login.status_code(), StatusCode::OK);
        let cookie = login.header("set-cookie");
        let token = cookie
            .to_str()
            .expect("ASCII")
            .split_once(';')
            .and_then(|(pair, _)| pair.strip_prefix(&format!("{SESSION_COOKIE_NAME}=")))
            .expect("the session cookie")
            .to_owned();

        for (case, response) in [
            ("cookie", me_with_cookie(&server, &token).await),
            ("bearer", me_with_bearer(&server, &token).await),
        ] {
            assert_eq!(response.status_code(), StatusCode::OK, "{case}");
            assert_eq!(response.json::<Value>(), me_body(&user, "admin"), "{case}");
            assert_eq!(response.header("cache-control"), "no-store", "{case}");
            assert_eq!(
                response.header("content-type"),
                "application/json",
                "{case}"
            );
            // Reading the session sets no cookie: the refresh is its own
            // slice (ADR 0006 §1).
            assert!(response.maybe_header("set-cookie").is_none(), "{case}");
        }
    })
    .await;
}

#[tokio::test]
async fn a_password_change_session_reaches_me() {
    // ADR 0006 §4: `/auth/me` is one of the three routes a
    // `password_change` token may reach, so the admin UI can send the user
    // to the password form.
    with_server(|server, ctx| async move {
        // A password hash and a TOTP secret in the row, so the exact body
        // below shows that neither goes out. Neither protects anything: the
        // hash text is no hash, and the secret bytes encrypt nothing.
        let user = account(&ctx, "csere@example.test", |new| {
            new.token_version = ActiveValue::Set(2);
            new.password_hash = ActiveValue::Set(Some("not a hash of anything".to_owned()));
            new.totp_secret_encrypted = ActiveValue::Set(Some(vec![7; 48]));
            new.totp_enabled_at = ActiveValue::Set(Some(instant().fixed_offset()));
        })
        .await;
        let token = token_with(
            &ctx,
            &user.pid.to_string(),
            &session_claims(2, SessionScope::PasswordChange),
        );
        let response = me_with_cookie(&server, &token).await;
        assert_eq!(response.status_code(), StatusCode::OK);
        assert_eq!(response.json::<Value>(), me_body(&user, "password_change"));
    })
    .await;
}

/// The `401 unauthorized` envelope.
fn unauthorized() -> Value {
    json!({
        "error": "unauthorized",
        "description": "There is no valid admin session; sign in again",
        "field": null,
        "params": {}
    })
}

/// A key made at run time, not the configured one: every byte of the
/// configured key flipped.
fn other_secret(secret: &str) -> String {
    let flipped: Vec<u8> = STANDARD
        .decode(secret)
        .expect("base64")
        .iter()
        .map(|byte| !byte)
        .collect();
    STANDARD.encode(flipped)
}

/// The claims of an `admin` token for `pid` at version 0 that expires at
/// `exp`, in seconds since the Unix epoch.
fn admin_claims_until(pid: &str, exp: i64) -> Value {
    let mut claims = session_claims(0, SessionScope::Admin);
    claims["pid"] = json!(pid);
    claims["exp"] = json!(exp);
    claims
}

/// An expiry far ahead, 2100-01-01T00:00:00Z.
const FAR_AHEAD: i64 = 4_102_444_800;

/// A hand-signed HS512 `admin` token for `pid` at version 0 that expires at
/// `exp`, in seconds since the Unix epoch.
fn signed_until(ctx: &AppContext, pid: &str, exp: i64) -> String {
    hand_made(Signing::Hs512, &secret(ctx), &admin_claims_until(pid, exp))
}

/// Requests whose token Loco's own extractor refuses, for the live account
/// `live`: no token, a token it cannot read or verify, one signed with
/// another algorithm or not at all, an expired one, and a `pid` that names
/// no account.
async fn token_refusals(
    server: &TestServer,
    ctx: &AppContext,
    live: &Model,
) -> Vec<(&'static str, TestResponse)> {
    let pid = live.pid.to_string();
    let admin = session_claims(0, SessionScope::Admin);
    let token = admin_token(ctx, live);
    let foreign = JWT::new(&other_secret(&secret(ctx)))
        .generate_token(
            7200,
            pid.clone(),
            admin.as_object().expect("object").clone(),
        )
        .expect("signed");
    vec![
        // Neither a cookie nor a header.
        ("no token", server.get(ME).await),
        (
            "the token under another cookie name",
            server
                .get(ME)
                .add_header(COOKIE, format!("admin_session={token}"))
                .await,
        ),
        (
            "another authorization scheme",
            server
                .get(ME)
                .add_header(AUTHORIZATION, format!("Basic {token}"))
                .await,
        ),
        // The cookie is there, with nothing in it.
        ("an empty cookie value", me_with_cookie(server, "").await),
        ("not a JWT", me_with_cookie(server, "nem.jwt.token").await),
        (
            "signed with another key",
            me_with_cookie(server, &foreign).await,
        ),
        // Both hold valid claims with an `exp` far ahead, so each fails for
        // its algorithm alone: Loco accepts HS512 only, whatever the header
        // names (`Validation::new` with the configured algorithm).
        (
            "HS256 with the configured key",
            me_with_cookie(
                server,
                &hand_made(
                    Signing::Hs256,
                    &secret(ctx),
                    &admin_claims_until(&pid, FAR_AHEAD),
                ),
            )
            .await,
        ),
        (
            "alg none, unsigned",
            me_with_cookie(
                server,
                &hand_made(
                    Signing::Unsigned,
                    &secret(ctx),
                    &admin_claims_until(&pid, FAR_AHEAD),
                ),
            )
            .await,
        ),
        (
            "expired",
            me_with_cookie(server, &signed_until(ctx, &pid, 1_000_000_000)).await,
        ),
        (
            "an account that does not exist",
            me_with_cookie(
                server,
                &token_with(ctx, "5d0f8a52-7c1e-4b9a-9e3d-2a6b8c4f1e07", &admin),
            )
            .await,
        ),
        (
            "a pid that is not a UUID",
            me_with_cookie(server, &token_with(ctx, "aktiv@example.test", &admin)).await,
        ),
    ]
}

/// Requests with a well-signed, unexpired token that the session check
/// refuses: the account's state, the `tv` claim and the `scope` claim. The
/// accounts other than `live` are created here.
async fn session_refusals(
    server: &TestServer,
    ctx: &AppContext,
    live: &Model,
) -> Vec<(&'static str, TestResponse)> {
    let trashed = account(ctx, "lomtar@example.test", |new| {
        new.deleted_at = ActiveValue::Set(Some(instant().fixed_offset()));
    })
    .await;
    // The shape the anonymisation CHECKs require (ADR 0006 §7).
    let anonymised = account(ctx, "anonim@example.test", |new| {
        new.email = ActiveValue::Set(None);
        new.active = ActiveValue::Set(false);
        new.deleted_at = ActiveValue::Set(Some(instant().fixed_offset()));
        new.anonymised_at = ActiveValue::Set(Some(instant().fixed_offset()));
    })
    .await;
    let inactive = account(ctx, "inaktiv@example.test", |new| {
        new.active = ActiveValue::Set(false);
    })
    .await;
    // An account event incremented the version after the token was signed
    // at 0 (ADR 0006 §2).
    let revoked = account(ctx, "visszavont@example.test", |new| {
        new.token_version = ActiveValue::Set(1);
    })
    .await;
    let pid = live.pid.to_string();
    let admin = session_claims(0, SessionScope::Admin);
    let claims_of = |claims: Value| token_with(ctx, &pid, &claims);
    let mfa = claims_of(session_claims(0, SessionScope::Mfa));
    vec![
        (
            "trashed",
            me_with_cookie(server, &admin_token(ctx, &trashed)).await,
        ),
        (
            "anonymised",
            me_with_cookie(server, &admin_token(ctx, &anonymised)).await,
        ),
        (
            "inactive",
            me_with_cookie(server, &admin_token(ctx, &inactive)).await,
        ),
        (
            "a stale tv",
            me_with_cookie(server, &token_with(ctx, &revoked.pid.to_string(), &admin)).await,
        ),
        (
            "no tv",
            me_with_cookie(server, &claims_of(json!({ "scope": "admin" }))).await,
        ),
        (
            "a tv that is text",
            me_with_cookie(server, &claims_of(json!({ "tv": "0", "scope": "admin" }))).await,
        ),
        (
            "no scope",
            me_with_cookie(server, &claims_of(json!({ "tv": 0 }))).await,
        ),
        (
            "an unknown scope",
            me_with_cookie(server, &claims_of(json!({ "tv": 0, "scope": "root" }))).await,
        ),
        // ADR 0006 follow-up work: an mfa token sent as the session cookie
        // or as a Bearer token is rejected everywhere but /totp.
        ("an mfa cookie", me_with_cookie(server, &mfa).await),
        ("an mfa Bearer token", me_with_bearer(server, &mfa).await),
    ]
}

#[tokio::test]
async fn every_refused_session_gets_the_same_401() {
    with_server(|server, ctx| async move {
        let live = account(&ctx, "aktiv@example.test", |_| {}).await;
        // The hand-made signer is right: with an `exp` ahead the session
        // opens, so the expired case fails for its `exp` alone, and the
        // HS256 and unsigned cases for their algorithm alone.
        let ahead = signed_until(&ctx, &live.pid.to_string(), FAR_AHEAD);
        let ahead = me_with_cookie(&server, &ahead).await;
        assert_eq!(ahead.status_code(), StatusCode::OK);
        assert_eq!(ahead.json::<Value>(), me_body(&live, "admin"));

        let mut responses = token_refusals(&server, &ctx, &live).await;
        responses.extend(session_refusals(&server, &ctx, &live).await);
        let (_, first) = &responses[0];
        assert_eq!(first.status_code(), StatusCode::UNAUTHORIZED);
        assert_eq!(first.json::<Value>(), unauthorized());
        assert_eq!(first.header("cache-control"), "no-store");
        assert_eq!(first.header("content-type"), "application/json");
        assert!(first.maybe_header("set-cookie").is_none());
        for (case, response) in &responses {
            assert_eq!(observable(response), observable(first), "{case}");
        }
        // Nothing above changed the live account: its own token still works.
        let response = me_with_cookie(&server, &admin_token(&ctx, &live)).await;
        assert_eq!(response.status_code(), StatusCode::OK);
    })
    .await;
}

#[tokio::test]
async fn the_cookie_is_read_before_the_bearer_header() {
    // `auth.jwt.location` lists the session cookie, then the Bearer header,
    // in every configuration (ADR 0006 §1). Loco takes the token from the
    // first location that holds one and never falls back to the next when
    // that token fails, so a request carrying both is judged by its cookie.
    with_server(|server, ctx| async move {
        let user = account(&ctx, "mindketto@example.test", |_| {}).await;
        let token = admin_token(&ctx, &user);

        let bad_cookie = server
            .get(ME)
            .add_header(COOKIE, format!("{SESSION_COOKIE_NAME}=nem.jwt.token"))
            .authorization_bearer(&token)
            .await;
        assert_eq!(bad_cookie.status_code(), StatusCode::UNAUTHORIZED);
        assert_eq!(bad_cookie.json::<Value>(), unauthorized());

        let bad_bearer = server
            .get(ME)
            .add_header(COOKIE, format!("{SESSION_COOKIE_NAME}={token}"))
            .authorization_bearer("nem.jwt.token")
            .await;
        assert_eq!(bad_bearer.status_code(), StatusCode::OK);
        assert_eq!(bad_bearer.json::<Value>(), me_body(&user, "admin"));
    })
    .await;
}

#[tokio::test]
async fn the_account_is_read_on_every_request() {
    // The user lookup is never cached (ADR 0016): an account event applies
    // to the very next request with the same token (ADR 0006 §2, §7).
    with_server(|server, ctx| async move {
        let user = account(&ctx, "friss@example.test", |_| {}).await;
        let token = admin_token(&ctx, &user);
        assert_eq!(
            me_with_cookie(&server, &token).await.status_code(),
            StatusCode::OK
        );

        // Revocation: the version moves on, the old token stops working,
        // and a token at the new version works.
        let user = update(&ctx, user, |active| {
            active.token_version = ActiveValue::Set(1);
        })
        .await;
        let stale = me_with_cookie(&server, &token).await;
        assert_eq!(stale.status_code(), StatusCode::UNAUTHORIZED);
        assert_eq!(stale.json::<Value>(), unauthorized());
        let token = admin_token(&ctx, &user);
        assert_eq!(
            me_with_cookie(&server, &token).await.status_code(),
            StatusCode::OK
        );

        // Deactivation, and back: the same token follows the stored flag.
        let user = update(&ctx, user, |active| {
            active.active = ActiveValue::Set(false);
        })
        .await;
        assert_eq!(
            me_with_cookie(&server, &token).await.status_code(),
            StatusCode::UNAUTHORIZED
        );
        let user = update(&ctx, user, |active| {
            active.active = ActiveValue::Set(true);
        })
        .await;
        assert_eq!(
            me_with_cookie(&server, &token).await.status_code(),
            StatusCode::OK
        );

        // The trash.
        update(&ctx, user, |active| {
            active.deleted_at = ActiveValue::Set(Some(instant().fixed_offset()));
        })
        .await;
        assert_eq!(
            me_with_cookie(&server, &token).await.status_code(),
            StatusCode::UNAUTHORIZED
        );
    })
    .await;
}

/// Runs the extractor for the scopes `A` on a request that carries `token`
/// in the session cookie, and returns the session's scope or the refusal's
/// status.
async fn extract<A: backend3::controllers::admin::session::AcceptedScopes>(
    ctx: &AppContext,
    token: &str,
) -> Result<SessionScope, StatusCode> {
    let request = Request::get("/api/admin/x")
        .header(COOKIE, format!("{SESSION_COOKIE_NAME}={token}"))
        .body(())
        .expect("request");
    let (mut parts, ()) = request.into_parts();
    match AdminSession::<A>::from_request_parts(&mut parts, ctx).await {
        Ok(session) => Ok(session.scope),
        Err(response) => Err(response.status()),
    }
}

#[tokio::test]
async fn a_full_session_route_refuses_a_password_change_session() {
    // No route that takes only `AdminSession` exists yet, so the extractor
    // is called directly: the default marker accepts `admin` only, and
    // the own-account marker `admin` and `password_change` (ADR 0006 §4).
    with_server(|_server, ctx| async move {
        let user = account(&ctx, "hatokor@example.test", |_| {}).await;
        let pid = user.pid.to_string();
        let admin = admin_token(&ctx, &user);
        let password_change =
            token_with(&ctx, &pid, &session_claims(0, SessionScope::PasswordChange));
        let mfa = token_with(&ctx, &pid, &session_claims(0, SessionScope::Mfa));

        assert_eq!(
            extract::<AdminScope>(&ctx, &admin).await,
            Ok(SessionScope::Admin)
        );
        assert_eq!(
            extract::<AdminScope>(&ctx, &password_change).await,
            Err(StatusCode::UNAUTHORIZED)
        );
        assert_eq!(
            extract::<AdminScope>(&ctx, &mfa).await,
            Err(StatusCode::UNAUTHORIZED)
        );
        assert_eq!(
            extract::<AdminOrPasswordChange>(&ctx, &admin).await,
            Ok(SessionScope::Admin)
        );
        assert_eq!(
            extract::<AdminOrPasswordChange>(&ctx, &password_change).await,
            Ok(SessionScope::PasswordChange)
        );
        assert_eq!(
            extract::<AdminOrPasswordChange>(&ctx, &mfa).await,
            Err(StatusCode::UNAUTHORIZED)
        );
        // The session carries the account as read for the request.
        let request = Request::get("/api/admin/x")
            .header(AUTHORIZATION, format!("Bearer {admin}"))
            .body(())
            .expect("request");
        let (mut parts, ()) = request.into_parts();
        let Ok(session) = AdminSession::<AdminScope>::from_request_parts(&mut parts, &ctx).await
        else {
            panic!("the admin token opens a session");
        };
        assert_eq!(
            (session.user.id, session.claims.pid.as_str()),
            (user.id, pid.as_str())
        );
    })
    .await;
}

#[tokio::test]
async fn a_broken_jwt_configuration_answers_as_the_extractor_documents() {
    // Pins the extractor's documentation against loco-rs 1.2.0. A missing
    // `auth` section is a server fault, `500`. A secret that is not base64
    // fails inside Loco's token validation, which reports it as a token that
    // is not valid, so it answers `401`; ADR 0034 §3 places a boot check of
    // the secret so that such a configuration never serves.
    with_server(|_server, ctx| async move {
        let user = account(&ctx, "beallitas@example.test", |_| {}).await;
        let token = admin_token(&ctx, &user);
        assert_eq!(
            extract::<AdminScope>(&ctx, &token).await,
            Ok(SessionScope::Admin)
        );

        let mut not_base64 = ctx.clone();
        not_base64
            .config
            .auth
            .as_mut()
            .and_then(|auth| auth.jwt.as_mut())
            .expect("auth.jwt")
            .secret = "nem base64!".to_owned();
        assert_eq!(
            extract::<AdminScope>(&not_base64, &token).await,
            Err(StatusCode::UNAUTHORIZED)
        );

        let mut no_auth = ctx.clone();
        no_auth.config.auth = None;
        assert_eq!(
            extract::<AdminScope>(&no_auth, &token).await,
            Err(StatusCode::INTERNAL_SERVER_ERROR)
        );
    })
    .await;
}

#[tokio::test]
async fn a_database_failure_answers_500_never_401() {
    // The admin UI signs out on 401 (ADR 0006, follow-up work), so a
    // database that cannot be reached must not answer it.
    with_server(|server, ctx| async move {
        let user = account(&ctx, "kieses@example.test", |_| {}).await;
        let token = admin_token(&ctx, &user);
        // `with_app` closes the pool again afterwards, which is a no-op.
        ctx.db.close_by_ref().await.expect("close the pool");
        let response = me_with_cookie(&server, &token).await;
        assert_eq!(response.status_code(), StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(
            response.json::<Value>()["error"],
            json!("internal_server_error")
        );
        assert_eq!(response.header("cache-control"), "no-store");
    })
    .await;
}
