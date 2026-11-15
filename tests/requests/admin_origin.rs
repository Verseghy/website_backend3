//! The `Origin` check of cookie-authenticated admin writes (ADR 0006 §3, as
//! amended by ADR 0044; parity AUTH-01, SHR-01).
//!
//! A page on a sibling host, such as the public site at
//! `https://verseghy-gimnazium.net`, is the same site as the admin, so the
//! browser sends the `SameSite=Strict` session cookie with its requests
//! (ADR 0014). The route table's guard therefore answers
//! `403 origin_mismatch` to a `POST`, `PUT`, `PATCH` or `DELETE` that
//! carries the session cookie or the two-factor cookie unless its one
//! `Origin` header is the admin origin, and to one with no `Origin` header
//! at all (the maintainer's answer of 2026-10-25, research: DECISIONS R9
//! (D08)). These tests check, through the application's whole middleware
//! stack:
//!
//! - against production's admin origin, that the sibling hosts and every
//!   other spelling are refused and nothing is done, the session holds, and
//!   that the admin origin itself is let through;
//! - that a write without an `Origin` header, with `null` or with two
//!   `Origin` headers is refused;
//! - a walk over every write route of the application's table, so a route
//!   that a later slice adds is checked without a line of its own, and the
//!   same walk over a probe table with each method and requirement;
//! - that the check comes before the session and the requirement, so a
//!   refused write gets the same answer whatever its session, and before the
//!   handler;
//! - that it also comes before the sliding refresh of ADR 0006 §1, passive
//!   and explicit, so a refused write renews no session, not even one whose
//!   passive refresh is due;
//! - what it does not check: reads, and requests without a credential
//!   cookie, a first sign-in from a foreign `Origin` included (ADR 0044 §2;
//!   research: DECISIONS R35 (D152)). That a Bearer request is exempt is
//!   tested with the token endpoint that issues one (`auth-bearer-bcrypt`);
//! - that the check runs only on the table's routes: a path or a method the
//!   table does not declare keeps its `404` or `405`;
//! - that a context without an admin origin lets no checked write through.
//!
//! Every account is invented.

use std::sync::Arc;

use axum::{
    Json,
    http::{
        Method, StatusCode,
        header::{AUTHORIZATION, COOKIE, ORIGIN, SET_COOKIE},
    },
};
use axum_test::{TestRequest, TestResponse, TestServer};
use backend3::{
    app::{clock, set_clock},
    controllers::{
        admin::{
            origin::{admin_origin, set_admin_origin},
            routes::{AdminRoute, build, table},
            session::{AdminSession, session_limits},
        },
        admin_ui::ADMIN_SECURITY_HEADERS,
    },
    domain::{
        api_path::ADMIN_API_PREFIX,
        authz::{Permission, Requirement, Role},
        origin::{AdminOrigin, is_write_method},
        session::SessionScope,
        session_cookie::{MFA_COOKIE_NAME, SESSION_COOKIE_NAME},
    },
    models::users::Model,
    settings::Settings,
};
use chrono::TimeDelta;
use loco_rs::app::{AppContext, SharedStore};
use serde_json::{Value, json};

use crate::support::{
    ADMIN_ORIGIN,
    admin_probe::{account_with, probe_server, session_token},
    session_refresh::{at, place_session, renewed_cookie},
    with_app, with_server, with_server_at,
};

/// Production's admin origin (ADR 0014).
const PRODUCTION: &str = "https://backend3.microshift.verseghy-gimnazium.net";

/// Origins that production's admin refuses: the hosts of the same site,
/// which `SameSite` does not keep out, and the spellings no browser at the
/// admin sends.
const FOREIGN_TO_PRODUCTION: [&str; 10] = [
    "https://verseghy-gimnazium.net",
    "https://backend.microshift.verseghy-gimnazium.net",
    "https://backend2.microshift.verseghy-gimnazium.net",
    "http://backend3.microshift.verseghy-gimnazium.net",
    "https://backend3.microshift.verseghy-gimnazium.net:443",
    "https://backend3.microshift.verseghy-gimnazium.net/",
    "https://BACKEND3.microshift.verseghy-gimnazium.net",
    "https://backend3.microshift.verseghy-gimnazium.net.example",
    "https://example.org",
    "null",
];

