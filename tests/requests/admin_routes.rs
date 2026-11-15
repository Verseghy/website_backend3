//! The admin route table and its guard (ADR 0007 §4, §5; ADR 0021 §4;
//! parity SHR-01).
//!
//! The table: every route that `App::routes` mounts under `/api/admin` is
//! in `controllers::admin::routes::table`, with a requirement, and every
//! route of the table is mounted, each method once; Loco records a method
//! for every admin route, so none escapes that comparison; only the sign-in
//! steps are `Public`, and only the own-account routes of ADR 0006 §4 accept
//! a `password_change` session. A route added to `App::routes` outside the
//! table, or opened wider than these lists, fails here. The other hooks
//! that could mount a route (`before_routes`, `after_routes`, `serve`, an
//! initializer) never reach `App::routes`, so `tests/architecture.rs` checks
//! them by text (`tests/architecture/admin_mounts.rs`).
//!
//! The guard, on a probe table built by the same `build` and served behind
//! the application's middleware stack (`tests/support/admin_probe.rs`):
//! `401` is decided before `403`, and both before the handler; an account
//! that does not meet the requirement gets the `403 forbidden` envelope with
//! `Cache-Control: no-store`, and a list it may not see is never `[]`; each
//! kind of requirement admits exactly the accounts ADR 0007 names; a role
//! or grant change applies on the next request; a handler that takes the
//! session on a route without a guarded session answers `500`; and the body
//! limits follow the route's kind. Every account here is invented.

use std::collections::BTreeMap;

use axum::{
    Json,
    extract::rejection::JsonRejection,
    http::{
        Method, StatusCode,
        header::{COOKIE, ORIGIN, SET_COOKIE},
    },
    response::{IntoResponse as _, Response},
    routing::{any, get, post},
};
use axum_test::{TestResponse, TestServer};
use backend3::{
    app::App,
    controllers::admin::{
        JSON_BODY_LIMIT, json_rejection_response,
        routes::{AdminRoute, RequestBody, build, table},
        session::{AdminOrPasswordChange, AdminSession},
    },
    domain::{
        api_path::{ADMIN_API_PREFIX, ApiFamily, api_family},
        authz::{AccountGrants, Permission, Requirement, Role},
        session::SessionScope,
        session_cookie::SESSION_COOKIE_NAME,
    },
};
use loco_rs::{
    app::{AppContext, Hooks},
    controller::{AppRoutes, ListRoutes, Routes},
};
use sea_orm::ActiveValue;
use serde_json::{Value, json};

use crate::support::{
    ADMIN_ORIGIN,
    admin_probe::{account_with, give, probe_server, session_token, take_all},
    with_app, with_server,
};

/// The routes allowed to be `Public`: the sign-in steps, which issue the
/// session and so cannot ask for one (ADR 0006 §3, §4): the browser's
/// sign-in and the scripts' Bearer token endpoint. The two-factor step and
/// the reset-link redemption join this list with their slices; nothing else
/// ever does.
const PUBLIC_ROUTES: [(&str, &str); 2] = [
    ("POST", "/api/admin/auth/login"),
    ("POST", "/api/admin/auth/token"),
];

/// The routes allowed to accept a `password_change` session: those of
/// ADR 0006 §4 that exist so far. `POST /auth/change-password` joins with
/// its slice; no other route ever does.
const PASSWORD_CHANGE_ROUTES: [(&str, &str); 2] = [
    ("GET", "/api/admin/auth/me"),
    ("POST", "/api/admin/auth/logout"),
];

/// The routes of `routes` under `/api/admin`, as Loco's route table lists
/// them (`AppRoutes::collect()`, what `cargo loco routes` prints): each with
/// its path, `/api/admin` included, and the methods Loco recorded for it.
fn admin_routes_of(routes: &AppRoutes) -> Vec<ListRoutes> {
    routes
        .collect()
        .into_iter()
        .filter(|route| api_family(&route.uri) == Some(ApiFamily::Admin))
        .collect()
}

/// How often each `(method, path)` of `pairs` occurs, keyed `METHOD path`.
fn counts(pairs: impl IntoIterator<Item = (Method, String)>) -> BTreeMap<String, usize> {
    let mut counts = BTreeMap::new();
    for (method, path) in pairs {
        *counts.entry(format!("{method} {path}")).or_default() += 1;
    }
    counts
}

