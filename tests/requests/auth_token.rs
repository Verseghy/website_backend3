//! `POST /api/admin/auth/token`, the scripts' sign-in (ADR 0006 §3; parity
//! AUTH-01), through the whole middleware stack.
//!
//! - A correct e-mail address and password answer `200 {token, expires_at}`
//!   and set no cookie: a full `admin` session token for the
//!   `Authorization: Bearer` header, with the browser session's claims and
//!   lifetime, whose `expires_at` is its own `exp`. The sign-in is recorded
//!   like the browser's (`last_login_at`, the `login` entry).
//! - Every refusal is the browser sign-in's, byte for byte: the one `401`
//!   for every credential failure, an account with two-factor on or a
//!   forced password change included, so the endpoint never issues a
//!   `password_change` or `mfa` token; the `422` and the `413`.
//! - A Bearer request is exempt from the `Origin` check only without a
//!   credential cookie (ADR 0044 §2, reading 1 that the maintainer accepted
//!   on 2026-11-15, research: DECISIONS R35 (D151)): a Bearer write without
//!   `Origin` passes, and one that also carries the session or the
//!   two-factor cookie is refused.
//! - A Bearer token is never refreshed: neither the passive refresh nor
//!   `POST /auth/refresh`, which answers it `204` without a cookie (research:
//!   DECISIONS R35 (D154)), even with less than an hour left, when the same
//!   token in the cookie would be renewed.
//!
//! Loco signs `exp` from the wall clock (recipe 7 step 4 of the project
//! skill), so `exp` is checked only between wall-clock readings around the
//! request, and the refresh test places the app clock relative to a token
//! the endpoint issued. Every account and password is invented.

use axum::http::{
    HeaderMap, StatusCode,
    header::{ALLOW, COOKIE, ORIGIN},
};
use axum_test::{TestResponse, TestServer};
use backend3::{
    controllers::admin::JSON_BODY_LIMIT,
    domain::session_cookie::{MFA_COOKIE_NAME, SESSION_COOKIE_NAME},
    models::{
        activity_log,
        users::{self, ActiveModel, Model},
    },
};
use chrono::{DateTime, Utc};
use loco_rs::{
    app::AppContext,
    auth::jwt::JWT,
    prelude::{ActiveModelTrait, ActiveValue},
};
use sea_orm::{EntityTrait, QueryOrder};
use serde_json::{Value, json};

use crate::support::{
    ADMIN_ORIGIN,
    session_refresh::{at, cookie_header, jwt_secret, renewed_cookie, set_cookies, token_times},
    with_server, with_server_at,
};

const LOGIN: &str = "/api/admin/auth/login";
const TOKEN: &str = "/api/admin/auth/token";
const ME: &str = "/api/admin/auth/me";
const REFRESH: &str = "/api/admin/auth/refresh";
const LOGOUT: &str = "/api/admin/auth/logout";
/// The password of every account below.
const PASSWORD: &str = "helyes jelszó 2026";

/// The app clock's "now" when the tests begin: 2026-11-02T09:00:00Z.
fn start() -> DateTime<Utc> {
    at(1_793_610_000)
}

/// Inserts a live, active account with `email` and the Argon2id hash of
/// [`PASSWORD`], after `change` has adjusted it.
async fn account(ctx: &AppContext, email: &str, change: impl FnOnce(&mut ActiveModel)) -> Model {
    let mut new = ActiveModel {
        email: ActiveValue::Set(Some(email.to_owned())),
        name: ActiveValue::Set("Próba Szkript".to_owned()),
        password_hash: ActiveValue::Set(Some(users::hash_password(PASSWORD).expect("hash"))),
        ..Default::default()
    };
    change(&mut new);
    new.insert(&ctx.db).await.expect("insert the account")
}

/// `POST /auth/token` with `{email, password}`, as a script sends it: no
/// cookie and no `Origin`.
async fn token_request(server: &TestServer, email: &str, password: &str) -> TestResponse {
    server
        .post(TOKEN)
        .json(&json!({ "email": email, "password": password }))
        .await
}

/// The token of a successful `POST /auth/token`.
async fn issued_token(server: &TestServer, email: &str) -> String {
    let response = token_request(server, email, PASSWORD).await;
    assert_eq!(response.status_code(), StatusCode::OK, "{email}");
    response.json::<Value>()["token"]
        .as_str()
        .expect("a token")
        .to_owned()
}