const LOGIN: &str = "/api/admin/auth/login";
const LOGOUT: &str = "/api/admin/auth/logout";
const ME: &str = "/api/admin/auth/me";
const REFRESH: &str = "/api/admin/auth/refresh";

/// The envelope of `origin_mismatch` (ADR 0021 §4).
fn origin_mismatch() -> Value {
    json!({
        "error": "origin_mismatch",
        "description": "The request does not come from the admin's origin; a write with the \
                        session cookie must carry it in the Origin header",
        "field": null,
        "params": {},
    })
}

/// Asserts the `403 origin_mismatch` answer, with the headers every admin
/// refusal carries and no cookie: the session is left as it is.
fn assert_origin_refused(response: &TestResponse, case: &str) {
    assert_eq!(response.status_code(), StatusCode::FORBIDDEN, "{case}");
    assert_eq!(response.json::<Value>(), origin_mismatch(), "{case}");
    assert_eq!(response.header("cache-control"), "no-store", "{case}");
    assert_eq!(
        response.header("content-type"),
        "application/json",
        "{case}"
    );
    assert!(response.maybe_header(SET_COOKIE).is_none(), "{case}");
}

/// The `Cookie` header of a browser that holds `token` in the session
/// cookie, beside a cookie of a sibling host.
fn session_cookie(token: &str) -> String {
    format!("theme=dark; {SESSION_COOKIE_NAME}={token}")
}

/// `request` with `token` in the session cookie and `origin`, if any, as
/// its `Origin` header.
fn with_cookie(request: TestRequest, token: &str, origin: Option<&str>) -> TestRequest {
    let request = request.add_header(COOKIE, session_cookie(token));
    match origin {
        Some(origin) => request.add_header(ORIGIN, origin),
        None => request,
    }
}

/// An invented account with the `admin` role, and a full session for it.
async fn admin(ctx: &AppContext, email: &str) -> (Model, String) {
    let user = account_with(ctx, email, &[Role::Admin], &[], |_| {}).await;
    let token = session_token(ctx, &user, SessionScope::Admin);
    (user, token)
}

/// Whether `token`'s session still holds: `GET /auth/me` with it answers
/// `200`.
async fn session_holds(server: &TestServer, token: &str) -> bool {
    with_cookie(server.get(ME), token, None).await.status_code() == StatusCode::OK
}

/// The stored token version of `user`.
async fn token_version(ctx: &AppContext, user: &Model) -> i32 {
    Model::find_by_pid(&ctx.db, &user.pid.to_string())
        .await
        .expect("the stored account")
        .token_version
}

#[tokio::test]
async fn the_test_origin_is_the_configured_one() {
    // tests/support's ADMIN_ORIGIN is what config/test.yaml configures, and
    // what `App::after_context` put into the shared store.
    with_app(|_router, ctx| async move {
        let settings = Settings::from_section(ctx.config.settings.as_ref()).expect("settings");
        assert_eq!(settings.admin_auth.origin.as_str(), ADMIN_ORIGIN);
        assert_eq!(admin_origin(&ctx).expect("stored").as_str(), ADMIN_ORIGIN);
    })
    .await;
}

#[tokio::test]
async fn a_sibling_host_cannot_log_the_admin_out() {
    // ADR 0006 §3: the public site's page makes the browser post the
    // logout with the cookie. It is refused, nothing ends, and the admin
    // origin itself is let through.
    with_server(|server, ctx| async move {
        set_admin_origin(&ctx, AdminOrigin::parse(PRODUCTION).expect("an origin"));
        let (user, token) = admin(&ctx, "admin@example.test").await;
        for foreign in FOREIGN_TO_PRODUCTION {
            let response = with_cookie(server.post(LOGOUT), &token, Some(foreign)).await;
            assert_origin_refused(&response, foreign);
        }
        assert_eq!(token_version(&ctx, &user).await, 0, "no logout ran");
        assert!(session_holds(&server, &token).await);

        let response = with_cookie(server.post(LOGOUT), &token, Some(PRODUCTION)).await;
        assert_eq!(response.status_code(), StatusCode::NO_CONTENT);
        assert_eq!(token_version(&ctx, &user).await, 1);
        assert!(!session_holds(&server, &token).await);
    })
    .await;
}

