//! The role matrix of ADR 0007 over the admin route table
//! (`tests/support/authz_matrix.rs`; ADR 0007 §2, §5; parity RBAC-05).
//!
//! The application's table: every route, called as each of the 37 matrix
//! accounts with a full session, answers exactly as its requirement says
//! for that account. The table is read at run time, so every route that a
//! later slice adds is called here without a line of its own, and the
//! ADR 0007 §5 test of the user editor (every permission except
//! `edit users` gets `403` on each user, role and grant change) holds as
//! soon as those routes exist.
//!
//! The harness itself, on probe tables served behind the application's
//! middleware stack: the accounts and what each is expected to meet, how
//! an answer is read, that it finds a guard that admits or refuses other
//! accounts than the route declares, that the user-editor rule refuses a
//! write route open to that account, that a handler which ends the
//! account's sessions does not end the walk, that every write carries the
//! admin origin and no read does, that a public route's own `401` is no
//! finding, and that a server error to an admitted account is one. Every
//! account is invented.

use std::collections::BTreeSet;

use axum::{
    Json,
    http::{HeaderMap, HeaderValue, Method, StatusCode, header::ORIGIN},
};
use backend3::{
    controllers::admin::{
        auth,
        routes::{AdminRoute, build, table},
    },
    domain::authz::{AccountGrants, Permission, Requirement, Role},
};
use serde_json::{Value, json};

use crate::support::{
    ADMIN_ORIGIN,
    admin_probe::probe_server,
    authz_matrix::{
        ABSENT_ID, MatrixCell, MatrixSubject, Outcome, USER_EDITOR_PATH, cell_findings, fill_path,
        is_user_editor_write, is_write, run,
    },
    with_app, with_server,
};

/// The number of accounts the matrix calls each route as.
const SUBJECTS: usize = 37;

#[tokio::test]
async fn every_route_of_the_admin_route_table_answers_as_the_role_matrix_says() {
    with_server(|server, ctx| async move {
        let routes = table();
        let report = run(&server, &ctx, &routes).await;
        assert!(
            report.findings.is_empty(),
            "the admin routes and the role matrix of ADR 0007 disagree:\n{}",
            report.findings.join("\n")
        );
        // Every route as every account, so the walk never passes empty.
        assert!(!routes.is_empty());
        assert_eq!(report.cells.len(), routes.len() * SUBJECTS);
        // `/auth/me` serves every signed-in account, whatever it holds.
        for subject in MatrixSubject::all() {
            assert_eq!(
                report.outcome(&Method::GET, "/api/admin/auth/me", subject),
                Some(Outcome::Admitted(StatusCode::OK)),
                "{subject}"
            );
        }
    })
    .await;
}

#[test]
fn the_matrix_holds_each_role_no_role_each_grant_and_each_all_but_one_set() {
    let subjects = MatrixSubject::all();
    assert_eq!(subjects.len(), SUBJECTS);
    // Four roles, alone.
    for role in Role::ALL {
        let subject = MatrixSubject::Role(role);
        assert!(subjects.contains(&subject), "{subject}");
        assert_eq!(subject.roles(), [role]);
        assert_eq!(subject.extra(), []);
    }
    assert!(subjects.contains(&MatrixSubject::Nothing));
    assert_eq!(MatrixSubject::Nothing.roles(), []);
    assert_eq!(MatrixSubject::Nothing.extra(), []);
    for permission in Permission::ALL {
        let single = MatrixSubject::Grant(permission);
        assert!(subjects.contains(&single), "{single}");
        assert_eq!(single.roles(), []);
        assert_eq!(single.extra(), [permission]);
        // Every permission but one, and never the admin role, which no
        // extra grant gives (ADR 0007 §2).
        let all_but = MatrixSubject::AllBut(permission);
        assert!(subjects.contains(&all_but), "{all_but}");
        assert_eq!(all_but.roles(), []);
        let extra: BTreeSet<Permission> = all_but.extra().into_iter().collect();
        assert_eq!(extra.len(), Permission::ALL.len() - 1, "{all_but}");
        assert!(!extra.contains(&permission), "{all_but}");
    }
    // One invented address per account, so the walk can insert them all.
    let emails: BTreeSet<String> = subjects.iter().map(|subject| subject.email()).collect();
    assert_eq!(emails.len(), SUBJECTS);
    assert!(emails.iter().all(|email| email.ends_with("@example.test")));
    assert_eq!(
        MatrixSubject::AllBut(Permission::EditSiteSettings).email(),
        "matrix-all-but-edit-site-settings@example.test"
    );
    assert_eq!(
        MatrixSubject::AllBut(Permission::EditUsers).to_string(),
        r#"every permission but "edit users""#
    );
    assert_eq!(
        MatrixSubject::Role(Role::Writer).to_string(),
        "the writer role"
    );
    assert_eq!(
        MatrixSubject::Grant(Permission::ViewLogs).to_string(),
        r#"the single grant "view logs""#
    );
    assert_eq!(MatrixSubject::Nothing.to_string(), "no role and no grant");
}

