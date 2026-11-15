//! The activity entries of sign-in and logout through the HTTP endpoints
//! (ADR 0011 §4, §5; ADR 0006 §2, §4), with the whole middleware stack:
//!
//! - a sign-in writes `login` and sets `last_login_at`, both at the entry's
//!   instant;
//! - a failed sign-in of a known account writes `login_failed`, whatever
//!   made it fail, including an account deactivated while its password was
//!   being verified, and the answer stays the one `401`;
//! - a failed sign-in that names no known account, and a body refused
//!   before any lookup (`422` or `413`), write nothing (research: DECISIONS
//!   R9 (D09));
//! - a logout writes `logout`, and a refused logout writes nothing;
//! - each entry is about the account, carries the client address that
//!   `remote_ip` resolved (ADR 0023 §2: the rightmost `X-Forwarded-For`
//!   value, or the socket address) and the request id of the response's
//!   `x-request-id`, an id the server made even when the caller sent one
//!   (research: DECISIONS R23 (D119));
//! - none of them writes a revision or changes `updated_at`;
//! - a refused entry rolls its sign-in or logout back and answers `500`
//!   without a cookie, except the `login_failed` entry, whose refusal
//!   leaves the failure's `401` exactly the one an unknown address gets
//!   (ADR 0006 §4, ADR 0023 §3).
//!
//! Addresses come from the documentation ranges of RFC 5737 and RFC 3849;
//! every account and password is invented.

use std::{net::IpAddr, time::Duration};

use axum::http::{HeaderMap, StatusCode, header::COOKIE};
use axum_test::{TestResponse, TestServer};
use backend3::{
    domain::{
        activity::{Action, Actor},
        session_cookie::SESSION_COOKIE_NAME,
    },
    models::{
        activity_log,
        users::{self, ActiveModel, Model},
    },
};
use chrono::{DateTime, Utc};
use loco_rs::{
    app::AppContext,
    controller::middleware::limit_payload::DefaultBodyLimitKind,
    prelude::{ActiveModelTrait, ActiveValue, ConnectionTrait, Uuid},
};
use sea_orm::{
    ConnectOptions, Database, DatabaseConnection, DbBackend, EntityTrait, QueryOrder, Statement,
    TransactionTrait,
};
use serde_json::{Value, json};

use crate::support::with_server;

const LOGIN: &str = "/api/admin/auth/login";
const LOGOUT: &str = "/api/admin/auth/logout";
/// The password of every account below.
const PASSWORD: &str = "helyes jelszó 2026";
/// An `X-Forwarded-For` chain: a value the client wrote, then the one the
/// router appended, which is the client's address (ADR 0023 §2).
const FORWARDED_FOR: &str = "192.0.2.66, 203.0.113.9";
/// The client address that [`FORWARDED_FOR`] resolves to.
const CLIENT: &str = "203.0.113.9";
/// A request id of the caller's choice, which must never be stored.
const CALLER_REQUEST_ID: &str = "kovacs_janos@verseghy";

/// The instant the invented accounts record as `updated_at`.
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

/// What [`entries`] returns while the log is empty.
const NO_ENTRIES: [activity_log::Model; 0] = [];

/// Every activity entry, oldest first.
async fn entries(ctx: &AppContext) -> Vec<activity_log::Model> {
    activity_log::Entity::find()
        .order_by_asc(activity_log::Column::Id)
        .all(&ctx.db)
        .await
        .expect("read the activity log")
}

/// The number of revisions of every record.
async fn revision_count(ctx: &AppContext) -> i64 {
    ctx.db
        .query_one_raw(Statement::from_string(
            DbBackend::Postgres,
            "SELECT count(*) FROM revisions",
        ))
        .await
        .expect("count")
        .expect("one row")
        .try_get_by_index(0)
        .expect("a count")
}

/// `POST /auth/login` from behind the router ([`FORWARDED_FOR`]), with the
/// caller's own `x-request-id`.
async fn login(server: &TestServer, email: &str, password: &str) -> TestResponse {
    server
        .post(LOGIN)
        .add_header("x-forwarded-for", FORWARDED_FOR)
        .add_header("x-request-id", CALLER_REQUEST_ID)
        .json(&json!({ "email": email, "password": password }))
        .await
}

