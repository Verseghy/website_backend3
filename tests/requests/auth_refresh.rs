//! The sliding session refresh (ADR 0006 §1, §8; parity AUTH-01, whose
//! "refreshed on activity" it builds, and AUTH-02, whose "Remember me" it
//! replaces).
//!
//! `POST /api/admin/auth/refresh` always renews a full session in the
//! session cookie to two hours, and the route table's guard renews one with
//! less than 3,600 s left on any admin request; neither ever sets an expiry
//! past `auth_time` plus `admin_auth.session_absolute_max_secs` (12 hours in
//! `config/test.yaml`). A Bearer token and a `password_change` session are
//! never renewed, and an answer carries at most one session cookie.
//!
//! Loco's JWT reads the system time (recipe 7 step 4 of the project skill),
//! so these tests never age a token through Loco. They place the app clock
//! relative to real tokens instead (`support::session_refresh`), assert
//! exactly what the server derives from the app clock (the cookie's
//! `Max-Age`, the kept `auth_time`), and check the renewed token's `exp`
//! only against that lifetime added to wall-clock readings taken just
//! before and after the request. Every account and password here is
//! invented.

use axum::{
    Json,
    http::{StatusCode, header::ALLOW},
};
use axum_test::{TestResponse, TestServer};
use backend3::{
    controllers::admin::{
        routes::{AdminRoute, build},
        session::AdminSession,
    },
    domain::{
        authz::{Requirement, Role},
        session::SessionScope,
        session_cookie::SESSION_COOKIE_NAME,
    },
    models::users::{self, ActiveModel, Model},
};
use chrono::{DateTime, TimeDelta, Utc};
use loco_rs::{
    app::AppContext,
    auth::jwt::JWT,
    prelude::{ActiveModelTrait, ActiveValue},
};
use serde_json::{Map, Value, json};

use crate::support::{
    TestClock,
    admin_probe::{account_with, probe_server},
    session_refresh::{
        SetSessionCookie, at, cookie_header, jwt_secret, place_session, renewed_cookie,
        set_cookies, token_times,
    },
    with_server_at,
};

const LOGIN: &str = "/api/admin/auth/login";
const ME: &str = "/api/admin/auth/me";
const REFRESH: &str = "/api/admin/auth/refresh";
const LOGOUT: &str = "/api/admin/auth/logout";
/// The password of the accounts that sign in through the endpoint.
const PASSWORD: &str = "helyes jelszó 2026";
/// `admin_auth.session_absolute_max_secs` of config/test.yaml.
const LIMIT_SECS: i64 = 43_200;

/// Where the app clock starts: 2026-11-02T09:00:00Z.
fn start() -> DateTime<Utc> {
    at(1_793_610_000)
}

/// `minutes` minutes.
fn minutes(minutes: i64) -> TimeDelta {
    TimeDelta::minutes(minutes)
}

/// Inserts a live, active account with `email` and the Argon2id hash of
/// [`PASSWORD`].
async fn account(ctx: &AppContext, email: &str) -> Model {
    ActiveModel {
        email: ActiveValue::Set(Some(email.to_owned())),
        name: ActiveValue::Set("Próba Szerkesztő".to_owned()),
        password_hash: ActiveValue::Set(Some(users::hash_password(PASSWORD).expect("hash"))),
        ..Default::default()
    }
    .insert(&ctx.db)
    .await
    .expect("insert the account")
}

/// Signs in through the endpoint and returns the session cookie's token.
async fn sign_in(server: &TestServer, email: &str) -> String {
    let response = server
        .post(LOGIN)
        .json(&json!({ "email": email, "password": PASSWORD }))
        .await;
    assert_eq!(response.status_code(), StatusCode::OK);
    renewed_cookie(&response).expect("the sign-in cookie").token
}

/// `POST /auth/refresh` with `token` in the session cookie.
async fn refresh_with_cookie(server: &TestServer, token: &str) -> TestResponse {
    server
        .post(REFRESH)
        .add_header("cookie", cookie_header(token))
        .await
}