/// Every requirement a route can declare.
fn requirements() -> Vec<Requirement> {
    [
        Requirement::Public,
        Requirement::AnyActiveUser,
        Requirement::AdminRole,
    ]
    .into_iter()
    .chain(Permission::ALL.into_iter().map(Requirement::Permission))
    .collect()
}

#[test]
fn what_the_matrix_expects_follows_adr_0007() {
    use MatrixSubject::{AllBut, Grant, Nothing};
    let edit = |permission| Requirement::Permission(permission);
    // ADR 0007 §2: a writer edits posts but does not publish them; a
    // supervisor does both; a secretary has the canteen, not the posts.
    assert!(MatrixSubject::Role(Role::Writer).admits(edit(Permission::EditPosts)));
    assert!(!MatrixSubject::Role(Role::Writer).admits(edit(Permission::PublishPosts)));
    assert!(MatrixSubject::Role(Role::Supervisor).admits(edit(Permission::PublishPosts)));
    assert!(MatrixSubject::Role(Role::Secretary).admits(edit(Permission::EditCanteens)));
    assert!(!MatrixSubject::Role(Role::Secretary).admits(edit(Permission::EditPosts)));
    // Only the admin role meets the admin role; no grant gives it.
    assert!(MatrixSubject::Role(Role::Admin).admits(Requirement::AdminRole));
    for permission in Permission::ALL {
        assert!(!Grant(permission).admits(Requirement::AdminRole));
        assert!(!AllBut(permission).admits(Requirement::AdminRole));
    }
    // The all-but-one sets meet every permission but the one left out.
    assert!(!AllBut(Permission::EditUsers).admits(edit(Permission::EditUsers)));
    assert!(AllBut(Permission::EditUsers).admits(edit(Permission::ViewLogs)));
    // No role and no grant: only the own account and the public routes.
    assert!(Nothing.admits(Requirement::AnyActiveUser));
    assert!(Nothing.admits(Requirement::Public));
    // The expectation, computed from the account's definition, agrees with
    // the guard's own check on every pair; the matrix walk then checks the
    // guard's whole path (grants stored, read, checked and answered).
    for subject in MatrixSubject::all() {
        let grants = AccountGrants {
            roles: subject.roles().into_iter().collect(),
            extra: subject.extra().into_iter().collect(),
        };
        for requirement in requirements() {
            assert_eq!(
                subject.admits(requirement),
                requirement.is_met_by(&grants),
                "{subject} / {requirement}"
            );
        }
    }
}

#[test]
fn every_path_parameter_gets_an_id_no_row_has() {
    assert_eq!(ABSENT_ID.parse::<i32>(), Ok(i32::MAX));
    assert_eq!(
        fill_path("/api/admin/users/{id}/roles"),
        format!("/api/admin/users/{ABSENT_ID}/roles")
    );
    assert_eq!(
        fill_path("/api/admin/canteen-weeks/{isoYear}/{isoWeek}"),
        format!("/api/admin/canteen-weeks/{ABSENT_ID}/{ABSENT_ID}")
    );
    assert_eq!(
        fill_path("/api/admin/media/{*path}"),
        format!("/api/admin/media/{ABSENT_ID}")
    );
    assert_eq!(fill_path("/api/admin/auth/me"), "/api/admin/auth/me");
}