/// The token of the session cookie that `response` sets.
fn session_token(response: &TestResponse) -> String {
    response
        .header("set-cookie")
        .to_str()
        .expect("ASCII")
        .split_once(';')
        .and_then(|(pair, _)| pair.strip_prefix(&format!("{SESSION_COOKIE_NAME}=")))
        .expect("the session cookie")
        .to_owned()
}

/// `POST /auth/logout` with `token` in the session cookie, from behind the
/// router, with the caller's own `x-request-id`.
async fn logout(server: &TestServer, token: &str) -> TestResponse {
    server
        .post(LOGOUT)
        .add_header(COOKIE, format!("{SESSION_COOKIE_NAME}={token}"))
        .add_header("x-forwarded-for", FORWARDED_FOR)
        .add_header("x-request-id", CALLER_REQUEST_ID)
        .await
}

/// The response's request id, after checking that the server made it: a
/// UUID v4, never the caller's text.
fn server_request_id(response: &TestResponse) -> String {
    let id = response
        .header("x-request-id")
        .to_str()
        .expect("ASCII")
        .to_owned();
    assert_ne!(id, CALLER_REQUEST_ID);
    let uuid = Uuid::parse_str(&id).expect("a UUID");
    assert_eq!(uuid.get_version_num(), 4, "{id}");
    id
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

/// The request body limit that the configuration gives the server: no
/// committed configuration sets `limit_payload`, so Loco's default.
fn body_limit(ctx: &AppContext) -> usize {
    match ctx
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
    }
}

/// What [`set_cookies`] returns for a response that sets no cookie.
const NO_COOKIES: [String; 0] = [];

/// Every `Set-Cookie` value of `response`.
fn set_cookies(response: &TestResponse) -> Vec<String> {
    response
        .iter_headers_by_name("set-cookie")
        .map(|value| value.to_str().expect("ASCII").to_owned())
        .collect()
}

/// Asserts that `entry` is the account entry `action` about `user`, by
/// `actor`, written for `response`'s request from [`CLIENT`].
fn assert_entry(
    entry: &activity_log::Model,
    action: Action,
    actor: Actor,
    user: &Model,
    response: &TestResponse,
) {
    assert_eq!(entry.action(), Ok(action));
    assert_eq!(entry.actor(), Ok(actor), "{action}");
    assert_eq!(
        (entry.entity_type.as_deref(), entry.entity_id),
        (Some("users"), Some(user.id)),
        "{action}"
    );
    assert_eq!(
        (entry.revision_id, &entry.label, &entry.details),
        (None, &None, &None),
        "{action}"
    );
    assert_eq!(
        entry.client_address(),
        Some(CLIENT.parse::<IpAddr>().expect("an address")),
        "{action}"
    );
    assert_eq!(
        entry.request_id.as_deref(),
        Some(server_request_id(response).as_str()),
        "{action}"
    );
}

#[tokio::test]
async fn a_sign_in_sets_last_login_at_and_writes_login() {
    with_server(|server, ctx| async move {
        let user = account(&ctx, "belep@example.test", |_| {}).await;
        let other = account(&ctx, "marad@example.test", |_| {}).await;

        let response = login(&server, "belep@example.test", PASSWORD).await;
        assert_eq!(response.status_code(), StatusCode::OK);

        let [login_entry] = &entries(&ctx).await[..] else {
            panic!("one entry expected");
        };
        assert_entry(
            login_entry,
            Action::Login,
            Actor::User(user.id),
            &user,
            &response,
        );
        // `last_login_at` is the entry's instant, PostgreSQL's clock (D90),
        // and nothing else of the account changed: not `updated_at`, not
        // `updated_by`, and no revision was written (ADR 0011 §4).
        let after = stored(&ctx, &user).await;
        assert_eq!(after.last_login_at, Some(login_entry.occurred_at));
        assert_eq!(
            Model {
                last_login_at: None,
                ..after.clone()
            },
            user
        );
        assert_eq!(
            (after.updated_at, after.updated_by),
            (instant().fixed_offset(), None)
        );
        assert_eq!(revision_count(&ctx).await, 0);
        assert_eq!(stored(&ctx, &other).await, other);
    })
    .await;
}

