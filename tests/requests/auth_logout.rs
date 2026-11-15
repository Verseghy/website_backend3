//! `POST /api/admin/auth/logout` (ADR 0006 §2, §4; parity AUTH-04).
//!
//! Logout ends every session of the account: a second, independently issued
//! token of the same account stops working too, as a cookie and as a Bearer
//! token, while other accounts' sessions hold. It answers `204` with the
//! `Set-Cookie` that deletes the session cookie, pinned as a snapshot, and
//! leaves `updated_at` and `updated_by` alone (ADR 0011 §4). That
//! `Set-Cookie` is the only one, also for a token close to its expiry,
//! which the passive refresh of ADR 0006 §1 must leave alone here. A
//! `password_change` session may log out. Without a valid session it
//! answers the route table guard's `401` and revokes nothing. Every other method
//! answers `405` and revokes nothing. A database failure answers `500`, and
//! then the cookie stays and the sessions hold. Every account and password
//! here is invented.

use axum::http::{
    Method, StatusCode,
    header::{ALLOW, COOKIE},
};
use axum_test::{TestResponse, TestServer};
use backend3::{
    domain::{session::SessionScope, session_cookie::SESSION_COOKIE_NAME},
    models::users::{self, ActiveModel, Model},
};
use chrono::{DateTime, TimeDelta, Utc};
use loco_rs::{
    app::AppContext,
    auth::jwt::JWT,
    prelude::{ActiveModelTrait, ActiveValue, ConnectionTrait},
};
use serde_json::{Map, Value, json};

use crate::support::with_server;

const LOGIN: &str = "/api/admin/auth/login";
const LOGOUT: &str = "/api/admin/auth/logout";
const ME: &str = "/api/admin/auth/me";
/// The password of the accounts that sign in through the endpoint.
const PASSWORD: &str = "helyes jelszó 2026";

/// The instant the invented accounts record as `updated_at`, and the
/// `auth_time` of the tokens made here.
fn instant() -> DateTime<Utc> {
    DateTime::parse_from_rfc3339("2026-10-25T08:00:00Z")
        .expect("RFC 3339")
        .with_timezone(&Utc)
}

/// Inserts a live, active account with `email`, the Argon2id hash of
/// [`PASSWORD`] and `updated_at` at [`instant`], after `change` has
/// adjusted it.
async fn account(ctx: &AppContext, email: &str, change: impl FnOnce(&mut ActiveModel)) -> Model {
    let mut new = ActiveModel {
        email: ActiveValue::Set(Some(email.to_owned())),
        name: ActiveValue::Set("Próba Szerkesztő".to_owned()),
        password_hash: ActiveValue::Set(Some(users::hash_password(PASSWORD).expect("hash"))),
        updated_at: ActiveValue::Set(instant().fixed_offset()),
        ..Default::default()
    };
    change(&mut new);
    new.insert(&ctx.db).await.expect("insert the account")
}

/// The stored row of `user`.
async fn stored(ctx: &AppContext, user: &Model) -> Model {
    Model::find_by_pid(&ctx.db, &user.pid.to_string())
        .await
        .expect("the stored account")
}

/// `auth.jwt.secret` of config/test.yaml.
fn secret(ctx: &AppContext) -> String {
    ctx.config
        .get_jwt_config()
        .expect("auth.jwt")
        .secret
        .clone()
}

/// A full `admin` session token for `user` with `auth_time`, as sign-in
/// issues one.
fn admin_token_at(ctx: &AppContext, user: &Model, auth_time: DateTime<Utc>) -> String {
    user.generate_session_token(&secret(ctx), 7200, auth_time)
        .expect("signed")
}

/// A token signed with the configured secret for `user` at `user`'s
/// version, with `scope`.
fn scoped_token(ctx: &AppContext, user: &Model, scope: SessionScope) -> String {
    let claims: Map<String, Value> = json!({
        "tv": user.token_version,
        "scope": scope.claim(),
        "auth_time": instant().timestamp(),
    })
    .as_object()
    .expect("an object")
    .clone();
    JWT::new(&secret(ctx))
        .generate_token(7200, user.pid.to_string(), claims)
        .expect("signed")
}

/// Signs in through the endpoint and returns the token of the session
/// cookie it sets.
async fn sign_in(server: &TestServer, email: &str) -> String {
    let response = server
        .post(LOGIN)
        .json(&json!({ "email": email, "password": PASSWORD }))
        .await;
    assert_eq!(response.status_code(), StatusCode::OK);
    response
        .header("set-cookie")
        .to_str()
        .expect("ASCII")
        .split_once(';')
        .and_then(|(pair, _)| pair.strip_prefix(&format!("{SESSION_COOKIE_NAME}=")))
        .expect("the session cookie")
        .to_owned()
}