#[tokio::test]
async fn a_cookie_write_without_exactly_the_admin_origin_is_refused() {
    // D08: no `Origin` header is refused like a foreign one (Bearer
    // requests are exempt; this one has the cookie). Neither is two
    // headers, even when one of them is right.
    with_server(|server, ctx| async move {
        let (user, token) = admin(&ctx, "admin@example.test").await;
        let cases: [(&str, TestRequest); 4] = [
            ("no Origin", with_cookie(server.post(LOGOUT), &token, None)),
            (
                "Origin: null",
                with_cookie(server.post(LOGOUT), &token, Some("null")),
            ),
            (
                "an empty Origin",
                with_cookie(server.post(LOGOUT), &token, Some("")),
            ),
            (
                "two Origin headers",
                with_cookie(server.post(LOGOUT), &token, Some(ADMIN_ORIGIN))
                    .add_header(ORIGIN, ADMIN_ORIGIN),
            ),
        ];
        for (case, request) in cases {
            assert_origin_refused(&request.await, case);
        }
        assert_eq!(token_version(&ctx, &user).await, 0, "no logout ran");
        assert!(session_holds(&server, &token).await);
    })
    .await;
}

#[tokio::test]
async fn a_cookie_counts_however_the_request_carries_it() {
    // The check reads the cookies as Loco does, so no spelling of the
    // `Cookie` header gets a credential cookie past it; with a Bearer
    // header beside it, Loco reads the cookie first, so it is checked too.
    with_server(|server, ctx| async move {
        let (user, token) = admin(&ctx, "admin@example.test").await;
        let foreign = "https://verseghy-gimnazium.net";
        let cookies = [
            format!("{SESSION_COOKIE_NAME}={token}"),
            format!("lang=hu;{SESSION_COOKIE_NAME}={token}; theme=dark"),
            format!("__Host-admin%5Fsession={token}"),
            format!("{SESSION_COOKIE_NAME}="),
            // The two-factor step's cookie (ADR 0006 §4): its step reads it
            // itself, and the check covers it from the start.
            format!("{MFA_COOKIE_NAME}={token}"),
        ];
        for cookie in &cookies {
            let response = server
                .post(LOGOUT)
                .add_header(COOKIE, cookie.as_str())
                .add_header(ORIGIN, foreign)
                .await;
            assert_origin_refused(&response, cookie);
        }
        // In a second `Cookie` header, as HTTP/2 clients may send it.
        let response = server
            .post(LOGOUT)
            .add_header(COOKIE, "theme=dark")
            .add_header(COOKIE, format!("{SESSION_COOKIE_NAME}={token}"))
            .add_header(ORIGIN, foreign)
            .await;
        assert_origin_refused(&response, "a second Cookie header");
        // Beside a Bearer token.
        let response = with_cookie(server.post(LOGOUT), &token, Some(foreign))
            .add_header(AUTHORIZATION, format!("Bearer {token}"))
            .await;
        assert_origin_refused(&response, "the cookie beside a Bearer token");
        assert_eq!(token_version(&ctx, &user).await, 0, "no logout ran");
    })
    .await;
}