/// `GET /auth/me` with `token` in the session cookie.
async fn me_with_cookie(server: &TestServer, token: &str) -> TestResponse {
    server
        .get(ME)
        .add_header("cookie", cookie_header(token))
        .await
}

/// The wall clock in whole seconds. Only around a request whose token's
/// `exp` the test checks (recipe 7 step 4 of the project skill).
fn wall_secs() -> i64 {
    Utc::now().timestamp()
}

/// Asserts that `response` is the explicit refresh's answer, `204` with no
/// body and no caching, and returns the session cookie it sets, if any.
fn refresh_answer(response: &TestResponse, case: &str) -> Option<SetSessionCookie> {
    assert_eq!(response.status_code(), StatusCode::NO_CONTENT, "{case}");
    assert!(response.as_bytes().is_empty(), "{case}");
    assert_eq!(response.header("cache-control"), "no-store", "{case}");
    renewed_cookie(response)
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

/// Asserts that `cookie` renews `user`'s session: a full `admin` token at
/// the account's version, with `auth_time` kept, whose `exp` is its
/// `Max-Age` after the request (between the wall-clock readings `before`
/// and `after`).
fn assert_renews(
    ctx: &AppContext,
    cookie: &SetSessionCookie,
    user: &Model,
    auth_time: i64,
    (before, after): (i64, i64),
    case: &str,
) {
    let claims = claims_of(ctx, &cookie.token);
    assert_eq!(claims["pid"], user.pid.to_string(), "{case}");
    assert_eq!(claims["tv"], user.token_version, "{case}");
    assert_eq!(claims["scope"], "admin", "{case}");
    assert_eq!(claims["auth_time"], auth_time, "{case}: auth_time is kept");
    let lifetime = i64::try_from(cookie.max_age).expect("small");
    let exp = token_times(ctx, &cookie.token).expires_at;
    assert!(
        (before + lifetime..=after + lifetime).contains(&exp),
        "{case}: {before} + {lifetime} <= {exp} <= {after} + {lifetime}"
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
async fn the_explicit_refresh_sets_a_new_two_hour_cookie_whatever_is_left() {
    with_server_at(start(), |server, ctx, clock| async move {
        let user = account(&ctx, "frissito@example.test").await;
        let token = sign_in(&server, "frissito@example.test").await;
        // Half an hour after the sign-in.
        clock.set(start() + minutes(30));

        let before = wall_secs();
        let response = refresh_with_cookie(&server, &token).await;
        let after = wall_secs();
        let cookie = refresh_answer(&response, "30 min after sign-in").expect("a new cookie");
        assert_eq!(cookie.max_age, 7200);
        // ADR 0006, follow-up work: the attributes are pinned, beside the
        // sign-in's (auth_login.rs) and the logout's (auth_logout.rs).
        insta::assert_snapshot!(
            cookie.header.replace(&cookie.token, "<token>"),
            @"__Host-admin_session=<token>; HttpOnly; Secure; SameSite=Strict; Path=/; Max-Age=7200"
        );
        assert_renews(
            &ctx,
            &cookie,
            &user,
            start().timestamp(),
            (before, after),
            "30 min",
        );
        assert_eq!(
            me_with_cookie(&server, &cookie.token).await.status_code(),
            StatusCode::OK
        );

        // With the full two hours left, or ten minutes, it renews just the
        // same: two hours after the request.
        for left in [minutes(120), minutes(10)] {
            let placed = place_session(&ctx, &clock, &user, minutes(60), left);
            let before = wall_secs();
            let response = refresh_with_cookie(&server, &placed.token).await;
            let after = wall_secs();
            let case = format!("{left} left");
            let cookie = refresh_answer(&response, &case).expect("a new cookie");
            assert_eq!(cookie.max_age, 7200, "{case}");
            assert_renews(
                &ctx,
                &cookie,
                &user,
                placed.times.auth_time,
                (before, after),
                &case,
            );
        }

        // A renewed token is revoked like any other: logout ends it. (The
        // clock goes back next to the sign-in, which `place_session` left
        // far behind.)
        clock.set(start() + minutes(60));
        let renewed = refresh_answer(&refresh_with_cookie(&server, &token).await, "again")
            .expect("a new cookie");
        let logout = server
            .post(LOGOUT)
            .add_header("cookie", cookie_header(&renewed.token))
            .await;
        assert_eq!(logout.status_code(), StatusCode::NO_CONTENT);
        for ended in [&token, &renewed.token] {
            assert_eq!(
                me_with_cookie(&server, ended).await.status_code(),
                StatusCode::UNAUTHORIZED
            );
        }
    })
    .await;
}

#[tokio::test]
async fn no_refresh_expires_past_the_absolute_limit() {
    // ADR 0006 §1: "Neither refresh path sets an expiry later than
    // auth_time plus admin_auth.session_absolute_max_secs".
    with_server_at(start(), |server, ctx, clock| async move {
        let user = account(&ctx, "korlat@example.test").await;
        let token = sign_in(&server, "korlat@example.test").await;
        let signed_in = start().timestamp();
        for (elapsed, max_age) in [
            (TimeDelta::hours(10), 7200),
            (TimeDelta::hours(11), 3600),
            (TimeDelta::hours(12) - TimeDelta::seconds(1), 1),
        ] {
            clock.set(start() + elapsed);
            let before = wall_secs();
            let response = refresh_with_cookie(&server, &token).await;
            let after = wall_secs();
            let case = format!("{elapsed} after the sign-in");
            let cookie = refresh_answer(&response, &case).expect("a new cookie");
            assert_eq!(cookie.max_age, max_age, "{case}");
            assert_renews(&ctx, &cookie, &user, signed_in, (before, after), &case);
        }
        // A moment after a whole second, Loco's own reading of the system
        // clock may already fall in the next second, so a capped renewal
        // counts the time left from there: one second less, and the issued
        // `exp` cannot pass the limit (`domain::session::renewal`).
        for (elapsed, max_age) in [
            (TimeDelta::hours(11) + TimeDelta::milliseconds(500), 3599),
            (
                TimeDelta::hours(12) - TimeDelta::seconds(2) + TimeDelta::nanoseconds(1),
                1,
            ),
        ] {
            clock.set(start() + elapsed);
            let before = wall_secs();
            let response = refresh_with_cookie(&server, &token).await;
            let after = wall_secs();
            let case = format!("{elapsed} after the sign-in");
            let cookie = refresh_answer(&response, &case).expect("a new cookie");
            assert_eq!(cookie.max_age, max_age, "{case}");
            assert_renews(&ctx, &cookie, &user, signed_in, (before, after), &case);
        }
        clock.set(
            start() + TimeDelta::hours(12) - TimeDelta::seconds(1) + TimeDelta::nanoseconds(1),
        );
        assert_eq!(
            refresh_answer(
                &refresh_with_cookie(&server, &token).await,
                "a moment after 11 h 59 min 59 s"
            ),
            None,
            "a moment after 11 h 59 min 59 s: less than a whole second is left"
        );
        // At and past the limit there is nothing to renew: no cookie, and
        // the session ends with the token it has.
        for elapsed in [TimeDelta::hours(12), TimeDelta::hours(13)] {
            clock.set(start() + elapsed);
            let response = refresh_with_cookie(&server, &token).await;
            assert_eq!(
                refresh_answer(&response, &format!("{elapsed}")),
                None,
                "{elapsed}"
            );
        }

        // The limit counts from the sign-in, not from the last refresh: a
        // token renewed at 11 h is renewed at 11 h 30 min to the limit only.
        clock.set(start() + TimeDelta::hours(11));
        let renewed = refresh_answer(&refresh_with_cookie(&server, &token).await, "11 h")
            .expect("a new cookie");
        clock.set(start() + TimeDelta::hours(11) + minutes(30));
        let again = refresh_answer(
            &refresh_with_cookie(&server, &renewed.token).await,
            "11 h 30 min",
        )
        .expect("a new cookie");
        assert_eq!(again.max_age, 1800);
        assert_eq!(claims_of(&ctx, &again.token)["auth_time"], signed_in);
    })
    .await;
}

#[tokio::test]
async fn a_request_with_less_than_an_hour_left_gets_a_new_two_hour_cookie() {
    // ADR 0006 §1, "Sliding refresh, passive": any admin request with a
    // full session in the cookie and less than 3,600 s left.
    with_server_at(start(), |server, ctx, clock| async move {
        let user = account(&ctx, "passziv@example.test").await;
        let placed = place_session(&ctx, &clock, &user, minutes(70), minutes(50));
        let before = wall_secs();
        let response = me_with_cookie(&server, &placed.token).await;
        let after = wall_secs();
        assert_eq!(response.status_code(), StatusCode::OK);
        assert_eq!(response.header("cache-control"), "no-store");
        assert_eq!(response.json::<Value>()["email"], "passziv@example.test");
        let cookie = renewed_cookie(&response).expect("a renewed cookie");
        assert_eq!(cookie.max_age, 7200);
        insta::assert_snapshot!(
            cookie.header.replace(&cookie.token, "<token>"),
            @"__Host-admin_session=<token>; HttpOnly; Secure; SameSite=Strict; Path=/; Max-Age=7200"
        );
        assert_renews(
            &ctx,
            &cookie,
            &user,
            placed.times.auth_time,
            (before, after),
            "50 min",
        );
        assert_eq!(
            me_with_cookie(&server, &cookie.token).await.status_code(),
            StatusCode::OK
        );
    })
    .await;
}

#[tokio::test]
async fn the_passive_refresh_fires_only_with_less_than_3600_s_left() {
    with_server_at(start(), |server, ctx, clock| async move {
        let user = account(&ctx, "kuszob@example.test").await;
        for (left, renewed) in [
            (TimeDelta::seconds(3599), true),
            (TimeDelta::seconds(3600), false),
            (TimeDelta::seconds(3601), false),
            (minutes(120), false),
            (minutes(2), true),
        ] {
            let placed = place_session(&ctx, &clock, &user, minutes(60), left);
            let response = me_with_cookie(&server, &placed.token).await;
            assert_eq!(response.status_code(), StatusCode::OK, "{left}");
            match renewed_cookie(&response) {
                Some(cookie) => {
                    assert!(renewed, "{left} left: no cookie expected, got {cookie:?}");
                    assert_eq!(cookie.max_age, 7200, "{left}");
                }
                None => assert!(!renewed, "{left} left: a renewed cookie expected"),
            }
        }
    })
    .await;
}

#[tokio::test]
async fn the_passive_refresh_stops_at_the_absolute_limit() {
    with_server_at(start(), |server, ctx, clock| async move {
        let user = account(&ctx, "passziv-korlat@example.test").await;
        // Eleven hours after the sign-in, with 50 min left: renewed to the
        // limit, an hour away, and no further.
        let placed = place_session(&ctx, &clock, &user, TimeDelta::hours(11), minutes(50));
        let limit_left = placed.times.auth_time + LIMIT_SECS - placed.now.timestamp();
        assert!((3599..=3600).contains(&limit_left), "{limit_left}");
        let cookie = renewed_cookie(&me_with_cookie(&server, &placed.token).await)
            .expect("renewed to the limit");
        assert_eq!(
            cookie.max_age,
            u64::try_from(limit_left).expect("positive"),
            "the limit, not two hours"
        );
        assert_eq!(
            claims_of(&ctx, &cookie.token)["auth_time"],
            placed.times.auth_time
        );

        // Within the last half hour the token already ends at the limit:
        // nothing later can be given, so no cookie is set.
        let placed = place_session(
            &ctx,
            &clock,
            &user,
            TimeDelta::hours(11) + minutes(30),
            minutes(30),
        );
        let response = me_with_cookie(&server, &placed.token).await;
        assert_eq!(response.status_code(), StatusCode::OK);
        assert_eq!(set_cookies(&response), Vec::<String>::new());
    })
    .await;
}

/// A token for `user` with `scope` and `left` of its lifetime left at the
/// app clock, which this moves, signed in an hour before.
fn scoped_session(
    ctx: &AppContext,
    clock: &TestClock,
    user: &Model,
    scope: SessionScope,
    left: TimeDelta,
) -> String {
    let lifetime = u64::try_from(left.num_seconds()).expect("positive");
    let sign = |auth_time: i64| {
        let claims: Map<String, Value> = json!({
            "tv": user.token_version,
            "scope": scope.claim(),
            "auth_time": auth_time,
        })
        .as_object()
        .expect("an object")
        .clone();
        JWT::new(&jwt_secret(ctx))
            .generate_token(lifetime, user.pid.to_string(), claims)
            .expect("signed")
    };
    let probe = token_times(ctx, &sign(0)).expires_at;
    let token = sign(probe - left.num_seconds() - 3600);
    clock.set(at(token_times(ctx, &token).expires_at - left.num_seconds()));
    token
}

#[tokio::test]
async fn neither_a_bearer_token_nor_a_password_change_session_is_refreshed() {
    // ADR 0006 §3: a Bearer token "is not refreshed, so scripts sign in
    // again"; §1 renews only an `admin`-scope token, and §4 opens the
    // refresh route to no `password_change` session.
    with_server_at(start(), |server, ctx, clock| async move {
        let user = account(&ctx, "szkript@example.test").await;
        let placed = place_session(&ctx, &clock, &user, minutes(70), minutes(50));
        let me = server.get(ME).authorization_bearer(&placed.token).await;
        assert_eq!(me.status_code(), StatusCode::OK);
        assert_eq!(set_cookies(&me), Vec::<String>::new(), "passive, Bearer");
        let refresh = server
            .post(REFRESH)
            .authorization_bearer(&placed.token)
            .await;
        assert_eq!(refresh_answer(&refresh, "explicit, Bearer"), None);
        // The same token in the cookie is renewed, on either path.
        assert!(renewed_cookie(&me_with_cookie(&server, &placed.token).await).is_some());
        assert!(
            refresh_answer(&refresh_with_cookie(&server, &placed.token).await, "cookie").is_some()
        );

        let password_change = scoped_session(
            &ctx,
            &clock,
            &user,
            SessionScope::PasswordChange,
            minutes(50),
        );
        let me = me_with_cookie(&server, &password_change).await;
        assert_eq!(me.status_code(), StatusCode::OK);
        assert_eq!(me.json::<Value>()["scope"], "password_change");
        assert_eq!(
            set_cookies(&me),
            Vec::<String>::new(),
            "passive, password_change"
        );
        let refresh = refresh_with_cookie(&server, &password_change).await;
        assert_eq!(refresh.status_code(), StatusCode::UNAUTHORIZED);
        assert_eq!(refresh.json::<Value>(), unauthorized());
        assert_eq!(set_cookies(&refresh), Vec::<String>::new());
    })
    .await;
}

#[tokio::test]
async fn the_refresh_without_a_valid_session_answers_401_and_sets_no_cookie() {
    with_server_at(start(), |server, ctx, _clock| async move {
        account(&ctx, "lejart@example.test").await;
        let token = sign_in(&server, "lejart@example.test").await;
        // Logged out: the token's version is stale.
        let logout = server
            .post(LOGOUT)
            .add_header("cookie", cookie_header(&token))
            .await;
        assert_eq!(logout.status_code(), StatusCode::NO_CONTENT);
        let inactive = account(&ctx, "inaktiv@example.test").await;
        let inactive_token = sign_in(&server, "inaktiv@example.test").await;
        let mut deactivated: ActiveModel = inactive.into();
        deactivated.active = ActiveValue::Set(false);
        deactivated.update(&ctx.db).await.expect("deactivate");

        let cases = [
            ("no token", server.post(REFRESH).await),
            (
                "not a JWT",
                refresh_with_cookie(&server, "nem.jwt.token").await,
            ),
            ("logged out", refresh_with_cookie(&server, &token).await),
            (
                "deactivated",
                refresh_with_cookie(&server, &inactive_token).await,
            ),
        ];
        for (case, response) in cases {
            assert_eq!(response.status_code(), StatusCode::UNAUTHORIZED, "{case}");
            assert_eq!(response.json::<Value>(), unauthorized(), "{case}");
            assert_eq!(response.header("cache-control"), "no-store", "{case}");
            assert_eq!(set_cookies(&response), Vec::<String>::new(), "{case}");
        }
        // POST only, like the logout: a link or an image cannot call it.
        let get = server
            .get(REFRESH)
            .add_header(
                "cookie",
                cookie_header(&sign_in(&server, "lejart@example.test").await),
            )
            .await;
        assert_eq!(get.status_code(), StatusCode::METHOD_NOT_ALLOWED);
        assert_eq!(get.header(ALLOW), "POST");
        assert_eq!(set_cookies(&get), Vec::<String>::new());
    })
    .await;
}

#[tokio::test]
async fn an_answer_carries_at_most_one_session_cookie() {
    // A browser keeps the last of two cookies with one name, so the passive
    // refresh never adds one beside the handler's own: the explicit
    // refresh's answer carries its renewal alone, also when the passive
    // one is due (the logout's case is in auth_logout.rs).
    with_server_at(start(), |server, ctx, clock| async move {
        let user = account(&ctx, "egy-suti@example.test").await;
        let placed = place_session(&ctx, &clock, &user, minutes(70), minutes(50));
        let response = refresh_with_cookie(&server, &placed.token).await;
        let cookies = set_cookies(&response);
        assert_eq!(cookies.len(), 1, "{cookies:?}");
        assert!(
            cookies[0].starts_with(&format!("{SESSION_COOKIE_NAME}=")),
            "{cookies:?}"
        );
        assert_eq!(
            refresh_answer(&response, "explicit, passive due")
                .expect("a new cookie")
                .max_age,
            7200
        );
    })
    .await;
}

/// The id of the handler's account, for the probe route.
async fn who(session: AdminSession) -> Json<Value> {
    Json(json!({ "id": session.user.id }))
}

#[tokio::test]
async fn every_guarded_answer_gets_the_passive_refresh_a_403_included() {
    // The guard renews the session it opened on whatever it answers: the
    // handler's answer, and its own 403 for an account that does not meet
    // the route's requirement, since the session itself is valid. The probe
    // route stands for the admin-only routes that later slices add.
    with_server_at(start(), |_server, ctx, clock| async move {
        let probe = probe_server(
            &ctx,
            build(vec![AdminRoute::get(
                "/probe/admin-only",
                Requirement::AdminRole,
                who,
            )]),
        );
        let writer = account_with(&ctx, "iro@example.test", &[Role::Writer], &[], |_| {}).await;
        let admin = account_with(&ctx, "admin@example.test", &[Role::Admin], &[], |_| {}).await;
        for (user, status) in [(&writer, StatusCode::FORBIDDEN), (&admin, StatusCode::OK)] {
            let placed = place_session(&ctx, &clock, user, minutes(70), minutes(50));
            let response = probe
                .get("/api/admin/probe/admin-only")
                .add_header("cookie", cookie_header(&placed.token))
                .await;
            assert_eq!(response.status_code(), status, "{}", user.id);
            let cookie = renewed_cookie(&response).expect("a renewed cookie");
            assert_eq!(cookie.max_age, 7200, "{}", user.id);
            assert_eq!(claims_of(&ctx, &cookie.token)["pid"], user.pid.to_string());
        }
    })
    .await;
}