/// The cookie header a browser sends with `token` in the session cookie,
/// beside another cookie.
fn session_cookie_header(token: &str) -> String {
    format!("theme=dark; {SESSION_COOKIE_NAME}={token}")
}

/// `POST /auth/logout` with `token` in the session cookie.
async fn logout_with_cookie(server: &TestServer, token: &str) -> TestResponse {
    server
        .post(LOGOUT)
        .add_header(COOKIE, session_cookie_header(token))
        .await
}

/// `POST /auth/logout` with `token` as a Bearer token.
async fn logout_with_bearer(server: &TestServer, token: &str) -> TestResponse {
    server.post(LOGOUT).authorization_bearer(token).await
}

/// The status of `GET /auth/me` with `token` in the cookie and as a Bearer
/// token, in that order.
async fn me_statuses(server: &TestServer, token: &str) -> [StatusCode; 2] {
    let cookie = server
        .get(ME)
        .add_header(COOKIE, session_cookie_header(token))
        .await;
    let bearer = server.get(ME).authorization_bearer(token).await;
    [cookie.status_code(), bearer.status_code()]
}

const OPEN: [StatusCode; 2] = [StatusCode::OK, StatusCode::OK];
const ENDED: [StatusCode; 2] = [StatusCode::UNAUTHORIZED, StatusCode::UNAUTHORIZED];

/// What [`set_cookies`] returns for a response that sets no cookie.
const NO_COOKIES: [String; 0] = [];

/// Every `Set-Cookie` value of `response`.
fn set_cookies(response: &TestResponse) -> Vec<String> {
    response
        .iter_headers_by_name("set-cookie")
        .map(|value| value.to_str().expect("ASCII").to_owned())
        .collect()
}