#[tokio::test]
async fn reads_and_writes_without_a_credential_cookie_are_not_checked() {
    with_server(|server, ctx| async move {
        let (_user, token) = admin(&ctx, "admin@example.test").await;
        let foreign = "https://verseghy-gimnazium.net";
        // A read changes nothing, and no CORS header lets the other origin
        // read the answer (ADR 0014).
        let me = with_cookie(server.get(ME), &token, Some(foreign)).await;
        assert_eq!(me.status_code(), StatusCode::OK);
        // A sign-in that carries no credential cookie, with any or no
        // Origin and other cookies, reaches the sign-in, which refuses the
        // invented account without a password. The maintainer kept it so on
        // 2026-11-15 (research: DECISIONS R35 (D152)): no page on another
        // origin can send it, because it reads only JSON and `/api/admin`
        // answers no preflight (ADR 0044 §2; tests/requests/auth_login.rs,
        // tests/requests/public_router.rs, tests/boot/config_files.rs).
        let body = json!({ "email": "admin@example.test", "password": "bármi, ami hosszú" });
        for (case, request) in [
            ("no Origin", server.post(LOGIN)),
            (
                "a foreign Origin",
                server.post(LOGIN).add_header(ORIGIN, foreign),
            ),
            (
                "another cookie",
                server
                    .post(LOGIN)
                    .add_header(COOKIE, "theme=dark")
                    .add_header(ORIGIN, foreign),
            ),
        ] {
            let response = request.json(&body).await;
            assert_eq!(response.status_code(), StatusCode::UNAUTHORIZED, "{case}");
            assert_eq!(
                response.json::<Value>()["error"],
                json!("invalid_credentials"),
                "{case}"
            );
        }
        // A sign-in that carries the session cookie is a cookie write like
        // any other, `Public` as it is.
        let response = with_cookie(server.post(LOGIN), &token, Some(foreign))
            .json(&body)
            .await;
        assert_origin_refused(&response, "a sign-in with the session cookie");
    })
    .await;
}

#[tokio::test]
async fn the_check_runs_only_on_the_routes_of_the_table() {
    // The check is the guard's first step, and the guard wraps the handler
    // of each route of the table, after routing (ADR 0044 §5). A cookie
    // write from another origin to a path the table does not declare keeps
    // the 404 of an unknown path, and one with a method the route does not
    // declare keeps axum's 405: neither reaches a guard.
    with_server(|server, ctx| async move {
        let (_user, token) = admin(&ctx, "admin@example.test").await;
        let foreign = "https://verseghy-gimnazium.net";
        let unknown = with_cookie(
            server.post(&format!("{ADMIN_API_PREFIX}/no-such-route")),
            &token,
            Some(foreign),
        )
        .await;
        assert_eq!(unknown.status_code(), StatusCode::NOT_FOUND);
        let wrong_method = with_cookie(server.post(ME), &token, Some(foreign)).await;
        assert_eq!(wrong_method.status_code(), StatusCode::METHOD_NOT_ALLOWED);
    })
    .await;
}

/// The path of `route` with every parameter given a value.
fn concrete(route: &AdminRoute) -> String {
    route
        .path()
        .split('/')
        .map(|segment| {
            if segment.starts_with('{') {
                "1"
            } else {
                segment
            }
        })
        .collect::<Vec<_>>()
        .join("/")
}

/// Sends every write route of `routes` with `token` in the session cookie,
/// once from each origin of `foreign` and once without an `Origin`, and
/// asserts the refusal; returns how many routes it sent.
async fn assert_every_write_route_refuses(
    server: &TestServer,
    routes: &[AdminRoute],
    token: &str,
    foreign: &[&str],
) -> usize {
    let writes: Vec<&AdminRoute> = routes
        .iter()
        .filter(|route| is_write_method(route.method().as_str()))
        .collect();
    for route in &writes {
        let path = concrete(route);
        let origins = foreign.iter().copied().map(Some).chain([None]);
        for origin in origins {
            let response =
                with_cookie(server.method(route.method().clone(), &path), token, origin).await;
            assert_origin_refused(
                &response,
                &format!("{} {path} from {origin:?}", route.method()),
            );
        }
    }
    writes.len()
}

#[tokio::test]
async fn every_write_route_of_the_table_refuses_a_foreign_origin() {
    // Walks the application's table, so a write route that a later slice
    // adds is checked here without a line of its own; `Public` routes are
    // walked too, with the cookie.
    with_server(|server, ctx| async move {
        set_admin_origin(&ctx, AdminOrigin::parse(PRODUCTION).expect("an origin"));
        let (user, token) = admin(&ctx, "admin@example.test").await;
        let walked =
            assert_every_write_route_refuses(&server, &table(), &token, &FOREIGN_TO_PRODUCTION)
                .await;
        // Today: the sign-in and the logout.
        assert!(walked >= 2, "{walked} write routes");
        // Nothing reached a handler: the session holds, no logout ran.
        assert_eq!(token_version(&ctx, &user).await, 0);
        assert!(session_holds(&server, &token).await);
    })
    .await;
}