#[test]
fn every_method_but_the_safe_ones_writes() {
    let extension = Method::from_bytes(b"PURGE").expect("an extension method");
    for method in [
        Method::POST,
        Method::PUT,
        Method::PATCH,
        Method::DELETE,
        Method::CONNECT,
        extension,
    ] {
        assert!(is_write(&method), "{method}");
    }
    // RFC 9110 §9.2.1's safe methods, which the guard's `Origin` check
    // leaves alone (ADR 0006 §3, ADR 0044).
    for method in [Method::GET, Method::HEAD, Method::OPTIONS, Method::TRACE] {
        assert!(!is_write(&method), "{method}");
    }
}

#[test]
fn the_user_editor_rule_covers_every_write_under_the_users_path() {
    assert_eq!(USER_EDITOR_PATH, "/api/admin/users");
    // Creating, updating, deleting, restoring, and the role and grant
    // changes (ADR 0007 §5), and the account actions below the path.
    for (method, path) in [
        (Method::POST, "/api/admin/users"),
        (Method::PUT, "/api/admin/users/{id}"),
        (Method::PATCH, "/api/admin/users/{id}"),
        (Method::DELETE, "/api/admin/users/{id}"),
        (Method::POST, "/api/admin/users/{id}/restore"),
        (Method::PUT, "/api/admin/users/{id}/roles"),
        (Method::PUT, "/api/admin/users/{id}/permissions"),
        (Method::POST, "/api/admin/users/{id}/purge"),
    ] {
        assert!(is_user_editor_write(&method, path), "{method} {path}");
    }
    // Reads are not §5's operations, and other paths are not the editor's.
    for (method, path) in [
        (Method::GET, "/api/admin/users"),
        (Method::HEAD, "/api/admin/users/{id}"),
        (Method::OPTIONS, "/api/admin/users/{id}"),
        (Method::POST, "/api/admin/users-export"),
        (Method::POST, "/api/admin/auth/logout"),
        (Method::PUT, "/api/admin/roles"),
    ] {
        assert!(!is_user_editor_write(&method, path), "{method} {path}");
    }
}

#[test]
fn only_the_forbidden_envelope_reads_as_the_guard_s_refusal() {
    let forbidden = br#"{"error":"forbidden","description":"x","field":null,"params":{}}"#;
    assert_eq!(
        Outcome::of(StatusCode::FORBIDDEN, forbidden),
        Outcome::Refused
    );
    assert_eq!(
        Outcome::of(StatusCode::UNAUTHORIZED, b""),
        Outcome::Unauthorized
    );
    // A record-level rule answers 403 with a code of its own: the handler
    // ran, so the guard admitted the account.
    let record_rule =
        br#"{"error":"post.published_locked","description":"x","field":null,"params":{}}"#;
    assert_eq!(
        Outcome::of(StatusCode::FORBIDDEN, record_rule),
        Outcome::Admitted(StatusCode::FORBIDDEN)
    );
    assert_eq!(
        Outcome::of(StatusCode::FORBIDDEN, b"not json"),
        Outcome::Admitted(StatusCode::FORBIDDEN)
    );
    // The guard's `Origin` check (ADR 0006 §3, ADR 0044), which answers
    // before the session is opened, is not the requirement's refusal
    // either: read as it, it would let a request that never reached the
    // requirement pass for an account the route must refuse.
    let origin = br#"{"error":"origin_mismatch","description":"x","field":null,"params":{}}"#;
    assert_eq!(
        Outcome::of(StatusCode::FORBIDDEN, origin),
        Outcome::Admitted(StatusCode::FORBIDDEN)
    );
    for status in [
        StatusCode::OK,
        StatusCode::NO_CONTENT,
        StatusCode::NOT_FOUND,
        StatusCode::UNPROCESSABLE_ENTITY,
    ] {
        assert_eq!(Outcome::of(status, forbidden), Outcome::Admitted(status));
    }
}