/// Asserts that `response` is the logout's success: `204`, no body, no
/// caching, and exactly the one `Set-Cookie` that deletes the session
/// cookie.
fn assert_logged_out(response: &TestResponse, case: &str) {
    assert_eq!(response.status_code(), StatusCode::NO_CONTENT, "{case}");
    assert!(response.as_bytes().is_empty(), "{case}");
    assert_eq!(response.header("cache-control"), "no-store", "{case}");
    let cookies = set_cookies(response);
    let [cookie] = &cookies[..] else {
        panic!("{case}: one Set-Cookie expected: {cookies:?}");
    };
    assert_eq!(
        cookie, "__Host-admin_session=; HttpOnly; Secure; SameSite=Strict; Path=/; Max-Age=0",
        "{case}"
    );
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

#[tokio::test]
async fn logout_ends_every_session_of_the_account_and_clears_the_cookie() {
    with_server(|server, ctx| async move {
        let user = account(&ctx, "kilepo@example.test", |_| {}).await;
        let colleague = account(&ctx, "kollega@example.test", |_| {}).await;
        // The browser session, and a second session of the same account
        // issued on its own (another device, a script): their `auth_time`
        // differs, so the two tokens differ.
        let browser = sign_in(&server, "kilepo@example.test").await;
        let second = admin_token_at(&ctx, &user, instant() - TimeDelta::hours(1));
        assert_ne!(browser, second);
        let colleague_token = admin_token_at(&ctx, &colleague, instant());
        for token in [&browser, &second, &colleague_token] {
            assert_eq!(me_statuses(&server, token).await, OPEN);
        }
        let before = stored(&ctx, &user).await;

        let response = logout_with_cookie(&server, &browser).await;
        assert_logged_out(&response, "cookie");
        // ADR 0006, follow-up work: a snapshot pins the attributes of the
        // cookie that logout clears, beside the sign-in's in auth_login.rs.
        insta::assert_snapshot!(
            response.header("set-cookie").to_str().expect("ASCII"),
            @"__Host-admin_session=; HttpOnly; Secure; SameSite=Strict; Path=/; Max-Age=0"
        );

        // Every token of the account ends, through either location; the
        // colleague's session holds.
        assert_eq!(me_statuses(&server, &browser).await, ENDED);
        assert_eq!(me_statuses(&server, &second).await, ENDED);
        assert_eq!(me_statuses(&server, &colleague_token).await, OPEN);
        let ended = server
            .get(ME)
            .add_header(COOKIE, session_cookie_header(&second))
            .await;
        assert_eq!(ended.json::<Value>(), unauthorized());

        // An account event (ADR 0011 §4): the version moved by one, and
        // nothing else did, `updated_at` and `updated_by` included.
        let after = stored(&ctx, &user).await;
        assert_eq!(after.token_version, before.token_version + 1);
        assert_eq!(
            (after.updated_at, after.updated_by),
            (instant().fixed_offset(), None)
        );
        assert_eq!(
            Model {
                token_version: before.token_version,
                ..after
            },
            before
        );
        assert_eq!(stored(&ctx, &colleague).await, colleague);

        // The account signs in again, at the new version.
        let again = sign_in(&server, "kilepo@example.test").await;
        assert_eq!(me_statuses(&server, &again).await, OPEN);
    })
    .await;
}

#[tokio::test]
async fn a_bearer_logout_ends_the_browser_session_too() {
    // Logout through a script's Bearer token ends the browser's cookie
    // session of the same account (ADR 0006 §2). The clearing `Set-Cookie`
    // is sent anyway; a client that sent no cookie ignores it.
    with_server(|server, ctx| async move {
        account(&ctx, "szkript@example.test", |_| {}).await;
        let browser = sign_in(&server, "szkript@example.test").await;
        let user = Model::find_live_by_email(&ctx.db, "szkript@example.test")
            .await
            .expect("the account");
        let script = admin_token_at(&ctx, &user, instant());

        assert_logged_out(&logout_with_bearer(&server, &script).await, "bearer");
        assert_eq!(me_statuses(&server, &script).await, ENDED);
        assert_eq!(me_statuses(&server, &browser).await, ENDED);
        assert_eq!(stored(&ctx, &user).await.token_version, 1);
    })
    .await;
}

#[tokio::test]
async fn logout_near_expiry_sets_only_the_clearing_cookie() {
    // ADR 0006 §1 adds a passive refresh: an `admin` request with less than
    // 3,600 s left gets a new cookie. A browser keeps the last cookie of a
    // name that an answer sets, so a refreshed token beside the clearing
    // value would leave a cookie behind the logout. The refresh must leave
    // logout out: with 600 s left, the answer still carries the clearing
    // `Set-Cookie` alone, and the session ends.
    with_server(|server, ctx| async move {
        let user = account(&ctx, "lejaro@example.test", |_| {}).await;
        let near_expiry = user
            .generate_session_token(&secret(&ctx), 600, instant())
            .expect("signed");
        assert_eq!(me_statuses(&server, &near_expiry).await, OPEN);

        assert_logged_out(
            &logout_with_cookie(&server, &near_expiry).await,
            "600 s left",
        );
        assert_eq!(me_statuses(&server, &near_expiry).await, ENDED);
        assert_eq!(stored(&ctx, &user).await.token_version, 1);
    })
    .await;
}

#[tokio::test]
async fn a_password_change_session_may_log_out() {
    // ADR 0006 §4: logout is one of the three routes a `password_change`
    // token may reach, so a user who must set a new password can leave
    // instead. It ends the account's full sessions too.
    with_server(|server, ctx| async move {
        let user = account(&ctx, "jelszocsere@example.test", |new| {
            new.token_version = ActiveValue::Set(3);
        })
        .await;
        let password_change = scoped_token(&ctx, &user, SessionScope::PasswordChange);
        let full = admin_token_at(&ctx, &user, instant());

        assert_logged_out(
            &logout_with_cookie(&server, &password_change).await,
            "password_change",
        );
        assert_eq!(me_statuses(&server, &password_change).await, ENDED);
        assert_eq!(me_statuses(&server, &full).await, ENDED);
        assert_eq!(stored(&ctx, &user).await.token_version, 4);
    })
    .await;
}

#[tokio::test]
async fn logout_without_a_valid_session_answers_401_and_revokes_nothing() {
    // The route table's guard answers before the handler runs, with the one `401` of
    // every refused session (tests/requests/auth_session.rs has the full
    // list), and sets no cookie. No account's version moves.
    with_server(|server, ctx| async move {
        let live = account(&ctx, "elo@example.test", |_| {}).await;
        let inactive = account(&ctx, "inaktiv@example.test", |new| {
            new.active = ActiveValue::Set(false);
        })
        .await;
        let trashed = account(&ctx, "lomtar@example.test", |new| {
            new.deleted_at = ActiveValue::Set(Some(instant().fixed_offset()));
        })
        .await;
        // A session that already logged out: its second logout is refused.
        let logged_out = account(&ctx, "kilepett@example.test", |_| {}).await;
        let old = admin_token_at(&ctx, &logged_out, instant());
        assert_logged_out(&logout_with_cookie(&server, &old).await, "first logout");

        let cases = [
            ("no token", server.post(LOGOUT).await),
            (
                "not a JWT",
                logout_with_cookie(&server, "nem.jwt.token").await,
            ),
            ("the second logout", logout_with_cookie(&server, &old).await),
            (
                "an mfa token",
                logout_with_cookie(&server, &scoped_token(&ctx, &live, SessionScope::Mfa)).await,
            ),
            (
                "an mfa Bearer token",
                logout_with_bearer(&server, &scoped_token(&ctx, &live, SessionScope::Mfa)).await,
            ),
            (
                "an inactive account",
                logout_with_cookie(&server, &admin_token_at(&ctx, &inactive, instant())).await,
            ),
            (
                "a trashed account",
                logout_with_cookie(&server, &admin_token_at(&ctx, &trashed, instant())).await,
            ),
        ];
        for (case, response) in &cases {
            assert_eq!(response.status_code(), StatusCode::UNAUTHORIZED, "{case}");
            assert_eq!(response.json::<Value>(), unauthorized(), "{case}");
            assert_eq!(response.header("cache-control"), "no-store", "{case}");
            assert_eq!(set_cookies(response), NO_COOKIES, "{case}");
        }
        for (account, version) in [(&live, 0), (&inactive, 0), (&trashed, 0), (&logged_out, 1)] {
            assert_eq!(
                stored(&ctx, account).await.token_version,
                version,
                "{:?}",
                account.email
            );
        }
        // The live account's session was never touched.
        assert_eq!(
            me_statuses(&server, &admin_token_at(&ctx, &live, instant())).await,
            OPEN
        );
    })
    .await;
}

#[tokio::test]
async fn every_other_method_answers_405_and_revokes_nothing() {
    // POST only (ADR 0006 §2): a GET would let a link, an image or a
    // prefetch sign the user out. axum answers the other methods with 405
    // and `Allow: POST`; the session holds.
    with_server(|server, ctx| async move {
        let user = account(&ctx, "metodus@example.test", |_| {}).await;
        let token = admin_token_at(&ctx, &user, instant());
        for method in [Method::GET, Method::PUT, Method::PATCH, Method::DELETE] {
            let response = server
                .method(method.clone(), LOGOUT)
                .add_header(COOKIE, session_cookie_header(&token))
                .await;
            assert_eq!(
                response.status_code(),
                StatusCode::METHOD_NOT_ALLOWED,
                "{method}"
            );
            assert_eq!(response.header(ALLOW), "POST", "{method}");
            assert_eq!(response.header("cache-control"), "no-store", "{method}");
            assert_eq!(set_cookies(&response), NO_COOKIES, "{method}");
        }
        assert_eq!(stored(&ctx, &user).await.token_version, 0);
        assert_eq!(me_statuses(&server, &token).await, OPEN);
    })
    .await;
}

#[tokio::test]
async fn a_failed_revocation_answers_500_and_keeps_the_cookie() {
    // The admin UI signs out on 401 only (ADR 0006, follow-up work), so a
    // write that fails must answer 500, never 401 or 204: the sessions
    // still hold, and the browser keeps its cookie. A trigger that refuses
    // every UPDATE of `users` makes the write fail after the guard's read
    // succeeded; the per-test database is dropped afterwards.
    with_server(|server, ctx| async move {
        let user = account(&ctx, "hiba@example.test", |_| {}).await;
        let token = admin_token_at(&ctx, &user, instant());
        ctx.db
            .execute_unprepared(
                "CREATE FUNCTION refuse_update() RETURNS trigger LANGUAGE plpgsql AS \
                 $$ BEGIN RAISE EXCEPTION 'test: users is read-only'; END $$; \
                 CREATE TRIGGER refuse_update BEFORE UPDATE ON users \
                 FOR EACH ROW EXECUTE FUNCTION refuse_update();",
            )
            .await
            .expect("create the refusing trigger");

        let response = logout_with_cookie(&server, &token).await;
        assert_eq!(response.status_code(), StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(
            response.json::<Value>()["error"],
            json!("internal_server_error")
        );
        assert_eq!(response.header("cache-control"), "no-store");
        assert_eq!(set_cookies(&response), NO_COOKIES);
        assert_eq!(stored(&ctx, &user).await.token_version, 0);
        assert_eq!(me_statuses(&server, &token).await, OPEN);
    })
    .await;
}