/// The account the handler's session carries.
async fn who(session: AdminSession) -> Json<Value> {
    Json(json!({ "id": session.user.id }))
}

/// A handler without a session.
async fn public() -> Json<Value> {
    Json(json!({ "public": true }))
}

/// A write route of each method and requirement kind.
fn probe_table() -> Vec<AdminRoute> {
    vec![
        AdminRoute::post(
            "/probe/labels",
            Requirement::Permission(Permission::EditLabels),
            who,
        ),
        AdminRoute::put("/probe/labels/{id}", Requirement::AdminRole, who),
        AdminRoute::patch("/probe/own", Requirement::AnyActiveUser, who),
        AdminRoute::delete("/probe/labels/{id}", Requirement::AdminRole, who),
        AdminRoute::post("/probe/public", Requirement::Public, public),
        AdminRoute::post("/probe/upload", Requirement::AdminRole, who).upload(),
        AdminRoute::get("/probe/labels", Requirement::AdminRole, who),
    ]
}

#[tokio::test]
async fn the_probe_table_refuses_a_foreign_origin_for_every_method_and_requirement() {
    // The application's table holds only `POST` writes so far, so the same
    // walk runs over `PUT`, `PATCH`, `DELETE` and each kind of requirement.
    with_app(|_router, ctx| async move {
        let server = probe_server(&ctx, build(probe_table()));
        let (_user, token) = admin(&ctx, "admin@example.test").await;
        let walked = assert_every_write_route_refuses(
            &server,
            &probe_table(),
            &token,
            &["https://verseghy-gimnazium.net", "http://localhost:5151"],
        )
        .await;
        assert_eq!(walked, 6);
        // From the admin origin, every write reaches its handler.
        for route in probe_table() {
            let path = concrete(&route);
            let response = with_cookie(
                server.method(route.method().clone(), &path),
                &token,
                Some(ADMIN_ORIGIN),
            )
            .await;
            assert_eq!(
                response.status_code(),
                StatusCode::OK,
                "{} {path}",
                route.method()
            );
        }
    })
    .await;
}

#[tokio::test]
async fn the_check_comes_before_the_session_and_the_requirement() {
    // A refused write gets the same answer whatever its session: a page on
    // another origin learns nothing about it, and no database read is
    // spent on it. From the admin origin the guard goes on as before.
    with_app(|_router, ctx| async move {
        let server = probe_server(&ctx, build(probe_table()));
        let nobody = account_with(&ctx, "senki@example.test", &[], &[], |_| {}).await;
        let nobody_token = session_token(&ctx, &nobody, SessionScope::Admin);
        let mfa_token = session_token(&ctx, &nobody, SessionScope::Mfa);
        let path = format!("{ADMIN_API_PREFIX}/probe/labels");
        for (case, token, from_admin) in [
            (
                "a token that is not one",
                "nem.jwt.token",
                StatusCode::UNAUTHORIZED,
            ),
            ("an mfa token", mfa_token.as_str(), StatusCode::UNAUTHORIZED),
            (
                "an account without edit labels",
                nobody_token.as_str(),
                StatusCode::FORBIDDEN,
            ),
        ] {
            let foreign = with_cookie(
                server.post(&path),
                token,
                Some("https://verseghy-gimnazium.net"),
            )
            .await;
            assert_origin_refused(&foreign, case);
            let missing = with_cookie(server.post(&path), token, None).await;
            assert_origin_refused(&missing, case);
            let admin = with_cookie(server.post(&path), token, Some(ADMIN_ORIGIN)).await;
            assert_eq!(admin.status_code(), from_admin, "{case}");
        }
        // No cookie at all: the session check answers, whatever the origin.
        let anonymous = server
            .method(Method::DELETE, &format!("{path}/1"))
            .add_header(ORIGIN, "https://verseghy-gimnazium.net")
            .await;
        assert_eq!(anonymous.status_code(), StatusCode::UNAUTHORIZED);
    })
    .await;
}