/// Everything a client can see of a response, except the two headers that
/// differ on every response: the `x-request-id` and the `date`, which has a
/// resolution of one second.
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

/// Every claim of `token` as one JSON object, validated with the test
/// secret.
fn claims_of(ctx: &AppContext, token: &str) -> Value {
    let claims = JWT::new(&jwt_secret(ctx))
        .validate(token)
        .expect("a valid token")
        .claims;
    serde_json::to_value(claims).expect("serializes")
}

/// The `403 origin_mismatch` envelope (ADR 0044 §5).
fn origin_mismatch() -> Value {
    json!({
        "error": "origin_mismatch",
        "description": "The request does not come from the admin's origin; a write with the \
                        session cookie must carry it in the Origin header",
        "field": null,
        "params": {},
    })
}

#[tokio::test]
async fn a_correct_password_answers_a_bearer_token_and_sets_no_cookie() {
    with_server_at(start(), |server, ctx, _clock| async move {
        let user = account(&ctx, "szkript@example.test", |_| {}).await;
        // Wall-clock readings around the request, only for Loco's `exp`
        // (recipe 7 step 4 of the project skill).
        let before = Utc::now().timestamp();
        // The address is matched without regard to case.
        let response = token_request(&server, "Szkript@EXAMPLE.test", PASSWORD).await;
        let after = Utc::now().timestamp();

        assert_eq!(response.status_code(), StatusCode::OK);
        assert_eq!(response.header("content-type"), "application/json");
        // The body holds a credential: no browser or proxy may keep it.
        assert_eq!(response.header("cache-control"), "no-store");
        assert_eq!(set_cookies(&response), Vec::<String>::new());
        let body = response.json::<Value>();
        let mut members: Vec<&str> = body
            .as_object()
            .expect("an object")
            .keys()
            .map(String::as_str)
            .collect();
        members.sort_unstable();
        assert_eq!(members, ["expires_at", "token"], "{body}");
        let token = body["token"].as_str().expect("a token");

        // The claims of the browser session (ADR 0006 §1, §3): a full
        // `admin` token, never `password_change` or `mfa`, signed in at the
        // app clock's "now", for two hours by the wall clock.
        let claims = claims_of(&ctx, token);
        assert_eq!(claims["pid"], json!(user.pid.to_string()));
        assert_eq!(
            (&claims["tv"], &claims["scope"]),
            (&json!(0), &json!("admin"))
        );
        assert_eq!(claims["auth_time"], json!(start().timestamp()));
        let exp = claims["exp"].as_i64().expect("exp");
        assert!(
            (before + 7200..=after + 7200).contains(&exp),
            "{before} + 7200 <= {exp} <= {after} + 7200"
        );
        assert_eq!(
            claims.as_object().map(serde_json::Map::len),
            Some(5),
            "{claims}"
        );
        // `expires_at` is that `exp`, an RFC 3339 instant in UTC.
        assert_eq!(body["expires_at"], json!(at(exp)));
        let expires_at = body["expires_at"].as_str().expect("a string");
        assert!(expires_at.ends_with('Z'), "{expires_at}");
        assert_eq!(
            DateTime::parse_from_rfc3339(expires_at)
                .expect("RFC 3339")
                .timestamp(),
            exp
        );

        // The script sends it as a Bearer token: a full session, which the
        // guard does not refresh.
        let me = server.get(ME).authorization_bearer(token).await;
        assert_eq!(me.status_code(), StatusCode::OK);
        assert_eq!(me.json::<Value>()["scope"], json!("admin"));
        assert_eq!(set_cookies(&me), Vec::<String>::new());

        // A sign-in like the browser's: `last_login_at` and one `login`
        // entry about the account (ADR 0006 §4, §7; ADR 0011 §5).
        let stored = Model::find_by_pid(&ctx.db, &user.pid.to_string())
            .await
            .expect("the stored account");
        assert!(stored.last_login_at.is_some());
        let entries = activity_log::Entity::find()
            .order_by_asc(activity_log::Column::Id)
            .all(&ctx.db)
            .await
            .expect("read the activity log");
        let [entry] = &entries[..] else {
            panic!("one entry expected: {entries:?}");
        };
        assert_eq!(entry.action, "login");
        assert_eq!(
            (entry.entity_type.as_deref(), entry.entity_id),
            (Some("users"), Some(user.id))
        );

        // POST only: the token is never in a URL or a link.
        let get = server.get(TOKEN).await;
        assert_eq!(get.status_code(), StatusCode::METHOD_NOT_ALLOWED);
        assert_eq!(get.header(ALLOW), "POST");
        assert_eq!(get.header("cache-control"), "no-store");
    })
    .await;
}

