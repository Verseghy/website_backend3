//! `POST /api/admin/auth/login`, the browser sign-in (ADR 0006 §1, §4), and
//! `Cache-Control: no-store` on every admin API response (ADR 0023 §6).
//!
//! A correct password answers `{"step":"done"}` with the session cookie,
//! whose token Loco's `JWTWithUser` extractor accepts from the cookie and
//! from a Bearer header. Every credential failure answers byte for byte the
//! same `401`, including, for now, the right password of an account with
//! two-factor on or a forced password change. A body that is not
//! `{email, password}` answers byte for byte the same `422`, and one above
//! the request body limit `413` (research: DECISIONS R15 (D74)). Every account
//! and password here is invented.

use axum::{
    extract::FromRequestParts,
    http::{
        HeaderMap, Method, Request, StatusCode,
        header::{AUTHORIZATION, COOKIE},
    },
};
use axum_test::{TestResponse, TestServer};
use backend3::{
    app::App,
    domain::{api_path::ADMIN_API_PREFIX, session_cookie::SESSION_COOKIE_NAME},
    models::users::{self, ActiveModel, Model},
};
use chrono::{DateTime, Utc};
use loco_rs::{
    app::{AppContext, Hooks},
    auth::jwt::JWT,
    controller::middleware::limit_payload::DefaultBodyLimitKind,
    prelude::{ActiveModelTrait, ActiveValue, auth::JWTWithUser},
};
use serde_json::{Value, json};

use crate::support::{with_server, with_server_at};

const LOGIN: &str = "/api/admin/auth/login";
/// The password of every account below.
const PASSWORD: &str = "helyes jelszó 2026";

/// Inserts a live, active account with `email` and the Argon2id hash of
/// [`PASSWORD`], after `change` has adjusted it.
async fn account(ctx: &AppContext, email: &str, change: impl FnOnce(&mut ActiveModel)) -> Model {
    let mut new = ActiveModel {
        email: ActiveValue::Set(Some(email.to_owned())),
        name: ActiveValue::Set("Próba Szerkesztő".to_owned()),
        password_hash: ActiveValue::Set(Some(users::hash_password(PASSWORD).expect("hash"))),
        ..Default::default()
    };
    change(&mut new);
    new.insert(&ctx.db).await.expect("insert the account")
}

/// The app clock's "now" of the sign-in test, and the instant that the
/// trashed and two-factor accounts record.
fn instant() -> DateTime<Utc> {
    DateTime::parse_from_rfc3339("2026-10-25T08:00:00Z")
        .expect("RFC 3339")
        .with_timezone(&Utc)
}