#[tokio::test]
async fn a_refused_write_renews_no_session() {
    // ADR 0006 §1 renews a session that the guard has opened; the `Origin`
    // check answers before any is opened, so neither the passive refresh,
    // which is due here, nor the explicit one reaches a refused write. The
    // same session is renewed by a read, and by the explicit refresh from
    // the admin origin.
    with_server_at(at(1_793_610_000), |server, ctx, clock| async move {
        let user = account_with(&ctx, "lejaro@example.test", &[Role::Admin], &[], |_| {}).await;
        let placed = place_session(
            &ctx,
            &clock,
            &user,
            TimeDelta::minutes(70),
            TimeDelta::minutes(50),
        );
        for path in [LOGOUT, REFRESH] {
            let foreign = with_cookie(
                server.post(path),
                &placed.token,
                Some("https://verseghy-gimnazium.net"),
            )
            .await;
            assert_origin_refused(&foreign, path);
            let missing = with_cookie(server.post(path), &placed.token, None).await;
            assert_origin_refused(&missing, path);
        }
        assert_eq!(token_version(&ctx, &user).await, 0, "no logout ran");

        let read = with_cookie(server.get(ME), &placed.token, None).await;
        assert_eq!(read.status_code(), StatusCode::OK);
        assert_eq!(
            renewed_cookie(&read).map(|cookie| cookie.max_age),
            Some(7200),
            "the passive refresh is due"
        );
        let refresh = with_cookie(server.post(REFRESH), &placed.token, Some(ADMIN_ORIGIN)).await;
        assert_eq!(refresh.status_code(), StatusCode::NO_CONTENT);
        assert_eq!(
            renewed_cookie(&refresh).map(|cookie| cookie.max_age),
            Some(7200)
        );
    })
    .await;
}

#[tokio::test]
async fn a_context_without_an_admin_origin_lets_no_checked_write_through() {
    // Every context that Loco builds has one (`App::after_context`); one
    // built by hand without it answers 500 to a checked write rather than
    // letting it through, and still serves what is not checked.
    with_app(|_router, ctx| async move {
        let (_user, token) = admin(&ctx, "admin@example.test").await;
        let mut bare = ctx.clone();
        bare.shared_store = Arc::new(SharedStore::default());
        // Everything else the guard reads stays: the app clock and the
        // session limits of the sliding refresh (ADR 0006 §1), which a
        // read with a session needs.
        set_clock(&bare, clock(&ctx).expect("the app clock"));
        bare.shared_store
            .insert(session_limits(&ctx).expect("the session limits"));
        let server = probe_server(&bare, build(probe_table()));
        let path = format!("{ADMIN_API_PREFIX}/probe/labels");
        let write = with_cookie(server.post(&path), &token, Some(ADMIN_ORIGIN)).await;
        assert_eq!(write.status_code(), StatusCode::INTERNAL_SERVER_ERROR);
        assert!(write.maybe_header(SET_COOKIE).is_none());
        let read = with_cookie(server.get(&path), &token, None).await;
        assert_eq!(read.status_code(), StatusCode::OK);
    })
    .await;
}

#[test]
fn the_admin_pages_let_their_writes_carry_the_admin_origin() {
    // Under the Fetch standard, a same-origin `POST` carries the page's
    // origin unless the page's referrer policy is `no-referrer`, which turns
    // it into `null`; with `same-origin` or a `strict-*` policy it is the
    // origin itself on an https page writing to the same https origin. The
    // admin's own writes therefore depend on the policy its pages are served
    // with (ADR 0023 §6): `no-referrer` there would refuse every write of the
    // admin. The E2E sign-out pins the header a real browser sends.
    let policy = ADMIN_SECURITY_HEADERS
        .iter()
        .find(|(name, _)| *name == "referrer-policy")
        .map(|(_, value)| *value);
    assert_eq!(policy, Some("strict-origin-when-cross-origin"));
}
