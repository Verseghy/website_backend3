//! A legacy bcrypt password through both password sign-ins, the browser's
//! `/api/admin/auth/login` and the scripts' `/api/admin/auth/token`
//! (ADR 0006 §4, §6; ADR 0011 §4; parity AUTH-01, whose fix this is).
//!
//! The importer copies each Laravel `$2y$10$` hash unchanged (ADR 0017,
//! data-migration.md §6.12). A sign-in with its password succeeds, stores an
//! Argon2id hash of the password in its place, and changes nothing else of
//! the account but `last_login_at`: no revision, the same `updated_at`, the
//! `login` entry only. The next sign-in verifies the Argon2id hash. A
//! failed sign-in, and a legacy password shorter than 12 characters, which
//! the committed configurations send to the password change, answer the one
//! `401` of every failed sign-in and keep the bcrypt hash.
//!
//! The sign-ins read that rule from the `SignInRules` in the shared store:
//! with `force_change_short_legacy_password` off, the short password signs
//! in and is rehashed, and a context without the rules answers `500` and
//! writes nothing.
//!
//! Every hash is generated here from an invented password (ADR 0006,
//! follow-up work); none is a credential.

use axum::http::{
    HeaderMap, StatusCode,
    header::{COOKIE, SET_COOKIE},
};
use axum_test::{TestResponse, TestServer};
use backend3::{
    controllers::admin::auth::{SignInRules, sign_in_rules},
    domain::session_cookie::SESSION_COOKIE_NAME,
    models::{
        activity_log,
        users::{ActiveModel, Model, PasswordCheck, check_stored_password},
    },
    settings::Settings,
};
use bcrypt::Version;
use chrono::{DateTime, Utc};
use loco_rs::{
    app::AppContext,
    prelude::{ActiveModelTrait, ActiveValue, ConnectionTrait},
};
use sea_orm::{DbBackend, EntityTrait, QueryOrder, Statement};
use serde_json::{Value, json};

use crate::support::{with_app, with_server};

const LOGIN: &str = "/api/admin/auth/login";
const TOKEN: &str = "/api/admin/auth/token";
const ME: &str = "/api/admin/auth/me";
/// The invented password of the legacy accounts: 18 characters.
const PASSWORD: &str = "helyes jelszó 2026";

/// The `updated_at` every account starts with.
fn instant() -> DateTime<Utc> {
    DateTime::parse_from_rfc3339("2026-10-25T08:00:00Z")
        .expect("RFC 3339")
        .with_timezone(&Utc)
}

/// The bcrypt hash of `password` at `cost`, written with the prefix of
/// `version`.
fn legacy_hash(password: &str, cost: u32, version: Version) -> String {
    bcrypt::hash_with_result(password, cost)
        .expect("bcrypt hash")
        .format_for_version(version)
}

/// Inserts a live, active, imported-looking account with `email`, the
/// stored hash `hash` and `updated_at` at [`instant`].
async fn legacy_account(ctx: &AppContext, email: &str, hash: &str) -> Model {
    ActiveModel {
        email: ActiveValue::Set(Some(email.to_owned())),
        name: ActiveValue::Set("Régi Szerkesztő".to_owned()),
        password_hash: ActiveValue::Set(Some(hash.to_owned())),
        updated_at: ActiveValue::Set(instant().fixed_offset()),
        ..Default::default()
    }
    .insert(&ctx.db)
    .await
    .expect("insert the account")
}

/// The stored row of `user`.
async fn stored(ctx: &AppContext, user: &Model) -> Model {
    Model::find_by_pid(&ctx.db, &user.pid.to_string())
        .await
        .expect("the stored account")
}