#[tokio::test]
async fn the_address_is_the_socket_s_without_a_forwarded_header() {
    // ADR 0023 §2: a request that did not pass the router carries no
    // header, and is known by its socket address: the test client's
    // loopback connection.
    with_server(|server, ctx| async move {
        account(&ctx, "belso@example.test", |_| {}).await;
        let response = server
            .post(LOGIN)
            .json(&json!({ "email": "belso@example.test", "password": PASSWORD }))
            .await;
        assert_eq!(response.status_code(), StatusCode::OK);
        let [login_entry] = &entries(&ctx).await[..] else {
            panic!("one entry expected");
        };
        let address = login_entry.client_address().expect("an address");
        assert!(address.is_loopback(), "{address}");
        assert_eq!(
            login_entry.request_id.as_deref(),
            Some(server_request_id(&response).as_str())
        );
    })
    .await;
}

#[tokio::test]
async fn a_failed_sign_in_of_a_known_account_writes_login_failed() {
    with_server(|server, ctx| async move {
        let live = account(&ctx, "aktiv@example.test", |_| {}).await;
        let inactive = account(&ctx, "inaktiv@example.test", |new| {
            new.active = ActiveValue::Set(false);
        })
        .await;
        let pending = account(&ctx, "fuggo@example.test", |new| {
            new.password_hash = ActiveValue::Set(None);
        })
        .await;
        // Until the two-factor step and the forced change exist, the right
        // password of these accounts fails too (research: DECISIONS R15
        // (D74)), and the failure is recorded like any other.
        let two_factor = account(&ctx, "ketlepcsos@example.test", |new| {
            new.totp_enabled_at = ActiveValue::Set(Some(instant().fixed_offset()));
        })
        .await;
        let must_change = account(&ctx, "csere@example.test", |new| {
            new.must_change_password = ActiveValue::Set(true);
        })
        .await;

        let mut responses = Vec::new();
        for (case, email, password, user) in [
            (
                "wrong password",
                "AKTIV@example.test",
                "rossz jelszó",
                &live,
            ),
            ("inactive", "inaktiv@example.test", PASSWORD, &inactive),
            ("no password yet", "fuggo@example.test", PASSWORD, &pending),
            (
                "two-factor on",
                "ketlepcsos@example.test",
                PASSWORD,
                &two_factor,
            ),
            ("must change", "csere@example.test", PASSWORD, &must_change),
        ] {
            let response = login(&server, email, password).await;
            assert_eq!(response.status_code(), StatusCode::UNAUTHORIZED, "{case}");
            assert_eq!(
                response.json::<Value>()["error"],
                json!("invalid_credentials"),
                "{case}"
            );
            assert_eq!(set_cookies(&response), NO_COOKIES, "{case}");
            responses.push((case, user, response));
        }

        let stored_entries = entries(&ctx).await;
        assert_eq!(stored_entries.len(), responses.len());
        for (entry, (case, user, response)) in stored_entries.iter().zip(&responses) {
            // The caller has not signed in, so the system writes the entry,
            // about the account it named.
            assert_entry(entry, Action::LoginFailed, Actor::System, user, response);
            assert_eq!(entry.actor_id, None, "{case}");
            // The account is untouched: no `last_login_at`, no `updated_at`.
            assert_eq!(stored(&ctx, user).await, **user, "{case}");
        }
        assert_eq!(revision_count(&ctx).await, 0);
    })
    .await;
}