#[tokio::test]
async fn every_credential_failure_is_the_browser_sign_in_s_401_byte_for_byte() {
    with_server(|server, ctx| async move {
        account(&ctx, "aktiv@example.test", |_| {}).await;
        account(&ctx, "inaktiv@example.test", |new| {
            new.active = ActiveValue::Set(false);
        })
        .await;
        account(&ctx, "lomtar@example.test", |new| {
            new.deleted_at = ActiveValue::Set(Some(start().fixed_offset()));
        })
        .await;
        account(&ctx, "fuggo@example.test", |new| {
            new.password_hash = ActiveValue::Set(None);
        })
        .await;
        // The token is never a `password_change` or `mfa` token (ADR 0006
        // §3): until the token's TOTP code and the 403
        // `password_change_required` exist, these get the one 401, as at the
        // browser sign-in (research: DECISIONS R15 (D74)).
        account(&ctx, "ketlepcsos@example.test", |new| {
            new.totp_enabled_at = ActiveValue::Set(Some(start().fixed_offset()));
        })
        .await;
        account(&ctx, "csere@example.test", |new| {
            new.must_change_password = ActiveValue::Set(true);
        })
        .await;

        let credentials = [
            ("wrong password", "aktiv@example.test", "rossz jelszó 2026"),
            ("unknown e-mail", "nincs@example.test", PASSWORD),
            (
                "an address no account can hold",
                "aktiv\u{0}@example.test",
                PASSWORD,
            ),
            ("inactive", "inaktiv@example.test", PASSWORD),
            ("trashed", "lomtar@example.test", PASSWORD),
            ("no password yet", "fuggo@example.test", PASSWORD),
            ("two-factor on", "ketlepcsos@example.test", PASSWORD),
            ("must change", "csere@example.test", PASSWORD),
        ];
        let mut answers = Vec::new();
        for (case, email, password) in credentials {
            let body = json!({ "email": email, "password": password });
            let login = server.post(LOGIN).json(&body).await;
            let token = server.post(TOKEN).json(&body).await;
            assert_eq!(observable(&token), observable(&login), "{case}");
            assert!(token.maybe_header("set-cookie").is_none(), "{case}");
            answers.push((case, observable(&token)));
        }
        // One answer for every case, so it tells nothing about the account.
        let (_, first) = answers.first().cloned().expect("eight cases");
        for (case, answer) in &answers {
            assert_eq!(answer, &first, "{case}");
        }
        let (status, headers, body) = first;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
        assert_eq!(headers.get("cache-control").expect("no-store"), "no-store");
        assert_eq!(
            serde_json::from_slice::<Value>(&body).expect("JSON"),
            json!({
                "error": "invalid_credentials",
                "description": "The e-mail address or the password is wrong",
                "field": null,
                "params": {}
            })
        );

        // Nothing above locked the account: the right password still gets a
        // token.
        assert_eq!(
            token_request(&server, "aktiv@example.test", PASSWORD)
                .await
                .status_code(),
            StatusCode::OK
        );
    })
    .await;
}