/// The actions of every activity entry, oldest first.
async fn actions(ctx: &AppContext) -> Vec<String> {
    activity_log::Entity::find()
        .order_by_asc(activity_log::Column::Id)
        .all(&ctx.db)
        .await
        .expect("read the activity log")
        .into_iter()
        .map(|entry| entry.action)
        .collect()
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

/// `POST path` with `{email, password}`.
async fn sign_in(server: &TestServer, path: &str, email: &str, password: &str) -> TestResponse {
    server
        .post(path)
        .json(&json!({ "email": email, "password": password }))
        .await
}

/// Everything a client can see of a response, except the `x-request-id`
/// and the `date`, which differ on every response.
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

/// The token of the session cookie that `response` sets.
fn cookie_token(response: &TestResponse) -> String {
    response
        .header("set-cookie")
        .to_str()
        .expect("ASCII")
        .split_once(';')
        .and_then(|(pair, _)| pair.strip_prefix(&format!("{SESSION_COOKIE_NAME}=")))
        .expect("the session cookie")
        .to_owned()
}

/// Asserts that `user`'s stored hash is now an Argon2id hash of `password`,
/// and that the rest of the account changed as a sign-in changes it and no
/// more (ADR 0011 §4); returns the stored hash.
async fn assert_rehashed(ctx: &AppContext, user: &Model, password: &str, case: &str) -> String {
    let after = stored(ctx, user).await;
    let hash = after.password_hash.clone().expect("a hash");
    assert!(
        hash.starts_with("$argon2id$v=19$m=19456,t=2,p=1$"),
        "{case}: {hash}"
    );
    assert_eq!(
        check_stored_password(Some(&hash), password),
        PasswordCheck::Matches,
        "{case}"
    );
    assert!(after.last_login_at.is_some(), "{case}");
    assert_eq!(after.updated_at, user.updated_at, "{case}");
    assert_eq!(after.updated_by, user.updated_by, "{case}");
    assert_eq!(after.token_version, user.token_version, "{case}");
    assert_eq!(after.name, user.name, "{case}");
    hash
}

#[tokio::test]
async fn a_generated_2y_10_hash_signs_in_and_the_next_sign_in_uses_argon2id() {
    with_server(|server, ctx| async move {
        // The shape of every imported hash: Laravel's `$2y$`, cost 10.
        let legacy = legacy_hash(PASSWORD, 10, Version::TwoY);
        assert!(
            legacy.starts_with("$2y$10$") && legacy.len() == 60,
            "{legacy}"
        );
        let user = legacy_account(&ctx, "regi@example.test", &legacy).await;

        let first = sign_in(&server, LOGIN, "regi@example.test", PASSWORD).await;
        assert_eq!(first.status_code(), StatusCode::OK);
        assert_eq!(first.text(), r#"{"step":"done"}"#);
        // A full session, like any other sign-in's.
        let me = server
            .get(ME)
            .add_header(
                COOKIE,
                format!("{SESSION_COOKIE_NAME}={}", cookie_token(&first)),
            )
            .await;
        assert_eq!(me.status_code(), StatusCode::OK);
        assert_eq!(me.json::<Value>()["scope"], json!("admin"));

        // Replaced in the sign-in's own write: an account event, so no
        // revision and `login` as the only entry (ADR 0011 §4, §5).
        let rehashed = assert_rehashed(&ctx, &user, PASSWORD, "first sign-in").await;
        assert_eq!(revision_count(&ctx).await, 0);
        assert_eq!(actions(&ctx).await, ["login"]);

        // The next sign-in verifies the Argon2id hash: it succeeds and
        // stores no new hash.
        let second = sign_in(&server, LOGIN, "regi@example.test", PASSWORD).await;
        assert_eq!(second.status_code(), StatusCode::OK);
        assert_eq!(
            stored(&ctx, &user).await.password_hash.as_deref(),
            Some(rehashed.as_str())
        );
        assert_eq!(actions(&ctx).await, ["login", "login"]);
        assert_eq!(revision_count(&ctx).await, 0);
    })
    .await;
}

#[tokio::test]
async fn every_legacy_spelling_is_rehashed_by_either_sign_in() {
    with_server(|server, ctx| async move {
        for (case, version, path, email) in [
            (
                "$2a$, browser",
                Version::TwoA,
                LOGIN,
                "a-bongeszo@example.test",
            ),
            (
                "$2b$, browser",
                Version::TwoB,
                LOGIN,
                "b-bongeszo@example.test",
            ),
            (
                "$2y$, script",
                Version::TwoY,
                TOKEN,
                "y-szkript@example.test",
            ),
            (
                "$2a$, script",
                Version::TwoA,
                TOKEN,
                "a-szkript@example.test",
            ),
            (
                "$2b$, script",
                Version::TwoB,
                TOKEN,
                "b-szkript@example.test",
            ),
        ] {
            let user = legacy_account(&ctx, email, &legacy_hash(PASSWORD, 4, version)).await;
            let response = sign_in(&server, path, email, PASSWORD).await;
            assert_eq!(response.status_code(), StatusCode::OK, "{case}");
            assert_rehashed(&ctx, &user, PASSWORD, case).await;
        }
        assert_eq!(revision_count(&ctx).await, 0);
    })
    .await;
}

#[tokio::test]
async fn a_failed_legacy_sign_in_is_the_one_401_and_keeps_the_hash() {
    with_server(|server, ctx| async move {
        let legacy = legacy_hash(PASSWORD, 4, Version::TwoY);
        let user = legacy_account(&ctx, "regi@example.test", &legacy).await;
        // Eleven characters: ADR 0006 §6 sends such a legacy password to
        // the password change, which does not exist yet, so the sign-in
        // refuses it like a wrong password (research: DECISIONS R15 (D74)).
        let short_password = "rovid jelsz";
        let short_legacy = legacy_hash(short_password, 4, Version::TwoY);
        let short = legacy_account(&ctx, "rovid@example.test", &short_legacy).await;

        let unknown = sign_in(&server, LOGIN, "nincs@example.test", PASSWORD).await;
        assert_eq!(unknown.status_code(), StatusCode::UNAUTHORIZED);
        for path in [LOGIN, TOKEN] {
            for (case, email, password) in [
                ("wrong password", "regi@example.test", "rossz jelszó 2026"),
                (
                    "short legacy password",
                    "rovid@example.test",
                    short_password,
                ),
            ] {
                let response = sign_in(&server, path, email, password).await;
                assert_eq!(
                    observable(&response),
                    observable(&unknown),
                    "{path}: {case}"
                );
            }
        }
        // Both keep their bcrypt hash, so the right long password still
        // signs in once and is rehashed then, and the short one stays
        // under the rule.
        assert_eq!(
            stored(&ctx, &user).await.password_hash.as_deref(),
            Some(legacy.as_str())
        );
        assert_eq!(
            stored(&ctx, &short).await.password_hash.as_deref(),
            Some(short_legacy.as_str())
        );
        // A failed sign-in of a known account writes `login_failed`
        // (ADR 0011 §5), the short password's included.
        assert_eq!(actions(&ctx).await, ["login_failed"; 4]);
        assert_eq!(
            sign_in(&server, TOKEN, "regi@example.test", PASSWORD)
                .await
                .status_code(),
            StatusCode::OK
        );
        assert_rehashed(&ctx, &user, PASSWORD, "after the failures").await;
    })
    .await;
}

#[tokio::test]
async fn the_sign_in_rules_come_from_the_settings() {
    // `App::after_context` stores the rule of every committed configuration
    // (ADR 0006 §8: `force_change_short_legacy_password: true`).
    with_app(|_router, ctx| async move {
        let settings = Settings::from_section(ctx.config.settings.as_ref()).expect("settings");
        assert!(settings.admin_auth.force_change_short_legacy_password);
        assert_eq!(
            sign_in_rules(&ctx).expect("stored"),
            SignInRules::new(&settings)
        );
        assert_eq!(
            SignInRules::new(&settings),
            SignInRules {
                force_change_short_legacy_password: true
            }
        );
    })
    .await;
}

#[tokio::test]
async fn the_sign_ins_read_the_short_legacy_password_rule_from_the_shared_store() {
    // Both sign-ins take the rule from the `SignInRules` that the shared
    // store holds, never from a fixed value: with
    // `force_change_short_legacy_password` off, a short legacy password
    // signs in at either sign-in and is rehashed like any other legacy
    // password (ADR 0006 §6, §8). The test server's router holds a clone of
    // `ctx`, whose shared store is the same `Arc`, so the value inserted
    // here is the one the handlers read.
    with_server(|server, ctx| async move {
        // Eleven characters, under the rule's twelve.
        let short_password = "rovid jelsz";
        let browser = legacy_account(
            &ctx,
            "rovid-bongeszo@example.test",
            &legacy_hash(short_password, 4, Version::TwoY),
        )
        .await;
        let script = legacy_account(
            &ctx,
            "rovid-szkript@example.test",
            &legacy_hash(short_password, 4, Version::TwoY),
        )
        .await;
        let cases = [
            (LOGIN, "rovid-bongeszo@example.test", &browser),
            (TOKEN, "rovid-szkript@example.test", &script),
        ];

        // On, as in every committed configuration: both are refused, and
        // keep their bcrypt hash.
        for (path, email, user) in cases {
            let refused = sign_in(&server, path, email, short_password).await;
            assert_eq!(refused.status_code(), StatusCode::UNAUTHORIZED, "{path}");
            assert_eq!(stored(&ctx, user).await, *user, "{path}");
        }

        ctx.shared_store.insert(SignInRules {
            force_change_short_legacy_password: false,
        });
        for (path, email, user) in cases {
            let accepted = sign_in(&server, path, email, short_password).await;
            assert_eq!(accepted.status_code(), StatusCode::OK, "{path}");
            if path == TOKEN {
                assert!(accepted.json::<Value>()["token"].is_string(), "{path}");
            } else {
                assert_eq!(accepted.text(), r#"{"step":"done"}"#, "{path}");
            }
            assert_rehashed(&ctx, user, short_password, path).await;
        }
        assert_eq!(
            actions(&ctx).await,
            ["login_failed", "login_failed", "login", "login"]
        );
        assert_eq!(revision_count(&ctx).await, 0);
    })
    .await;
}

#[tokio::test]
async fn a_context_without_the_sign_in_rules_lets_no_sign_in_through() {
    // Every context that Loco builds has them (`App::after_context`). One
    // without them answers `500` to either sign-in before it reads the
    // account, rather than guessing the rule: no cookie, no token, no
    // `last_login_at`, no rehash and no activity entry.
    with_server(|server, ctx| async move {
        let legacy = legacy_hash(PASSWORD, 4, Version::TwoY);
        let user = legacy_account(&ctx, "regi@example.test", &legacy).await;
        assert!(ctx.shared_store.remove::<SignInRules>().is_some());
        assert!(sign_in_rules(&ctx).is_err());

        for path in [LOGIN, TOKEN] {
            let response = sign_in(&server, path, "regi@example.test", PASSWORD).await;
            assert_eq!(
                response.status_code(),
                StatusCode::INTERNAL_SERVER_ERROR,
                "{path}"
            );
            let body = response.json::<Value>();
            assert_eq!(body["error"], json!("internal_server_error"), "{path}");
            assert!(body.get("token").is_none(), "{path}: {body}");
            assert!(response.maybe_header(SET_COOKIE).is_none(), "{path}");
        }
        // The account is exactly as it was: the bcrypt hash and no
        // `last_login_at` included.
        assert_eq!(stored(&ctx, &user).await, user);
        assert_eq!(user.password_hash.as_deref(), Some(legacy.as_str()));
        assert_eq!(user.last_login_at, None);
        assert_eq!(actions(&ctx).await, Vec::<String>::new());
    })
    .await;
}