/// One cell of the matrix, for the tests of [`cell_findings`].
///
/// - `method`, `path`: the route.
/// - `subject`: the account.
/// - `outcome`: how the server answered.
fn cell(method: Method, path: &str, subject: MatrixSubject, outcome: Outcome) -> MatrixCell {
    MatrixCell {
        method,
        path: path.to_owned(),
        subject,
        outcome,
    }
}

#[test]
fn each_disagreement_of_a_cell_is_named() {
    let edit_users = Requirement::Permission(Permission::EditUsers);
    let all_but_users = MatrixSubject::AllBut(Permission::EditUsers);
    // Agreeing cells.
    for (cell, requirement) in [
        (
            cell(
                Method::GET,
                "/api/admin/x",
                MatrixSubject::Nothing,
                Outcome::Refused,
            ),
            edit_users,
        ),
        (
            cell(
                Method::POST,
                "/api/admin/users",
                all_but_users,
                Outcome::Refused,
            ),
            edit_users,
        ),
        (
            cell(
                Method::GET,
                "/api/admin/x",
                MatrixSubject::Role(Role::Admin),
                Outcome::Admitted(StatusCode::NOT_FOUND),
            ),
            Requirement::AdminRole,
        ),
    ] {
        assert_eq!(cell_findings(&cell, requirement), Vec::<String>::new());
    }
    // A session taken for none.
    let findings = cell_findings(
        &cell(
            Method::GET,
            "/api/admin/x",
            MatrixSubject::Nothing,
            Outcome::Unauthorized,
        ),
        Requirement::AnyActiveUser,
    );
    assert_eq!(findings.len(), 1, "{findings:?}");
    assert!(findings[0].contains("valid full session of no role and no grant was answered 401"));
    // Refused although it meets the requirement.
    let findings = cell_findings(
        &cell(
            Method::GET,
            "/api/admin/x",
            MatrixSubject::Grant(Permission::EditUsers),
            Outcome::Refused,
        ),
        edit_users,
    );
    assert!(
        findings[0].contains(r#"meets the permission "edit users""#),
        "{findings:?}"
    );
    // Admitted although it does not.
    let findings = cell_findings(
        &cell(
            Method::DELETE,
            "/api/admin/x/{id}",
            MatrixSubject::Grant(Permission::EditPosts),
            Outcome::Admitted(StatusCode::NOT_FOUND),
        ),
        edit_users,
    );
    assert!(
        findings[0].contains("does not meet the permission \"edit users\"")
            && findings[0].contains("404 Not Found from the handler instead of 403 forbidden"),
        "{findings:?}"
    );
    // A user-editor write open to every permission but `edit users`: the
    // route's declaration agrees with the answer, ADR 0007 §5 does not.
    let findings = cell_findings(
        &cell(
            Method::PUT,
            "/api/admin/users/{id}/roles",
            all_but_users,
            Outcome::Admitted(StatusCode::NOT_FOUND),
        ),
        Requirement::Permission(Permission::ViewLogs),
    );
    assert_eq!(findings.len(), 1, "{findings:?}");
    assert!(findings[0].starts_with("PUT /api/admin/users/{id}/roles: ADR 0007 §5"));
}

#[test]
fn a_public_route_s_401_and_a_server_error_are_read_by_the_requirement() {
    // A public route's own `401`: the guard opens no session there, so a
    // sign-in step that reads a cookie of its own (ADR 0006 §4's two-factor
    // step) may answer it to a request without that cookie.
    let public_step = cell(
        Method::POST,
        "/api/admin/auth/totp",
        MatrixSubject::Role(Role::Writer),
        Outcome::Unauthorized,
    );
    assert_eq!(
        cell_findings(&public_step, Requirement::Public),
        Vec::<String>::new()
    );
    // The same answer on a route that needs a session is the guard's: the
    // valid full session was taken for none.
    assert_eq!(
        cell_findings(&public_step, Requirement::AnyActiveUser).len(),
        1
    );
    // A server error to an account the route admits: the walk has not
    // shown that the route serves it.
    let findings = cell_findings(
        &cell(
            Method::GET,
            "/api/admin/x",
            MatrixSubject::Role(Role::Admin),
            Outcome::Admitted(StatusCode::INTERNAL_SERVER_ERROR),
        ),
        Requirement::AdminRole,
    );
    assert_eq!(findings.len(), 1, "{findings:?}");
    assert!(
        findings[0].starts_with(
            "GET /api/admin/x: the admin role meets the admin role, which the route declares, \
             but was answered 500 Internal Server Error from the handler: a server error"
        ),
        "{findings:?}"
    );
    // To an account it must refuse, the one finding is the missing refusal.
    let findings = cell_findings(
        &cell(
            Method::GET,
            "/api/admin/x",
            MatrixSubject::Nothing,
            Outcome::Admitted(StatusCode::SERVICE_UNAVAILABLE),
        ),
        Requirement::AdminRole,
    );
    assert_eq!(findings.len(), 1, "{findings:?}");
    assert!(
        findings[0].contains("does not meet the admin role"),
        "{findings:?}"
    );
}

/// A probe handler: the guard's decision is all these tests read.
async fn probe() {}

/// One `GET` route per requirement a route can declare, after a logout
/// route that ends the account's sessions: the walk must sign a new token
/// after it, or every later route would answer `401`.
fn every_requirement() -> Vec<AdminRoute> {
    let mut routes = vec![AdminRoute::post(
        "/probe/logout",
        Requirement::AnyActiveUser,
        auth::logout,
    )];
    routes.push(AdminRoute::get("/probe/public", Requirement::Public, probe));
    routes.push(AdminRoute::get(
        "/probe/own",
        Requirement::AnyActiveUser,
        probe,
    ));
    routes.push(AdminRoute::get(
        "/probe/admin-role",
        Requirement::AdminRole,
        probe,
    ));
    for permission in Permission::ALL {
        // A leaked `&'static str` per permission: AdminRoute keeps its path
        // for the life of the router, as the table's literals live.
        let path: &'static str =
            Box::leak(format!("/probe/{}", permission.name().replace(' ', "-")).into_boxed_str());
        routes.push(AdminRoute::get(
            path,
            Requirement::Permission(permission),
            probe,
        ));
    }
    routes
}

#[tokio::test]
async fn each_requirement_admits_exactly_the_matrix_accounts_it_names() {
    with_app(|_router, ctx| async move {
        let server = probe_server(&ctx, build(every_requirement()));
        let routes = every_requirement();
        let report = run(&server, &ctx, &routes).await;
        assert!(report.findings.is_empty(), "{}", report.findings.join("\n"));
        assert_eq!(report.cells.len(), routes.len() * SUBJECTS);

        let at = |path: &str, subject| report.outcome(&Method::GET, path, subject);
        let ok = Some(Outcome::Admitted(StatusCode::OK));
        let refused = Some(Outcome::Refused);
        for subject in MatrixSubject::all() {
            // The logout ran for every account, and the walk went on.
            assert_eq!(
                report.outcome(&Method::POST, "/api/admin/probe/logout", subject),
                Some(Outcome::Admitted(StatusCode::NO_CONTENT)),
                "{subject}"
            );
            assert_eq!(at("/api/admin/probe/public", subject), ok, "{subject}");
            assert_eq!(at("/api/admin/probe/own", subject), ok, "{subject}");
        }
        // The admin role: the admin alone, never every grant but one.
        assert_eq!(
            at(
                "/api/admin/probe/admin-role",
                MatrixSubject::Role(Role::Admin)
            ),
            ok
        );
        assert_eq!(
            at(
                "/api/admin/probe/admin-role",
                MatrixSubject::AllBut(Permission::EditPosts)
            ),
            refused
        );
        // ADR 0007 §5's account: every permission but `edit users` is
        // refused the user editor's permission, and only that one.
        let all_but_users = MatrixSubject::AllBut(Permission::EditUsers);
        assert_eq!(at("/api/admin/probe/edit-users", all_but_users), refused);
        assert_eq!(at("/api/admin/probe/edit-posts", all_but_users), ok);
        assert_eq!(at("/api/admin/probe/view-logs", all_but_users), ok);
        // A single grant opens its own route only.
        let diaktoll = MatrixSubject::Grant(Permission::EditDiaktoll);
        assert_eq!(at("/api/admin/probe/edit-diaktoll", diaktoll), ok);
        assert_eq!(at("/api/admin/probe/edit-banners", diaktoll), refused);
        // No role and no grant: refused every permission.
        for permission in Permission::ALL {
            let path = format!("/api/admin/probe/{}", permission.name().replace(' ', "-"));
            assert_eq!(at(&path, MatrixSubject::Nothing), refused, "{path}");
        }
    })
    .await;
}

/// What a probe server enforces, beside the declarations of
/// [`misdeclared`] at the same paths.
fn enforced() -> Vec<AdminRoute> {
    vec![
        AdminRoute::get(
            "/probe/x",
            Requirement::Permission(Permission::EditPosts),
            probe,
        ),
        AdminRoute::get("/probe/y", Requirement::AdminRole, probe),
        AdminRoute::get("/probe/z", Requirement::AnyActiveUser, probe),
    ]
}

/// Declarations that differ from what [`enforced`] mounts: a guard that
/// checks something other than the route table says.
fn misdeclared() -> Vec<AdminRoute> {
    vec![
        AdminRoute::get(
            "/probe/x",
            Requirement::Permission(Permission::EditLabels),
            probe,
        ),
        AdminRoute::get(
            "/probe/y",
            Requirement::Permission(Permission::EditUsers),
            probe,
        ),
        AdminRoute::get(
            "/probe/z",
            Requirement::Permission(Permission::ViewLogs),
            probe,
        ),
    ]
}

#[tokio::test]
async fn the_matrix_names_every_account_a_guard_answers_otherwise_than_declared() {
    with_app(|_router, ctx| async move {
        let server = probe_server(&ctx, build(enforced()));
        let report = run(&server, &ctx, &misdeclared()).await;
        for expected in [
            // Lets in a permission it does not name.
            r#"GET /api/admin/probe/x: the single grant "edit posts" does not meet the permission "edit labels""#,
            // Refuses a permission it names.
            r#"GET /api/admin/probe/x: the single grant "edit labels" meets the permission "edit labels", which the route declares, but was answered 403 forbidden"#,
            // Asks for the admin role behind a permission: every grant but
            // one holds `edit users` and is refused.
            r#"GET /api/admin/probe/y: every permission but "edit posts" meets the permission "edit users""#,
            // Lets in every signed-in account behind a permission.
            r#"GET /api/admin/probe/z: no role and no grant does not meet the permission "view logs""#,
        ] {
            assert!(
                report
                    .findings
                    .iter()
                    .any(|finding| finding.starts_with(expected)),
                "{expected}\nin:\n{}",
                report.findings.join("\n")
            );
        }
        // Every finding is about one of the three routes, none about a
        // session: the walk reached the guard every time.
        assert!(
            report
                .findings
                .iter()
                .all(|finding| finding.starts_with("GET /api/admin/probe/")
                    && !finding.contains("401")),
            "{}",
            report.findings.join("\n")
        );
    })
    .await;
}

/// The user editor as the users slices will mount it, each write behind
/// `edit users` (and the purge behind the admin role, ADR 0007 §2).
fn user_editor() -> Vec<AdminRoute> {
    let edit_users = Requirement::Permission(Permission::EditUsers);
    vec![
        AdminRoute::get("/users", edit_users, probe),
        AdminRoute::post("/users", edit_users, probe),
        AdminRoute::put("/users/{id}", edit_users, probe),
        AdminRoute::delete("/users/{id}", edit_users, probe),
        AdminRoute::post("/users/{id}/restore", edit_users, probe),
        AdminRoute::put("/users/{id}/roles", edit_users, probe),
        AdminRoute::put("/users/{id}/permissions", edit_users, probe),
        AdminRoute::post("/users/{id}/purge", Requirement::AdminRole, probe),
    ]
}

#[tokio::test]
async fn every_permission_but_edit_users_is_refused_every_user_editor_write() {
    // ADR 0007 §5: "A test asserts that a user holding every permission
    // except `edit users` gets 403 on each of these operations."
    with_app(|_router, ctx| async move {
        let server = probe_server(&ctx, build(user_editor()));
        let routes = user_editor();
        let report = run(&server, &ctx, &routes).await;
        assert!(report.findings.is_empty(), "{}", report.findings.join("\n"));
        let all_but_users = MatrixSubject::AllBut(Permission::EditUsers);
        for route in &routes {
            assert_eq!(
                report.outcome(route.method(), &route.path(), all_but_users),
                Some(Outcome::Refused),
                "{route:?}"
            );
        }
    })
    .await;
}

/// A user editor whose role and grant changes and whose create are opened
/// to other permissions, consistently in the declaration and the guard.
fn open_user_editor() -> Vec<AdminRoute> {
    vec![
        // A read is not one of §5's operations: its own requirement
        // decides it.
        AdminRoute::get(
            "/users",
            Requirement::Permission(Permission::ViewLogs),
            probe,
        ),
        AdminRoute::post(
            "/users",
            Requirement::Permission(Permission::ViewLogs),
            probe,
        ),
        AdminRoute::put("/users/{id}/roles", Requirement::AnyActiveUser, probe),
    ]
}

#[tokio::test]
async fn the_user_editor_rule_holds_whatever_the_route_declares() {
    with_app(|_router, ctx| async move {
        let server = probe_server(&ctx, build(open_user_editor()));
        let report = run(&server, &ctx, &open_user_editor()).await;
        let section_5: Vec<&String> = report
            .findings
            .iter()
            .filter(|finding| finding.contains("ADR 0007 §5"))
            .collect();
        assert_eq!(section_5.len(), 2, "{}", report.findings.join("\n"));
        assert!(section_5[0].starts_with("POST /api/admin/users: ADR 0007 §5"));
        assert!(section_5[1].starts_with("PUT /api/admin/users/{id}/roles: ADR 0007 §5"));
        // The routes do what they declare, so §5 is the only finding.
        assert_eq!(report.findings.len(), 2, "{}", report.findings.join("\n"));
    })
    .await;
}

/// A probe handler that answers `204` only when the request carries the
/// `Origin` that a browser at the admin sends: the admin origin on a
/// write, none on a read; `400` otherwise.
///
/// - `method`: the request's method.
/// - `headers`: the request's headers.
async fn origin_probe(method: Method, headers: HeaderMap) -> StatusCode {
    let origins: Vec<&[u8]> = headers
        .get_all(ORIGIN)
        .iter()
        .map(HeaderValue::as_bytes)
        .collect();
    let expected: Vec<&[u8]> = if is_write(&method) {
        vec![ADMIN_ORIGIN.as_bytes()]
    } else {
        Vec::new()
    };
    if origins == expected {
        StatusCode::NO_CONTENT
    } else {
        StatusCode::BAD_REQUEST
    }
}

/// A read and the four writes on one path, open to every signed-in
/// account, behind [`origin_probe`].
fn origin_routes() -> Vec<AdminRoute> {
    vec![
        AdminRoute::get("/probe/origin", Requirement::AnyActiveUser, origin_probe),
        AdminRoute::post("/probe/origin", Requirement::AnyActiveUser, origin_probe),
        AdminRoute::put("/probe/origin", Requirement::AnyActiveUser, origin_probe),
        AdminRoute::patch("/probe/origin", Requirement::AnyActiveUser, origin_probe),
        AdminRoute::delete("/probe/origin", Requirement::AnyActiveUser, origin_probe),
    ]
}

#[tokio::test]
async fn every_write_of_the_matrix_carries_the_admin_origin_and_no_read_does() {
    // The guard refuses a cookie-authenticated write from any other
    // origin before it opens the session (ADR 0006 §3, ADR 0044), so a
    // write without the admin origin would never reach its route's
    // requirement.
    with_app(|_router, ctx| async move {
        let server = probe_server(&ctx, build(origin_routes()));
        let routes = origin_routes();
        let report = run(&server, &ctx, &routes).await;
        assert!(report.findings.is_empty(), "{}", report.findings.join("\n"));
        assert_eq!(report.cells.len(), routes.len() * SUBJECTS);
        for cell in &report.cells {
            assert_eq!(
                cell.outcome,
                Outcome::Admitted(StatusCode::NO_CONTENT),
                "{} {} as {}",
                cell.method,
                cell.path,
                cell.subject
            );
        }
    })
    .await;
}

/// A sign-in step that reads a cookie of its own, as ADR 0006 §4's
/// two-factor step does: the matrix sends only the session cookie, so it
/// answers `401`.
async fn own_cookie_step() -> (StatusCode, Json<Value>) {
    (
        StatusCode::UNAUTHORIZED,
        Json(json!({
            "error": "unauthorized",
            "description": "probe: no sign-in step cookie",
            "field": null,
            "params": {},
        })),
    )
}

#[tokio::test]
async fn a_public_route_s_own_401_is_no_finding() {
    with_app(|_router, ctx| async move {
        let routes = || {
            vec![
                AdminRoute::post("/probe/step", Requirement::Public, own_cookie_step),
                // After it, a route that needs the session: the walk goes on
                // with a valid token.
                AdminRoute::get("/probe/own", Requirement::AnyActiveUser, probe),
            ]
        };
        let server = probe_server(&ctx, build(routes()));
        let report = run(&server, &ctx, &routes()).await;
        assert!(report.findings.is_empty(), "{}", report.findings.join("\n"));
        for subject in MatrixSubject::all() {
            assert_eq!(
                report.outcome(&Method::POST, "/api/admin/probe/step", subject),
                Some(Outcome::Unauthorized),
                "{subject}"
            );
            assert_eq!(
                report.outcome(&Method::GET, "/api/admin/probe/own", subject),
                Some(Outcome::Admitted(StatusCode::OK)),
                "{subject}"
            );
        }
    })
    .await;
}

/// A handler that fails.
async fn failing() -> StatusCode {
    StatusCode::INTERNAL_SERVER_ERROR
}

#[tokio::test]
async fn a_server_error_to_an_admitted_account_is_a_finding() {
    with_app(|_router, ctx| async move {
        let view_logs = Requirement::Permission(Permission::ViewLogs);
        let routes = || vec![AdminRoute::get("/probe/failing", view_logs, failing)];
        let server = probe_server(&ctx, build(routes()));
        let report = run(&server, &ctx, &routes()).await;
        // One finding per account the route admits; the guard refuses the
        // others before the handler fails.
        let admitted: Vec<MatrixSubject> = MatrixSubject::all()
            .into_iter()
            .filter(|subject| subject.admits(view_logs))
            .collect();
        // `view logs` is held by at least the admin role.
        assert_ne!(admitted.len(), 0);
        assert_eq!(
            report.findings.len(),
            admitted.len(),
            "{}",
            report.findings.join("\n")
        );
        for subject in admitted {
            let expected = format!(
                "GET /api/admin/probe/failing: {subject} meets the permission \"view logs\", \
                 which the route declares, but was answered 500 Internal Server Error from the \
                 handler: a server error"
            );
            assert!(
                report
                    .findings
                    .iter()
                    .any(|finding| finding.starts_with(&expected)),
                "{expected}\nin:\n{}",
                report.findings.join("\n")
            );
        }
    })
    .await;
}