#[tokio::test]
async fn a_body_that_is_not_email_and_password_is_refused_as_at_the_browser_sign_in() {
    with_server(|server, ctx| async move {
        account(&ctx, "aktiv@example.test", |_| {}).await;
        // The right credentials as the two form types, which a page on
        // another origin can send without a CORS preflight, as it can send
        // `text/plain` (below). The token endpoint, like the browser
        // sign-in, takes no credential cookie and so has no `Origin` check;
        // reading JSON alone is half of why no other page can sign in
        // through it (ADR 0044 §2; research: DECISIONS R35 (D152)).
        let form_encoded = url::form_urlencoded::Serializer::new(String::new())
            .append_pair("email", "aktiv@example.test")
            .append_pair("password", PASSWORD)
            .finish();
        let boundary = "szkript-hatar";
        let multipart_type = format!("multipart/form-data; boundary={boundary}");
        let multipart = format!(
            "--{boundary}\r\nContent-Disposition: form-data; name=\"email\"\r\n\r\n\
             aktiv@example.test\r\n--{boundary}\r\nContent-Disposition: form-data; \
             name=\"password\"\r\n\r\n{PASSWORD}\r\n--{boundary}--\r\n"
        );
        // A body that is not `{email, password}`: the same `422` from both,
        // before any lookup; the right password does not help. A TOTP
        // `code` is not a member yet (it comes with the two-factor step).
        for (case, content_type, body) in [
            (
                "extra member",
                "application/json",
                json!({ "email": "aktiv@example.test", "password": PASSWORD, "code": "123456" })
                    .to_string(),
            ),
            (
                "an array",
                "application/json",
                json!(["aktiv@example.test", PASSWORD]).to_string(),
            ),
            (
                "not JSON",
                "application/json",
                "aktiv@example.test".to_owned(),
            ),
            ("empty", "application/json", String::new()),
            (
                "repeated members, the right ones last",
                "application/json",
                format!(
                    r#"{{"email":"nincs@example.test","password":"x","email":"aktiv@example.test","password":"{PASSWORD}"}}"#
                ),
            ),
            (
                "text/plain",
                "text/plain",
                json!({ "email": "aktiv@example.test", "password": PASSWORD }).to_string(),
            ),
            (
                "form-encoded",
                "application/x-www-form-urlencoded",
                form_encoded,
            ),
            ("multipart", multipart_type.as_str(), multipart),
        ] {
            let send = |path: &'static str| {
                server
                    .post(path)
                    .bytes(body.clone().into())
                    .content_type(content_type)
            };
            let login = send(LOGIN).await;
            let token = send(TOKEN).await;
            assert_eq!(
                token.status_code(),
                StatusCode::UNPROCESSABLE_ENTITY,
                "{case}"
            );
            assert_eq!(observable(&token), observable(&login), "{case}");
        }

        // A body above the admin API's 4 MiB JSON limit: the same `413`.
        let oversized = format!(
            r#"{{"email":"aktiv@example.test","password":"{PASSWORD}"{}}}"#,
            " ".repeat(JSON_BODY_LIMIT)
        );
        let send = |path: &'static str| {
            server
                .post(path)
                .bytes(oversized.clone().into())
                .content_type("application/json")
        };
        let login = send(LOGIN).await;
        let token = send(TOKEN).await;
        assert_eq!(token.status_code(), StatusCode::PAYLOAD_TOO_LARGE);
        assert_eq!(observable(&token), observable(&login));

        // Nothing above locked the account: the right password still gets a
        // token.
        assert_eq!(
            token_request(&server, "aktiv@example.test", PASSWORD)
                .await
                .status_code(),
            StatusCode::OK
        );
    })
    .await;
}