/// What is wrong between the routes that `routes` mounts under
/// `/api/admin` and the admin route table `declared`, one line per finding;
/// empty when every mounted admin route is declared once and every declared
/// route is mounted once.
///
/// - `routes`: the routes to check; `App::routes` in the application.
/// - `declared`: the route table; `table()` in the application.
fn table_findings(routes: &AppRoutes, declared: &[AdminRoute]) -> Vec<String> {
    let admin = admin_routes_of(routes);
    let mut findings = Vec::new();
    // loco-rs 1.2.0's `Routes::add` reads a route's methods from the `Debug`
    // text of its axum `MethodRouter` (`describe::method_action`) and finds
    // none for `any(...)` or a shape it cannot parse
    // (docs/design/openapi-spike.md, P5). Such a route would add no pair to
    // the comparison below and pass it unseen, so it is a finding itself.
    for route in admin.iter().filter(|route| route.actions.is_empty()) {
        findings.push(format!(
            "{}: Loco records no method for it (mounted with `any(...)` or a method router it \
             cannot read), so the comparison with the table cannot see it; declare it in \
             routes::table() with `AdminRoute` instead",
            route.uri
        ));
    }
    let mounted = counts(admin.iter().flat_map(|route| {
        route
            .actions
            .iter()
            .map(|method| (method.clone(), route.uri.clone()))
    }));
    let declared = counts(
        declared
            .iter()
            .map(|route| (route.method().clone(), route.path())),
    );
    for route in mounted
        .keys()
        .filter(|route| !declared.contains_key(*route))
    {
        findings.push(format!(
            "{route}: mounted under {ADMIN_API_PREFIX} without a requirement in the admin route \
             table (src/controllers/admin/routes.rs, ADR 0007 §5); add it to table() instead of \
             mounting it elsewhere"
        ));
    }
    for route in declared
        .keys()
        .filter(|route| !mounted.contains_key(*route))
    {
        findings.push(format!(
            "{route}: declared in the admin route table but not mounted as declared (a path \
             that Loco normalises, such as one with a trailing slash, shows here too)"
        ));
    }
    for (what, list) in [("mounted", &mounted), ("declared", &declared)] {
        for (route, count) in list.iter().filter(|(_, count)| **count > 1) {
            findings.push(format!("{route}: {what} {count} times"));
        }
    }
    findings
}

#[tokio::test]
async fn every_mounted_admin_route_is_declared_in_the_table_once() {
    with_app(|_router, ctx| async move {
        let routes = App::routes(&ctx);
        // The three routes of the auth slices at least, so the comparison
        // never passes on two empty lists.
        assert!(
            admin_routes_of(&routes).iter().any(
                |route| route.uri == "/api/admin/auth/login" && route.actions == [Method::POST]
            ),
            "{:?}",
            admin_routes_of(&routes)
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
        );
        let findings = table_findings(&routes, &table());
        assert!(
            findings.is_empty(),
            "the admin routes that App::routes mounts and the admin route table differ:\n{}",
            findings.join("\n")
        );
    })
    .await;
}

#[tokio::test]
async fn the_table_check_names_every_route_mounted_or_declared_wrongly() {
    // The check above passes only if it can fail: each mistake it exists
    // for, added to the application's own routes and table, is named.
    with_app(|_router, ctx| async move {
        async fn handler() {}
        let routes = App::routes(&ctx).add_route(
            Routes::new()
                .prefix(ADMIN_API_PREFIX)
                // A route Loco records no method for.
                .add("/sneaky-any", any(handler))
                // A route outside the table.
                .add("/sneaky-get", get(handler))
                // A table route mounted a second time.
                .add("/auth/login", post(handler)),
        );
        let mut declared = table();
        // Declared, never mounted.
        declared.push(AdminRoute::get(
            "/declared-only",
            Requirement::AdminRole,
            handler,
        ));
        // Declared twice.
        declared.push(AdminRoute::post(
            "/auth/logout",
            Requirement::AnyActiveUser,
            handler,
        ));
        let findings = table_findings(&routes, &declared);
        let starts = [
            "/api/admin/sneaky-any: Loco records no method for it",
            "GET /api/admin/sneaky-get: mounted under /api/admin without a requirement",
            "GET /api/admin/declared-only: declared in the admin route table but not mounted",
            "POST /api/admin/auth/login: mounted 2 times",
            "POST /api/admin/auth/logout: declared 2 times",
        ];
        assert_eq!(findings.len(), starts.len(), "{findings:#?}");
        for start in starts {
            assert!(
                findings.iter().any(|finding| finding.starts_with(start)),
                "{start} in {findings:#?}"
            );
        }
        // The application's own routes and table are clean.
        assert_eq!(
            table_findings(&App::routes(&ctx), &table()),
            Vec::<String>::new()
        );
    })
    .await;
}