#[tokio::test]
async fn failures_that_name_no_known_account_write_nothing() {
    // ADR 0011 §5 and research: DECISIONS R9 (D09): `login_failed` is for
    // known accounts only, so no typed address is ever stored. A trashed
    // account is not one the sign-in finds (ADR 0006 §4 loads the live
    // account), and a body refused before the lookup names none, whether it
    // is malformed (`422`) or above the body limit (`413`), even when it
    // carries a known address and a wrong password.
    with_server(|server, ctx| async move {
        account(&ctx, "aktiv@example.test", |_| {}).await;
        let trashed = account(&ctx, "lomtar@example.test", |new| {
            new.deleted_at = ActiveValue::Set(Some(instant().fixed_offset()));
        })
        .await;

        for (case, email) in [
            ("unknown e-mail", "nincs@example.test"),
            ("an address no account can hold", "aktiv\u{0}@example.test"),
            ("trashed", "lomtar@example.test"),
        ] {
            let response = login(&server, email, PASSWORD).await;
            assert_eq!(response.status_code(), StatusCode::UNAUTHORIZED, "{case}");
        }
        let refused_body = server
            .post(LOGIN)
            .add_header("x-forwarded-for", FORWARDED_FOR)
            .json(&json!({ "email": "aktiv@example.test", "password": 5 }))
            .await;
        assert_eq!(refused_body.status_code(), StatusCode::UNPROCESSABLE_ENTITY);
        // A known address and a wrong password, padded with spaces before
        // the closing brace (valid JSON) to one byte above the limit.
        let head = r#"{"email":"aktiv@example.test","password":"rossz jelszó""#;
        let padding = body_limit(&ctx) + 1 - head.len() - 1;
        let too_large = server
            .post(LOGIN)
            .add_header("x-forwarded-for", FORWARDED_FOR)
            .bytes(format!("{head}{}}}", " ".repeat(padding)).into())
            .content_type("application/json")
            .await;
        assert_eq!(too_large.status_code(), StatusCode::PAYLOAD_TOO_LARGE);
        assert_eq!(too_large.json::<Value>()["error"], json!("body_too_large"));

        assert_eq!(entries(&ctx).await, NO_ENTRIES);
        assert_eq!(stored(&ctx, &trashed).await, trashed);
    })
    .await;
}

#[tokio::test]
async fn an_account_deactivated_during_its_sign_in_gets_no_session() {
    // ADR 0006 §1, §2: an `edit users` holder deactivates the account while
    // its right password is being verified. The deactivation holds the
    // account's row until the sign-in's `UPDATE` waits for it, then commits;
    // the `UPDATE` then finds an inactive account at a newer token version
    // and matches nothing. The sign-in fails like any other: the one `401`,
    // no cookie, no `last_login_at`, no `login`, and a `login_failed` entry.
    with_server(|server, ctx| async move {
        let user = account(&ctx, "kozben@example.test", |_| {}).await;
        let unknown = login(&server, "nincs@example.test", PASSWORD).await;

        // A pool of its own: the app's two connections serve the sign-in
        // and the poll below.
        let mut options = ConnectOptions::new(ctx.config.database.uri.clone());
        options
            .max_connections(1)
            .min_connections(1)
            .sqlx_logging(false);
        let editor_pool = Database::connect(options)
            .await
            .expect("open a second pool");
        let editor = editor_pool.begin().await.expect("begin the deactivation");
        editor
            .execute_unprepared(&format!(
                "SELECT id FROM users WHERE id = {} FOR UPDATE",
                user.id
            ))
            .await
            .expect("hold the account's row");
        let deactivate = async {
            wait_for_the_sign_in_s_update(&ctx.db).await;
            editor
                .execute_unprepared(&format!(
                    "UPDATE users SET active = false, token_version = token_version + 1 \
                     WHERE id = {}",
                    user.id
                ))
                .await
                .expect("deactivate");
            editor.commit().await.expect("commit the deactivation");
        };
        let (response, ()) =
            tokio::join!(login(&server, "kozben@example.test", PASSWORD), deactivate);
        editor_pool.close().await.expect("close the second pool");

        assert_eq!(observable(&response), observable(&unknown));
        assert_eq!(set_cookies(&response), NO_COOKIES);
        let after = stored(&ctx, &user).await;
        assert_eq!(
            (after.active, after.last_login_at),
            (false, None),
            "deactivated, never signed in"
        );
        let [failed] = &entries(&ctx).await[..] else {
            panic!("one entry expected");
        };
        assert_entry(failed, Action::LoginFailed, Actor::System, &user, &response);
    })
    .await;
}