async fn login(server: &TestServer, email: &str, password: &str) -> TestResponse {
    server
        .post(LOGIN)
        .json(&json!({ "email": email, "password": password }))
        .await
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

#[tokio::test]
async fn a_correct_password_sets_the_session_cookie_and_answers_done() {
    with_server_at(instant(), |server, ctx, _clock| async move {
        let user = account(&ctx, "szerkeszto@example.test", |_| {}).await;
        // Wall-clock readings around the request, only for Loco's `exp`
        // (recipe 7 step 4 of the project skill).
        let before = Utc::now().timestamp();
        // The address is matched without regard to case.
        let response = login(&server, "Szerkeszto@EXAMPLE.test", PASSWORD).await;
        let after = Utc::now().timestamp();

        assert_eq!(response.status_code(), StatusCode::OK);
        assert_eq!(response.text(), r#"{"step":"done"}"#);
        assert_eq!(response.header("cache-control"), "no-store");
        let cookies: Vec<&str> = response
            .iter_headers_by_name("set-cookie")
            .map(|value| value.to_str().expect("ASCII"))
            .collect();
        let [cookie] = cookies[..] else {
            panic!("one Set-Cookie expected: {cookies:?}");
        };
        let (name_value, attributes) = cookie.split_once(';').expect("attributes");
        let token = name_value
            .strip_prefix(&format!("{SESSION_COOKIE_NAME}="))
            .expect("the session cookie");
        insta::assert_snapshot!(
            format!("{SESSION_COOKIE_NAME}=[token];{attributes}"),
            @"__Host-admin_session=[token]; HttpOnly; Secure; SameSite=Strict; Path=/; Max-Age=7200"
        );

        // The claims of ADR 0006 §1, signed with the configured secret.
        let secret = ctx
            .config
            .get_jwt_config()
            .expect("auth.jwt")
            .secret
            .clone();
        let claims = JWT::new(&secret)
            .validate(token)
            .expect("a valid token")
            .claims;
        let claims = serde_json::to_value(&claims).expect("serializes");
        assert_eq!(claims["pid"], json!(user.pid.to_string()));
        assert_eq!(
            (&claims["tv"], &claims["scope"]),
            (&json!(0), &json!("admin"))
        );
        // `auth_time` is the app clock's "now"; `exp` is the wall clock plus
        // the two-hour lifetime.
        assert_eq!(claims["auth_time"], json!(instant().timestamp()));
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

        // Loco's extractor accepts the token from the cookie and as Bearer,
        // and loads the account through `Authenticable`.
        for (header, value) in [
            (COOKIE, format!("theme=dark; {SESSION_COOKIE_NAME}={token}")),
            (AUTHORIZATION, format!("Bearer {token}")),
        ] {
            let request = Request::get("/api/admin/x").header(header, value).body(());
            let (mut parts, ()) = request.expect("request").into_parts();
            let auth = JWTWithUser::<Model>::from_request_parts(&mut parts, &ctx)
                .await
                .expect("the token is accepted");
            assert_eq!(auth.user.id, user.id);
        }
        // Under another cookie name it is not looked at.
        let request = Request::get("/api/admin/x").header(COOKIE, format!("admin_session={token}"));
        let (mut parts, ()) = request.body(()).expect("request").into_parts();
        assert!(
            JWTWithUser::<Model>::from_request_parts(&mut parts, &ctx)
                .await
                .is_err()
        );
    })
    .await;
}

#[tokio::test]
async fn every_failed_sign_in_gets_the_same_401() {
    with_server(|server, ctx| async move {
        account(&ctx, "aktiv@example.test", |_| {}).await;
        account(&ctx, "inaktiv@example.test", |new| {
            new.active = ActiveValue::Set(false);
        })
        .await;
        account(&ctx, "lomtar@example.test", |new| {
            new.deleted_at = ActiveValue::Set(Some(instant().fixed_offset()));
        })
        .await;
        account(&ctx, "fuggo@example.test", |new| {
            new.password_hash = ActiveValue::Set(None);
        })
        .await;
        // Until the two-factor step and the forced change exist, these never
        // get a full session from the password alone, and get the same 401
        // as a wrong password (research: DECISIONS R15 (D74)).
        account(&ctx, "ketlepcsos@example.test", |new| {
            new.totp_enabled_at = ActiveValue::Set(Some(instant().fixed_offset()));
        })
        .await;
        account(&ctx, "csere@example.test", |new| {
            new.must_change_password = ActiveValue::Set(true);
        })
        .await;

        let responses = [
            (
                "wrong password",
                login(&server, "aktiv@example.test", "rossz jelszó 2026").await,
            ),
            (
                "unknown e-mail",
                login(&server, "nincs@example.test", PASSWORD).await,
            ),
            (
                "an address no account can hold",
                login(&server, "aktiv\u{0}@example.test", PASSWORD).await,
            ),
            (
                "inactive",
                login(&server, "inaktiv@example.test", PASSWORD).await,
            ),
            (
                "trashed",
                login(&server, "lomtar@example.test", PASSWORD).await,
            ),
            (
                "no password yet",
                login(&server, "fuggo@example.test", PASSWORD).await,
            ),
            (
                "two-factor on",
                login(&server, "ketlepcsos@example.test", PASSWORD).await,
            ),
            (
                "must change",
                login(&server, "csere@example.test", PASSWORD).await,
            ),
        ];
        let (_, first) = &responses[0];
        assert_eq!(first.status_code(), StatusCode::UNAUTHORIZED);
        assert_eq!(
            first.json::<Value>(),
            json!({
                "error": "invalid_credentials",
                "description": "The e-mail address or the password is wrong",
                "field": null,
                "params": {}
            })
        );
        assert_eq!(first.header("cache-control"), "no-store");
        assert!(first.maybe_header("set-cookie").is_none());
        for (case, response) in &responses {
            assert_eq!(observable(response), observable(first), "{case}");
        }
        // The correct password still works: nothing above locked the account.
        assert_eq!(
            login(&server, "aktiv@example.test", PASSWORD)
                .await
                .status_code(),
            StatusCode::OK
        );
    })
    .await;
}

/// The `422 invalid_body` envelope (research: DECISIONS R15 (D74)).
fn invalid_body() -> Value {
    json!({
        "error": "invalid_body",
        "description": "The request body is not the JSON this endpoint takes",
        "field": null,
        "params": {}
    })
}

#[tokio::test]
async fn a_body_that_is_not_email_and_password_gets_422_before_any_lookup() {
    with_server(|server, ctx| async move {
        account(&ctx, "aktiv@example.test", |_| {}).await;

        let mut responses = Vec::new();
        for (case, body) in [
            ("no password", json!({ "email": "aktiv@example.test" })),
            ("no e-mail", json!({ "password": PASSWORD })),
            // The right password does not help: the body is not the DTO.
            (
                "extra field",
                json!({ "email": "aktiv@example.test", "password": PASSWORD, "x": 1 }),
            ),
            (
                "password not a string",
                json!({ "email": "aktiv@example.test", "password": 5 }),
            ),
            // The same shape error for an address no account has: the answer
            // is the same, because nothing is looked up.
            (
                "password not a string, unknown e-mail",
                json!({ "email": "nincs@example.test", "password": 5 }),
            ),
            (
                "e-mail null",
                json!({ "email": null, "password": PASSWORD }),
            ),
            ("an array", json!(["aktiv@example.test", PASSWORD])),
        ] {
            responses.push((case, server.post(LOGIN).json(&body).await));
        }
        let credentials = json!({ "email": "aktiv@example.test", "password": PASSWORD });
        for (case, content_type, body) in [
            (
                "not JSON",
                "application/json",
                "aktiv@example.test".to_owned(),
            ),
            (
                "cut-off JSON",
                "application/json",
                r#"{"email":"#.to_owned(),
            ),
            ("empty", "application/json", String::new()),
            // A repeated member is refused, whichever copy holds the right
            // credentials: no parser that keeps the first or the last copy
            // can read another account from the body than this one does.
            (
                "repeated members, the right ones last",
                "application/json",
                format!(
                    r#"{{"email":"nincs@example.test","password":"x","email":"aktiv@example.test","password":"{PASSWORD}"}}"#
                ),
            ),
            (
                "repeated members, the right ones first",
                "application/json",
                format!(
                    r#"{{"email":"aktiv@example.test","password":"{PASSWORD}","email":"nincs@example.test","password":"x"}}"#
                ),
            ),
            ("text/plain", "text/plain", credentials.to_string()),
            (
                "form-encoded",
                "application/x-www-form-urlencoded",
                "email=aktiv%40example.test".to_owned(),
            ),
        ] {
            let request = server
                .post(LOGIN)
                .bytes(body.into())
                .content_type(content_type);
            responses.push((case, request.await));
        }

        let (_, first) = &responses[0];
        assert_eq!(first.status_code(), StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(first.json::<Value>(), invalid_body());
        assert_eq!(first.header("cache-control"), "no-store");
        assert!(first.maybe_header("set-cookie").is_none());
        for (case, response) in &responses {
            assert_eq!(observable(response), observable(first), "{case}");
        }
        // Nothing above signed in or locked the account.
        assert_eq!(
            login(&server, "aktiv@example.test", PASSWORD)
                .await
                .status_code(),
            StatusCode::OK
        );
    })
    .await;
}

/// A JSON body of exactly `len` bytes with the right credentials: the object
/// padded with spaces before its closing brace, which JSON allows.
fn credentials_of_length(len: usize) -> String {
    let head = format!(r#"{{"email":"aktiv@example.test","password":"{PASSWORD}""#);
    let padding = len
        .checked_sub(head.len() + 1)
        .expect("longer than the credentials");
    format!("{head}{}}}", " ".repeat(padding))
}

#[tokio::test]
async fn a_body_above_the_size_limit_gets_413() {
    with_server(|server, ctx| async move {
        account(&ctx, "aktiv@example.test", |_| {}).await;
        // The limit the configuration gives the server; no config sets
        // `limit_payload`, so it is Loco's default of 2,000,000 bytes.
        let limit = match ctx
            .config
            .server
            .middlewares
            .limit_payload
            .clone()
            .unwrap_or_default()
            .body_limit
        {
            DefaultBodyLimitKind::Limit(limit) => limit,
            DefaultBodyLimitKind::Disable => panic!("the body limit is on"),
        };
        let send = |len: usize| {
            server
                .post(LOGIN)
                .bytes(credentials_of_length(len).into())
                .content_type("application/json")
        };

        // A body at the limit is read, and its right password signs in.
        let at_limit = send(limit).await;
        assert_eq!(at_limit.status_code(), StatusCode::OK);
        assert_eq!(at_limit.text(), r#"{"step":"done"}"#);

        // One byte more is refused before it is parsed.
        let above = send(limit + 1).await;
        assert_eq!(above.status_code(), StatusCode::PAYLOAD_TOO_LARGE);
        assert_eq!(
            above.json::<Value>(),
            json!({
                "error": "body_too_large",
                "description": "The request body is larger than the limit",
                "field": null,
                "params": {}
            })
        );
        assert_eq!(above.header("cache-control"), "no-store");
        assert!(above.maybe_header("set-cookie").is_none());

        // axum checks the content type before it reads the body.
        let other_type = server
            .post(LOGIN)
            .bytes(credentials_of_length(limit + 1).into())
            .content_type("text/plain")
            .await;
        assert_eq!(other_type.status_code(), StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(other_type.json::<Value>(), invalid_body());
    })
    .await;
}

#[tokio::test]
async fn every_admin_api_response_is_no_store() {
    with_server(|server, ctx| async move {
        // Every mounted admin route with each of its methods, so a route that
        // a later slice adds outside the layer fails here.
        let mut requests: Vec<(Method, String)> = App::routes(&ctx)
            .collect()
            .into_iter()
            .filter(|route| route.uri.starts_with(&format!("{ADMIN_API_PREFIX}/")))
            .flat_map(|route| {
                let uri = route.uri.replace("{id}", "1");
                route
                    .actions
                    .into_iter()
                    .map(move |method| (method, uri.clone()))
            })
            .collect();
        assert!(
            requests.contains(&(Method::POST, LOGIN.to_owned())),
            "{requests:?}"
        );
        // A method the path does not take (405), and an unknown path (404).
        requests.push((Method::GET, LOGIN.to_owned()));
        requests.push((Method::POST, format!("{ADMIN_API_PREFIX}/auth/nope")));
        for (method, uri) in requests {
            let response = server.method(method.clone(), &uri).await;
            assert_eq!(
                response.header("cache-control"),
                "no-store",
                "{method} {uri}"
            );
        }
        let response = server.get(LOGIN).await;
        assert_eq!(response.status_code(), StatusCode::METHOD_NOT_ALLOWED);
    })
    .await;
}