#[test]
fn only_the_sign_in_steps_are_public() {
    let public: Vec<(String, String)> = table()
        .iter()
        .filter(|route| route.requirement() == Requirement::Public)
        .map(|route| (route.method().to_string(), route.path()))
        .collect();
    let allowed: Vec<(String, String)> = PUBLIC_ROUTES
        .iter()
        .map(|(method, path)| ((*method).to_owned(), (*path).to_owned()))
        .collect();
    assert_eq!(
        public, allowed,
        "a Public route lets every request in without a session (ADR 0007 §5); only the \
         sign-in steps of ADR 0006 may be one"
    );
}

#[test]
fn only_the_own_account_routes_accept_a_password_change_session() {
    let opened: Vec<(String, String, Requirement)> = table()
        .iter()
        .filter(|route| route.scopes().contains(&SessionScope::PasswordChange))
        .map(|route| {
            (
                route.method().to_string(),
                route.path(),
                route.requirement(),
            )
        })
        .collect();
    let allowed: Vec<(String, String, Requirement)> = PASSWORD_CHANGE_ROUTES
        .iter()
        .map(|(method, path)| {
            (
                (*method).to_owned(),
                (*path).to_owned(),
                Requirement::AnyActiveUser,
            )
        })
        .collect();
    assert_eq!(opened, allowed, "ADR 0006 §4");
    for route in table() {
        assert!(!route.scopes().contains(&SessionScope::Mfa), "{route:?}");
    }
}