/// Polls `pg_stat_activity` until a session of the test's database waits
/// for a lock in an `UPDATE` of `users`, the sign-in's write; panics after
/// about 30 seconds, a debug build's Argon2id verification many times over.
///
/// - `db`: the app's pool. Each poll is its own statement outside any
///   transaction, because a transaction would keep its first reading of
///   the view.
async fn wait_for_the_sign_in_s_update(db: &DatabaseConnection) {
    for _ in 0..3000 {
        let waiting: i64 = db
            .query_one_raw(Statement::from_string(
                DbBackend::Postgres,
                "SELECT count(*) FROM pg_stat_activity WHERE datname = current_database() \
                 AND wait_event_type = 'Lock' AND query LIKE 'UPDATE \"users\"%'",
            ))
            .await
            .expect("read pg_stat_activity")
            .expect("one row")
            .try_get_by_index(0)
            .expect("a count");
        if waiting == 1 {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("the sign-in's UPDATE never waited for the account's row");
}

#[tokio::test]
async fn a_logout_writes_logout() {
    with_server(|server, ctx| async move {
        let user = account(&ctx, "kilep@example.test", |_| {}).await;
        let token = session_token(&login(&server, "kilep@example.test", PASSWORD).await);
        let signed_in = stored(&ctx, &user).await;

        let response = logout(&server, &token).await;
        assert_eq!(response.status_code(), StatusCode::NO_CONTENT);

        let all = entries(&ctx).await;
        let [login_entry, logout_entry] = &all[..] else {
            panic!("two entries expected: {all:?}");
        };
        assert_eq!(login_entry.action(), Ok(Action::Login));
        assert_entry(
            logout_entry,
            Action::Logout,
            Actor::User(user.id),
            &user,
            &response,
        );
        // Each request has its own id.
        assert_ne!(login_entry.request_id, logout_entry.request_id);
        // An account event: the version alone moved; `updated_at` and
        // `last_login_at` stayed, and no revision was written.
        let after = stored(&ctx, &user).await;
        assert_eq!(
            Model {
                token_version: signed_in.token_version,
                ..after.clone()
            },
            signed_in
        );
        assert_eq!(after.token_version, signed_in.token_version + 1);
        assert_eq!(after.updated_at, instant().fixed_offset());
        assert_eq!(revision_count(&ctx).await, 0);
    })
    .await;
}

#[tokio::test]
async fn a_refused_logout_writes_nothing() {
    // Without a valid session the route table's guard answers 401 before the
    // handler runs, and a method other than POST answers 405: no account
    // event happened, so no entry is written.
    with_server(|server, ctx| async move {
        let user = account(&ctx, "nem.lep.ki@example.test", |_| {}).await;
        let token = user
            .generate_session_token(
                &ctx.config.get_jwt_config().expect("auth.jwt").secret,
                7200,
                instant(),
            )
            .expect("signed");

        let no_session = server.post(LOGOUT).await;
        assert_eq!(no_session.status_code(), StatusCode::UNAUTHORIZED);
        let not_a_token = logout(&server, "nem.jwt.token").await;
        assert_eq!(not_a_token.status_code(), StatusCode::UNAUTHORIZED);
        let get = server
            .get(LOGOUT)
            .add_header(COOKIE, format!("{SESSION_COOKIE_NAME}={token}"))
            .await;
        assert_eq!(get.status_code(), StatusCode::METHOD_NOT_ALLOWED);

        assert_eq!(entries(&ctx).await, NO_ENTRIES);
        assert_eq!(stored(&ctx, &user).await, user);
    })
    .await;
}

/// Makes every INSERT into `activity_log` fail, so the next entry is
/// refused after the change it records; the per-test database is dropped
/// afterwards.
async fn refuse_activity_entries(ctx: &AppContext) {
    ctx.db
        .execute_unprepared(
            "CREATE FUNCTION refuse_entry() RETURNS trigger LANGUAGE plpgsql AS \
             $$ BEGIN RAISE EXCEPTION 'test: activity_log refuses entries'; END $$; \
             CREATE TRIGGER refuse_entry BEFORE INSERT ON activity_log \
             FOR EACH ROW EXECUTE FUNCTION refuse_entry();",
        )
        .await
        .expect("create the refusing trigger");
}

#[tokio::test]
async fn a_refused_entry_rolls_the_sign_in_back() {
    // ADR 0011 §5: a failed insert of the entry rolls the change back. The
    // sign-in answers 500 without the cookie, and `last_login_at` stays.
    with_server(|server, ctx| async move {
        let user = account(&ctx, "nincs.naplo@example.test", |_| {}).await;
        refuse_activity_entries(&ctx).await;

        let response = login(&server, "nincs.naplo@example.test", PASSWORD).await;
        assert_eq!(response.status_code(), StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(
            response.json::<Value>()["error"],
            json!("internal_server_error")
        );
        assert_eq!(set_cookies(&response), NO_COOKIES);
        assert_eq!(stored(&ctx, &user).await, user);
        assert_eq!(entries(&ctx).await, NO_ENTRIES);
    })
    .await;
}

#[tokio::test]
async fn a_refused_login_failed_entry_leaves_the_one_401() {
    // ADR 0006 §4 ("Same answer for every failure") and ADR 0023 §3: when
    // the database refuses writes but still answers reads (a read-only
    // database, a full disk), a known account's failure must still look
    // exactly like an unknown address's, or the answers would tell which
    // addresses belong to an account. The lost entry is logged instead.
    with_server(|server, ctx| async move {
        let user = account(&ctx, "nincs.naplo@example.test", |_| {}).await;
        let inactive = account(&ctx, "inaktiv.naplo@example.test", |new| {
            new.active = ActiveValue::Set(false);
        })
        .await;
        refuse_activity_entries(&ctx).await;

        let unknown = login(&server, "nincs.ilyen@example.test", "rossz jelszó").await;
        assert_eq!(unknown.status_code(), StatusCode::UNAUTHORIZED);
        assert_eq!(
            unknown.json::<Value>()["error"],
            json!("invalid_credentials")
        );
        for (case, email, password) in [
            ("wrong password", "nincs.naplo@example.test", "rossz jelszó"),
            ("inactive", "inaktiv.naplo@example.test", PASSWORD),
        ] {
            let known = login(&server, email, password).await;
            assert_eq!(observable(&known), observable(&unknown), "{case}");
        }
        assert_eq!(set_cookies(&unknown), NO_COOKIES);
        assert_eq!(entries(&ctx).await, NO_ENTRIES);
        assert_eq!(stored(&ctx, &user).await, user);
        assert_eq!(stored(&ctx, &inactive).await, inactive);
    })
    .await;
}

#[tokio::test]
async fn a_refused_entry_rolls_the_logout_back() {
    // The revocation and its entry share one transaction: when the entry
    // fails, the sessions hold and the browser keeps its cookie.
    with_server(|server, ctx| async move {
        let user = account(&ctx, "bent.marad@example.test", |_| {}).await;
        let token = session_token(&login(&server, "bent.marad@example.test", PASSWORD).await);
        let signed_in = stored(&ctx, &user).await;
        refuse_activity_entries(&ctx).await;

        let response = logout(&server, &token).await;
        assert_eq!(response.status_code(), StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(set_cookies(&response), NO_COOKIES);
        assert_eq!(stored(&ctx, &user).await, signed_in);
        let me = server
            .get("/api/admin/auth/me")
            .add_header(COOKIE, format!("{SESSION_COOKIE_NAME}={token}"))
            .await;
        assert_eq!(me.status_code(), StatusCode::OK);
        // Only the sign-in's entry, from before the trigger.
        let all = entries(&ctx).await;
        assert_eq!(
            all.iter()
                .map(|entry| entry.action.as_str())
                .collect::<Vec<_>>(),
            ["login"]
        );
    })
    .await;
}