#[tokio::test]
async fn a_bearer_write_needs_no_origin_unless_it_carries_a_credential_cookie() {
    // ADR 0044 §2, reading 1 (D151): a request is checked when it is a
    // write and carries a credential cookie, whatever else it carries.
    with_server_at(start(), |server, ctx, _clock| async move {
        let user = account(&ctx, "szkript@example.test", |_| {}).await;
        let token = issued_token(&server, "szkript@example.test").await;

        // A script's write: the Bearer header alone, no `Origin`. The
        // explicit refresh renews nothing for it (D154).
        let refresh = server.post(REFRESH).authorization_bearer(&token).await;
        assert_eq!(refresh.status_code(), StatusCode::NO_CONTENT);
        assert!(refresh.as_bytes().is_empty());
        assert_eq!(set_cookies(&refresh), Vec::<String>::new());
        // Without a credential cookie the `Origin` is not read at all: one
        // of a sibling host changes nothing, for the token's own sign-in or
        // for a write with the token.
        let sibling = "https://verseghy-gimnazium.net";
        let signed_in = server
            .post(TOKEN)
            .add_header(ORIGIN, sibling)
            .json(&json!({ "email": "szkript@example.test", "password": PASSWORD }))
            .await;
        assert_eq!(signed_in.status_code(), StatusCode::OK);
        let from_sibling = server
            .post(REFRESH)
            .authorization_bearer(&token)
            .add_header(ORIGIN, sibling)
            .await;
        assert_eq!(from_sibling.status_code(), StatusCode::NO_CONTENT);

        // The same write with a credential cookie beside the Bearer header,
        // and no `Origin`: Loco would read the token from the cookie, so it
        // is a cookie write, and refused before anything is read.
        for cookie in [
            cookie_header(&token),
            format!("{SESSION_COOKIE_NAME}={token}"),
            format!("{MFA_COOKIE_NAME}={token}"),
        ] {
            let response = server
                .post(REFRESH)
                .authorization_bearer(&token)
                .add_header(COOKIE, cookie.as_str())
                .await;
            assert_eq!(response.status_code(), StatusCode::FORBIDDEN, "{cookie}");
            assert_eq!(response.json::<Value>(), origin_mismatch(), "{cookie}");
            assert_eq!(set_cookies(&response), Vec::<String>::new(), "{cookie}");
        }
        // With the admin origin the cookie write passes, and the cookie
        // session is renewed as any browser session is.
        let with_origin = server
            .post(REFRESH)
            .authorization_bearer(&token)
            .add_header(COOKIE, cookie_header(&token))
            .add_header(ORIGIN, ADMIN_ORIGIN)
            .await;
        assert_eq!(with_origin.status_code(), StatusCode::NO_CONTENT);
        assert!(renewed_cookie(&with_origin).is_some());

        // A script's logout needs no `Origin` either, and ends every session
        // of the account, its own token included (ADR 0006 §2).
        let logout = server.post(LOGOUT).authorization_bearer(&token).await;
        assert_eq!(logout.status_code(), StatusCode::NO_CONTENT);
        let stored = Model::find_by_pid(&ctx.db, &user.pid.to_string())
            .await
            .expect("the stored account");
        assert_eq!(stored.token_version, user.token_version + 1);
        assert_eq!(
            server
                .get(ME)
                .authorization_bearer(&token)
                .await
                .status_code(),
            StatusCode::UNAUTHORIZED
        );
    })
    .await;
}

#[tokio::test]
async fn a_bearer_token_from_the_endpoint_is_never_refreshed() {
    // ADR 0006 §3: the token "is not refreshed, so scripts sign in again".
    // The app clock is placed so that the token has ten minutes left, when
    // the passive refresh would renew it in the session cookie.
    with_server_at(start(), |server, ctx, clock| async move {
        account(&ctx, "szkript@example.test", |_| {}).await;
        // A first token tells where the wall clock is (its `exp` less the
        // lifetime); the clock moves there, so the second token's
        // `auth_time` and `exp` are both on the app clock's timeline, and
        // the absolute limit is far away.
        let probe = issued_token(&server, "szkript@example.test").await;
        clock.set(at(token_times(&ctx, &probe).expires_at - 7200));
        let token = issued_token(&server, "szkript@example.test").await;
        let times = token_times(&ctx, &token);
        clock.set(at(times.expires_at - 600));

        // As a Bearer token: no refresh, passive or explicit (D154).
        let me = server.get(ME).authorization_bearer(&token).await;
        assert_eq!(me.status_code(), StatusCode::OK);
        assert_eq!(set_cookies(&me), Vec::<String>::new(), "passive, Bearer");
        let refresh = server.post(REFRESH).authorization_bearer(&token).await;
        assert_eq!(refresh.status_code(), StatusCode::NO_CONTENT);
        assert_eq!(
            set_cookies(&refresh),
            Vec::<String>::new(),
            "explicit, Bearer"
        );

        // The same token in the session cookie is renewed at this instant,
        // so the Bearer answers above are the rule, not the clock.
        let in_cookie = server
            .get(ME)
            .add_header(COOKIE, cookie_header(&token))
            .await;
        assert_eq!(in_cookie.status_code(), StatusCode::OK);
        assert!(renewed_cookie(&in_cookie).is_some(), "passive, cookie");
    })
    .await;
}