/// The tokens that no route but a `Public` one may admit: a deactivated
/// admin's full session, and the `mfa` and `password_change` tokens of two
/// active accounts. The admin's meets every requirement with a full
/// session, so only the scope can refuse it; the one without roles meets
/// none but `AnyActiveUser`, so a guard that let its scope through would
/// answer `403` there instead of `401` (ADR 0007 §5 decides the session
/// first), even where the handler's own scope check would still refuse it.
struct RefusedTokens {
    /// The active admin whose tokens these are, beside [`Self::nobody`].
    admin: backend3::models::users::Model,
    /// A full session of a deactivated admin.
    deactivated: String,
    /// `(whose, token)`: the two-factor step's tokens, which no route of
    /// the table accepts.
    mfa: [(&'static str, String); 2],
    /// `(whose, token)`: a forced password change's sessions, which only
    /// the routes that accept that scope may take.
    password_change: [(&'static str, String); 2],
}

impl RefusedTokens {
    /// Inserts the three invented accounts and signs their tokens.
    ///
    /// - `ctx`: the app context of the test's own database.
    async fn new(ctx: &AppContext) -> Self {
        let inactive = account_with(ctx, "inaktiv@example.test", &[Role::Admin], &[], |new| {
            new.active = ActiveValue::Set(false);
        })
        .await;
        let admin = account_with(ctx, "admin@example.test", &[Role::Admin], &[], |_| {}).await;
        let nobody = account_with(ctx, "senki@example.test", &[], &[], |_| {}).await;
        let both = |scope| {
            [
                ("an admin's", session_token(ctx, &admin, scope)),
                ("a roleless account's", session_token(ctx, &nobody, scope)),
            ]
        };
        Self {
            deactivated: session_token(ctx, &inactive, SessionScope::Admin),
            mfa: both(SessionScope::Mfa),
            password_change: both(SessionScope::PasswordChange),
            admin,
        }
    }
}

/// Asserts that every route of `routes` but the `Public` ones answers
/// `server` the `401` envelope without a session, to a deactivated admin
/// and to an `mfa` token, and, where the route does not accept that scope,
/// to a `password_change` session; each before the handler, its path
/// parameters or its body are read.
///
/// - `server`: the server that mounts `routes`.
/// - `routes`: the route table to walk.
/// - `tokens`: the tokens to send.
async fn assert_every_guarded_route_needs_a_session(
    server: &TestServer,
    routes: Vec<AdminRoute>,
    tokens: &RefusedTokens,
) {
    let guarded: Vec<AdminRoute> = routes
        .into_iter()
        .filter(|route| route.requirement() != Requirement::Public)
        .collect();
    assert!(!guarded.is_empty());
    for route in guarded {
        // A path parameter gets a value; the guard answers before it is
        // read.
        let path = route
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
            .join("/");
        let mut cases = vec![
            ("no session".to_owned(), None),
            (
                "a deactivated admin".to_owned(),
                Some(tokens.deactivated.as_str()),
            ),
        ];
        for (whose, token) in &tokens.mfa {
            cases.push((format!("{whose} mfa token"), Some(token.as_str())));
        }
        // The route's own declaration decides, so this walk checks that the
        // guard honours it; `only_the_own_account_routes_accept_a_password_change_session`
        // checks the declarations against ADR 0006 §4.
        if !route.scopes().contains(&SessionScope::PasswordChange) {
            for (whose, token) in &tokens.password_change {
                cases.push((
                    format!("{whose} password_change session"),
                    Some(token.as_str()),
                ));
            }
        }
        for (case, token) in cases {
            let request = server.method(route.method().clone(), &path);
            let response = match token {
                // From the admin origin, so that a write passes the
                // `Origin` check and reaches the session check
                // (`tests/requests/admin_origin.rs` walks the other origins).
                Some(token) => {
                    request
                        .add_header(COOKIE, format!("{SESSION_COOKIE_NAME}={token}"))
                        .add_header(ORIGIN, ADMIN_ORIGIN)
                        .await
                }
                None => request.await,
            };
            assert_refused(
                &response,
                StatusCode::UNAUTHORIZED,
                &unauthorized(),
                &format!("{} {path}: {case}", route.method()),
            );
        }
    }
}

#[tokio::test]
async fn every_route_but_the_sign_in_steps_needs_a_signed_in_account() {
    // SHR-01: every admin request needs an active, signed-in account
    // (ADR 0006). Walks the application's table, so a route that a later
    // slice adds is checked here without a line of its own.
    with_server(|server, ctx| async move {
        let tokens = RefusedTokens::new(&ctx).await;
        assert_every_guarded_route_needs_a_session(&server, table(), &tokens).await;
        // None of these requests reached a handler: the admin's sessions
        // still hold, so no logout ran either.
        let me = server
            .get(&format!("{ADMIN_API_PREFIX}/auth/me"))
            .add_header(
                COOKIE,
                format!(
                    "{SESSION_COOKIE_NAME}={}",
                    session_token(&ctx, &tokens.admin, SessionScope::Admin)
                ),
            )
            .await;
        assert_eq!(me.status_code(), StatusCode::OK);
    })
    .await;
}

#[tokio::test]
async fn the_walk_over_the_table_refuses_each_scope_a_route_does_not_accept() {
    // The application's table has no route yet that refuses a
    // `password_change` session, so the same walk runs on the probe table,
    // whose routes take every requirement and both scope sets: a guard that
    // opened a route to a scope it does not accept fails here today.
    with_app(|_router, ctx| async move {
        let tokens = RefusedTokens::new(&ctx).await;
        assert_every_guarded_route_needs_a_session(&probe(&ctx), probe_table(), &tokens).await;
    })
    .await;
}

#[test]
fn every_route_of_the_table_reads_a_json_body_at_most() {
    // No upload route exists yet; the first one (the author image) marks
    // itself with `.upload()` and changes this test.
    for route in table() {
        assert_eq!(route.body(), RequestBody::Json, "{route:?}");
    }
}

// --- The guard, on a probe table ---

/// A list that needs `edit posts`, with nothing in it: what a list handler
/// would answer for an empty table, so a guard that let the request through
/// would answer `[]`.
async fn empty_list(_session: AdminSession) -> Json<Value> {
    Json(json!([]))
}

/// The account, scope and grants the handler's session carries.
async fn who(session: AdminSession) -> Json<Value> {
    describe(
        session.user.id,
        &session.claims.pid,
        session.scope,
        &session.grants,
    )
}

/// The same, for a handler that also takes a `password_change` session.
async fn who_own(session: AdminSession<AdminOrPasswordChange>) -> Json<Value> {
    describe(
        session.user.id,
        &session.claims.pid,
        session.scope,
        &session.grants,
    )
}

/// The answer of [`who`] and [`who_own`].
fn describe(id: i64, pid: &str, scope: SessionScope, grants: &AccountGrants) -> Json<Value> {
    Json(json!({
        "id": id,
        "pid": pid,
        "scope": scope.claim(),
        "roles": grants.roles.iter().map(|role| role.name()).collect::<Vec<_>>(),
        "extra": grants.extra.iter().map(|grant| grant.name()).collect::<Vec<_>>(),
    }))
}

/// A handler without a session, as the sign-in is.
async fn public() -> Json<Value> {
    Json(json!({ "public": true }))
}

/// A handler that reads a JSON body as the admin handlers do.
async fn json_body(_session: AdminSession, body: Result<Json<Value>, JsonRejection>) -> Response {
    match body {
        Ok(Json(_)) => Json(json!({ "read": true })).into_response(),
        Err(rejection) => json_rejection_response(&rejection),
    }
}

/// The same without a session, for the upload route's body limit.
async fn public_json_body(body: Result<Json<Value>, JsonRejection>) -> Response {
    match body {
        Ok(Json(_)) => Json(json!({ "read": true })).into_response(),
        Err(rejection) => json_rejection_response(&rejection),
    }
}

/// One route of each kind of requirement, and the mistakes the guard must
/// catch.
fn probe_table() -> Vec<AdminRoute> {
    vec![
        AdminRoute::get(
            "/probe/posts",
            Requirement::Permission(Permission::EditPosts),
            empty_list,
        ),
        AdminRoute::get("/probe/purge", Requirement::AdminRole, who),
        AdminRoute::get("/probe/own", Requirement::AnyActiveUser, who),
        AdminRoute::get(
            "/probe/own-or-new-password",
            Requirement::AnyActiveUser,
            who_own,
        )
        .accepting::<AdminOrPasswordChange>(),
        // The table opens it to `password_change`, the handler does not.
        AdminRoute::get("/probe/stricter-handler", Requirement::AnyActiveUser, who)
            .accepting::<AdminOrPasswordChange>(),
        AdminRoute::get("/probe/public", Requirement::Public, public),
        // A mistake: a `Public` route whose handler wants a session.
        AdminRoute::get("/probe/public-with-session", Requirement::Public, who),
        AdminRoute::post(
            "/probe/labels",
            Requirement::Permission(Permission::EditLabels),
            json_body,
        ),
        AdminRoute::post("/probe/upload", Requirement::Public, public_json_body).upload(),
    ]
}

/// The probe table behind the application's middleware stack, beside a
/// route mounted outside the table (a mistake the table's test catches,
/// and the extractor refuses to serve).
fn probe(ctx: &AppContext) -> TestServer {
    let outside = Routes::new()
        .prefix(ADMIN_API_PREFIX)
        .add("/probe/outside", get(who));
    probe_server(ctx, build(probe_table()).merge(outside))
}

/// `GET path` with `token`, if any, in the session cookie.
async fn get_with(server: &TestServer, path: &str, token: Option<&str>) -> TestResponse {
    let request = server.get(&format!("{ADMIN_API_PREFIX}{path}"));
    match token {
        Some(token) => {
            request
                .add_header(COOKIE, format!("{SESSION_COOKIE_NAME}={token}"))
                .await
        }
        None => request.await,
    }
}

/// The admin envelope of `code` (ADR 0021 §4).
fn envelope(code: &str, description: &str) -> Value {
    json!({ "error": code, "description": description, "field": null, "params": {} })
}

fn unauthorized() -> Value {
    envelope(
        "unauthorized",
        "There is no valid admin session; sign in again",
    )
}

fn forbidden() -> Value {
    envelope(
        "forbidden",
        "The signed-in account may not perform this operation",
    )
}

/// Asserts the status and the envelope of a refusal, with the headers every
/// admin refusal carries and no cookie: a `403` leaves the session as it is.
fn assert_refused(response: &TestResponse, status: StatusCode, body: &Value, case: &str) {
    assert_eq!(response.status_code(), status, "{case}");
    assert_eq!(&response.json::<Value>(), body, "{case}");
    assert_eq!(response.header("cache-control"), "no-store", "{case}");
    assert_eq!(
        response.header("content-type"),
        "application/json",
        "{case}"
    );
    assert!(response.maybe_header(SET_COOKIE).is_none(), "{case}");
}

#[tokio::test]
async fn each_requirement_admits_exactly_the_accounts_adr_0007_names() {
    with_app(|_router, ctx| async move {
        let server = probe(&ctx);
        let nobody = account_with(&ctx, "senki@example.test", &[], &[], |_| {}).await;
        let writer = account_with(&ctx, "iro@example.test", &[Role::Writer], &[], |_| {}).await;
        let secretary =
            account_with(&ctx, "titkar@example.test", &[Role::Secretary], &[], |_| {}).await;
        let granted = account_with(
            &ctx,
            "egyeni@example.test",
            &[],
            &[Permission::EditPosts],
            |_| {},
        )
        .await;
        let every_grant =
            account_with(&ctx, "minden@example.test", &[], &Permission::ALL, |_| {}).await;
        let admin = account_with(&ctx, "admin@example.test", &[Role::Admin], &[], |_| {}).await;

        // (account, may list posts, may purge)
        for (user, posts, purge) in [
            (&nobody, false, false),
            (&writer, true, false),
            (&secretary, false, false),
            (&granted, true, false),
            // ADR 0007 §2: every permission as an extra grant is not the
            // admin role.
            (&every_grant, true, false),
            (&admin, true, true),
        ] {
            let token = session_token(&ctx, user, SessionScope::Admin);
            let case = user.email.clone().unwrap_or_default();

            let list = get_with(&server, "/probe/posts", Some(&token)).await;
            if posts {
                assert_eq!(list.status_code(), StatusCode::OK, "{case}");
                assert_eq!(list.json::<Value>(), json!([]), "{case}");
                assert_eq!(list.header("cache-control"), "no-store", "{case}");
            } else {
                // A list the account may not see is a 403, never `[]`.
                assert_refused(&list, StatusCode::FORBIDDEN, &forbidden(), &case);
            }

            let purge_answer = get_with(&server, "/probe/purge", Some(&token)).await;
            if purge {
                assert_eq!(purge_answer.status_code(), StatusCode::OK, "{case}");
            } else {
                assert_refused(&purge_answer, StatusCode::FORBIDDEN, &forbidden(), &case);
            }

            // Any active user, whatever it holds; the session the handler
            // gets is the account's, with its grants as read.
            let own = get_with(&server, "/probe/own", Some(&token)).await;
            assert_eq!(own.status_code(), StatusCode::OK, "{case}");
            assert_eq!(own.json::<Value>()["id"], json!(user.id), "{case}");
            assert_eq!(
                own.json::<Value>()["pid"],
                json!(user.pid.to_string()),
                "{case}"
            );

            // A public route lets everyone in.
            let open = get_with(&server, "/probe/public", Some(&token)).await;
            assert_eq!(open.status_code(), StatusCode::OK, "{case}");
        }

        // The session carries the roles and grants as stored.
        let token = session_token(&ctx, &granted, SessionScope::Admin);
        assert_eq!(
            get_with(&server, "/probe/own", Some(&token))
                .await
                .json::<Value>(),
            json!({
                "id": granted.id,
                "pid": granted.pid.to_string(),
                "scope": "admin",
                "roles": [],
                "extra": ["edit posts"],
            })
        );
    })
    .await;
}

#[tokio::test]
async fn no_valid_session_is_a_401_before_any_requirement_is_read() {
    with_app(|_router, ctx| async move {
        let server = probe(&ctx);
        // An admin whose session no longer holds: ADR 0007 §5 checks the
        // session first, so these are 401 even where the account would
        // otherwise be refused 403.
        let inactive = account_with(&ctx, "inaktiv@example.test", &[Role::Admin], &[], |new| {
            new.active = ActiveValue::Set(false);
        })
        .await;
        let nobody = account_with(&ctx, "senki@example.test", &[], &[], |_| {}).await;
        let inactive_token = session_token(&ctx, &inactive, SessionScope::Admin);
        // A `password_change` session on a route that accepts only a full
        // one: 401, although the account holds no permission either.
        let password_change = session_token(&ctx, &nobody, SessionScope::PasswordChange);
        // An `mfa` token, which no admin route accepts.
        let mfa = session_token(&ctx, &nobody, SessionScope::Mfa);
        for path in ["/probe/posts", "/probe/purge", "/probe/own"] {
            for (case, token) in [
                ("no token", None),
                ("not a token", Some("nem-token")),
                ("an inactive account", Some(inactive_token.as_str())),
                ("a password_change session", Some(password_change.as_str())),
                ("an mfa token", Some(mfa.as_str())),
            ] {
                let response = get_with(&server, path, token).await;
                assert_refused(
                    &response,
                    StatusCode::UNAUTHORIZED,
                    &unauthorized(),
                    &format!("{path}: {case}"),
                );
            }
        }
        // The public route asks for nothing.
        let response = get_with(&server, "/probe/public", None).await;
        assert_eq!(response.status_code(), StatusCode::OK);
        assert_eq!(response.json::<Value>(), json!({ "public": true }));
        assert_eq!(response.header("cache-control"), "no-store");
    })
    .await;
}

#[tokio::test]
async fn the_guard_decides_before_the_handler_reads_the_body() {
    with_app(|_router, ctx| async move {
        let server = probe(&ctx);
        let nobody = account_with(&ctx, "senki@example.test", &[], &[], |_| {}).await;
        let writer = account_with(&ctx, "iro@example.test", &[Role::Writer], &[], |_| {}).await;
        let nobody_token = session_token(&ctx, &nobody, SessionScope::Admin);
        let writer_token = session_token(&ctx, &writer, SessionScope::Admin);
        let path = format!("{ADMIN_API_PREFIX}/probe/labels");
        let too_large = format!(r#"{{"x":"{}"}}"#, "a".repeat(JSON_BODY_LIMIT));
        for (case, body, content_type) in [
            ("not JSON", "name=x".to_owned(), "application/json"),
            (
                "above the JSON limit",
                too_large.clone(),
                "application/json",
            ),
            ("a valid body", r#"{"x":1}"#.to_owned(), "application/json"),
        ] {
            let send = |token: Option<&str>| {
                let request = server
                    .post(&path)
                    .bytes(body.clone().into())
                    .content_type(content_type);
                match token {
                    // From the admin origin, as the admin UI sends it
                    // (ADR 0006 §3, ADR 0044).
                    Some(token) => request
                        .add_header(COOKIE, format!("{SESSION_COOKIE_NAME}={token}"))
                        .add_header(ORIGIN, ADMIN_ORIGIN),
                    None => request,
                }
            };
            // No session: 401, whatever the body.
            assert_refused(
                &send(None).await,
                StatusCode::UNAUTHORIZED,
                &unauthorized(),
                case,
            );
            // A session without `edit labels`: 403, whatever the body.
            assert_refused(
                &send(Some(&nobody_token)).await,
                StatusCode::FORBIDDEN,
                &forbidden(),
                case,
            );
            // A writer holds `edit labels` (ADR 0007 §2): only now is the
            // body read.
            let answer = send(Some(&writer_token)).await;
            let expected = match case {
                "not JSON" => (StatusCode::UNPROCESSABLE_ENTITY, "invalid_body"),
                "above the JSON limit" => (StatusCode::PAYLOAD_TOO_LARGE, "body_too_large"),
                _ => (StatusCode::OK, ""),
            };
            assert_eq!(answer.status_code(), expected.0, "{case}");
            if expected.0 == StatusCode::OK {
                assert_eq!(answer.json::<Value>(), json!({ "read": true }), "{case}");
            } else {
                assert_eq!(answer.json::<Value>()["error"], json!(expected.1), "{case}");
            }
        }
        // An upload route keeps the 128 MiB request limit, so the same body
        // that a JSON route refuses is read there (ADR 0015 §6).
        let upload = server
            .post(&format!("{ADMIN_API_PREFIX}/probe/upload"))
            .bytes(too_large.into())
            .content_type("application/json")
            .await;
        assert_eq!(upload.status_code(), StatusCode::OK);
        assert_eq!(upload.json::<Value>(), json!({ "read": true }));
        assert_eq!(upload.header("cache-control"), "no-store");
    })
    .await;
}

#[tokio::test]
async fn a_role_or_grant_change_applies_on_the_next_request() {
    // ADR 0007 §5, "Fresh data": the same token, before and after.
    with_app(|_router, ctx| async move {
        let server = probe(&ctx);
        let user = account_with(&ctx, "valtozo@example.test", &[], &[], |_| {}).await;
        let token = session_token(&ctx, &user, SessionScope::Admin);
        let list = || get_with(&server, "/probe/posts", Some(&token));
        assert_eq!(list().await.status_code(), StatusCode::FORBIDDEN);

        give(&ctx, &user, &[Role::Supervisor], &[]).await;
        assert_eq!(list().await.json::<Value>(), json!([]));

        take_all(&ctx, &user).await;
        assert_eq!(list().await.status_code(), StatusCode::FORBIDDEN);

        give(&ctx, &user, &[], &[Permission::EditPosts]).await;
        assert_eq!(list().await.status_code(), StatusCode::OK);
        // A 403 in between ended nothing: the session still opens the
        // account's own routes.
        assert_eq!(
            get_with(&server, "/probe/own", Some(&token))
                .await
                .status_code(),
            StatusCode::OK
        );
    })
    .await;
}

#[tokio::test]
async fn the_route_table_and_the_handler_both_decide_the_scope() {
    with_app(|_router, ctx| async move {
        let server = probe(&ctx);
        let user = account_with(&ctx, "uj-jelszo@example.test", &[], &[], |_| {}).await;
        let full = session_token(&ctx, &user, SessionScope::Admin);
        let password_change = session_token(&ctx, &user, SessionScope::PasswordChange);

        // Opened to `password_change` by the table and by the handler.
        let own = get_with(
            &server,
            "/probe/own-or-new-password",
            Some(&password_change),
        )
        .await;
        assert_eq!(own.status_code(), StatusCode::OK);
        assert_eq!(own.json::<Value>()["scope"], json!("password_change"));
        let own = get_with(&server, "/probe/own-or-new-password", Some(&full)).await;
        assert_eq!(own.json::<Value>()["scope"], json!("admin"));

        // Opened by the table only: the stricter handler refuses it.
        assert_refused(
            &get_with(&server, "/probe/stricter-handler", Some(&password_change)).await,
            StatusCode::UNAUTHORIZED,
            &unauthorized(),
            "stricter handler",
        );
        assert_eq!(
            get_with(&server, "/probe/stricter-handler", Some(&full))
                .await
                .status_code(),
            StatusCode::OK
        );
    })
    .await;
}

#[tokio::test]
async fn a_handler_gets_a_session_only_from_the_guard() {
    // A handler that takes the session on a `Public` route, or on a route
    // mounted outside the table, never serves: there is no guarded session
    // to hand it, and the extractor never opens one itself.
    with_app(|_router, ctx| async move {
        let server = probe(&ctx);
        let admin = account_with(&ctx, "admin@example.test", &[Role::Admin], &[], |_| {}).await;
        let token = session_token(&ctx, &admin, SessionScope::Admin);
        for path in ["/probe/public-with-session", "/probe/outside"] {
            for token in [None, Some(token.as_str())] {
                let response = get_with(&server, path, token).await;
                assert_eq!(
                    response.status_code(),
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "{path} {token:?}"
                );
                assert_eq!(
                    response.json::<Value>()["error"],
                    json!("internal_server_error"),
                    "{path}"
                );
            }
        }
    })
    .await;
}

#[tokio::test]
async fn the_application_s_own_routes_keep_their_answers() {
    // The auth routes of the table: the two sign-ins still need no
    // session, and `me` and logout still answer 401 without one.
    with_server(|server, _ctx| async move {
        for path in ["/auth/login", "/auth/token"] {
            let sign_in = server
                .post(&format!("{ADMIN_API_PREFIX}{path}"))
                .json(&json!({ "email": "senki@example.test", "password": "rossz jelszó" }))
                .await;
            assert_eq!(sign_in.status_code(), StatusCode::UNAUTHORIZED, "{path}");
            assert_eq!(
                sign_in.json::<Value>()["error"],
                json!("invalid_credentials"),
                "{path}"
            );
        }
        for (method, path) in [(Method::GET, "/auth/me"), (Method::POST, "/auth/logout")] {
            let response = server
                .method(method.clone(), &format!("{ADMIN_API_PREFIX}{path}"))
                .await;
            assert_refused(
                &response,
                StatusCode::UNAUTHORIZED,
                &unauthorized(),
                &format!("{method} {path}"),
            );
        }
    })
    .await;
}
