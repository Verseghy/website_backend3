//! The role matrix harness: every admin route, called as every kind of
//! account ADR 0007 names, against what its requirement says (ADR 0007 §5
//! and its follow-up "A matrix integration test calls every admin route as
//! each role, as a user with no role, and as users with single extra
//! grants").
//!
//! Why it exists: the route table declares one [`Requirement`] per route,
//! and its guard enforces it. The unit tests of `domain::authz` hold the
//! requirement to the role matrix of ADR 0007 §2, and
//! `tests/requests/admin_routes.rs` drives the guard on a probe table with
//! a handful of accounts. Neither calls the application's own routes with
//! the accounts that matter, so a route whose guard let in an account its
//! requirement refuses, or refused one it admits, would pass both. [`run`]
//! walks a route table, calls every route with a full session of each of
//! the 37 [`MatrixSubject`]s, and reports every answer that differs from
//! what the route's requirement says for that account. It reads the route
//! table at run time, so a route that a later slice adds is called without
//! a line of its own.
//!
//! The accounts ([`MatrixSubject::all`]): each role alone, no role and no
//! grant, each single extra grant, and each "all but one" set (every
//! permission except one as extra grants, without the `admin` role, which
//! no extra grant gives, ADR 0007 §2). The all-but-one sets find a route
//! that asks for one permission more than it declares; the single grants
//! find one that lets in a permission it does not name.
//!
//! What is expected ([`MatrixSubject::admits`]) is computed from the
//! account's definition and the role matrix ([`Role::permissions`], which
//! the unit tests of `domain::authz` hold to ADR 0007 §2's table), not by
//! the guard's own [`Requirement::is_met_by`], so the harness checks the
//! guard's whole path: the grants stored in `user_roles` and
//! `user_permissions`, read for the request, checked, and answered.
//!
//! What a cell reads ([`Outcome`], [`cell_findings`]): `403` with the
//! `forbidden` envelope is the guard's refusal; anything else means the
//! guard let the request through, whatever the handler then answered
//! (`404` for the absent record, `422` for the missing body). Two answers
//! of an admitted request are still findings: a `401` on a route that
//! needs a session, because every account holds a valid full session (on a
//! `Public` route the guard opens no session, so a `401` there is the
//! handler's own, such as a sign-in step that reads a cookie of its own),
//! and a `5xx`, which shows that the guard's path or the handler failed for
//! that account instead of answering it. A handler never answers the
//! `forbidden` code itself: a record-level rule answers its own code
//! (recipe 10 of the project skill), so this reading stays exact as the
//! handlers grow.
//!
//! On top of each route's own requirement, [`run`] holds the user editor
//! to ADR 0007 §5 directly: every write route under [`USER_EDITOR_PATH`]
//! (creating, updating, deleting or restoring a user, and changing roles
//! or extra grants) must refuse the account that holds every permission
//! except `edit users`, whatever the route declares. That test of §5 holds
//! as soon as the users slices mount those routes.
//!
//! What the matrix does not check: the record-level rules (a published
//! post, the user guards of ADR 0007 §6), because it sends no body and no
//! record reaches a handler, and whether the requirement a route declares
//! is the one ADR 0007 §4's table names for it, because it compares each
//! answer with the declaration. Each slice's request tests cover both
//! (recipe 10 step 4 of the project skill), and the access-row tests of
//! recipe 10 step 6 hold an ra-core resource's routes to its row.
//!
//! The requests leave alone everything the walk depends on: they carry no
//! body, and a path parameter gets [`ABSENT_ID`], which no row of a test
//! database has, so a handler that runs finds no record to change. A
//! handler may still change the calling account's own data, such as the
//! logout, which ends its sessions by moving its `token_version`, so the
//! account's token is signed again after every request that a handler may
//! have answered. Every write carries the admin origin ([`ADMIN_ORIGIN`]),
//! as the admin UI sends it, so the guard's `Origin` check (ADR 0006 §3,
//! ADR 0044) lets it through to the requirement. Every account is
//! invented.

use std::fmt;

use axum::http::{
    Method, StatusCode,
    header::{COOKIE, ORIGIN},
};
use axum_test::TestServer;
use backend3::{
    controllers::admin::routes::AdminRoute,
    domain::{
        authz::{Permission, Requirement, Role},
        origin::is_write_method,
        session::SessionScope,
        session_cookie::SESSION_COOKIE_NAME,
    },
    models::users,
};
use loco_rs::app::AppContext;
use sea_orm::EntityTrait;
use serde_json::Value;

use super::{
    ADMIN_ORIGIN,
    admin_probe::{account_with, session_token},
};

/// The value every path parameter gets: an id that no row of a per-test
/// database reaches (`i32::MAX`, so a handler that reads the segment as an
/// `i32` or an `i64` reads it too), so an admitted request to a record
/// route finds nothing to change.
pub const ABSENT_ID: &str = "2147483647";

/// The path of the user editor (ADR 0007 §1, `edit users`): the users
/// list and every route below it, among them the account actions of
/// ADR 0006 and the role and extra-grant changes.
///
/// The user-editor rule of [`cell_findings`] reaches only the writes under
/// this path ([`is_user_editor_write`]). A slice that mounts a write of a
/// user, its roles or its extra grants anywhere else (such as a restore of
/// the global trash of ADR 0011 §8 that can take a user) extends
/// [`is_user_editor_write`] to that route in the same PR, with a probe
/// test in `tests/requests/authz_matrix.rs`.
pub const USER_EDITOR_PATH: &str = "/api/admin/users";

/// One kind of account the matrix calls every route as.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MatrixSubject {
    /// Exactly this role, and no extra grant.
    Role(Role),
    /// No role and no extra grant: only the own account (ADR 0007 §3).
    Nothing,
    /// Exactly this extra grant, and no role.
    Grant(Permission),
    /// Every permission but this one as extra grants, and no role.
    AllBut(Permission),
}

impl MatrixSubject {
    /// Every account of the matrix: the four roles, no role, the sixteen
    /// single grants and the sixteen all-but-one sets, 37 in all.
    #[must_use]
    pub fn all() -> Vec<Self> {
        Role::ALL
            .into_iter()
            .map(Self::Role)
            .chain([Self::Nothing])
            .chain(Permission::ALL.into_iter().map(Self::Grant))
            .chain(Permission::ALL.into_iter().map(Self::AllBut))
            .collect()
    }

    /// The roles the account holds.
    #[must_use]
    pub fn roles(self) -> Vec<Role> {
        match self {
            Self::Role(role) => vec![role],
            Self::Nothing | Self::Grant(_) | Self::AllBut(_) => Vec::new(),
        }
    }

    /// The extra grants the account holds.
    #[must_use]
    pub fn extra(self) -> Vec<Permission> {
        match self {
            Self::Role(_) | Self::Nothing => Vec::new(),
            Self::Grant(permission) => vec![permission],
            Self::AllBut(left_out) => Permission::ALL
                .into_iter()
                .filter(|permission| *permission != left_out)
                .collect(),
        }
    }

    /// Whether the guard must let this account call a route that asks for
    /// `requirement`: what the matrix expects.
    ///
    /// - `requirement`: the route's requirement, as the route table
    ///   declares it.
    ///
    /// Read from the account's definition: a public route and an
    /// own-account route admit every signed-in account; the `admin` role
    /// only the admin (an extra grant never gives it, ADR 0007 §2); and a
    /// permission the accounts whose role grants it (ADR 0007 §2's matrix)
    /// or whose extra grants name it (§3).
    #[must_use]
    pub fn admits(self, requirement: Requirement) -> bool {
        match requirement {
            Requirement::Public | Requirement::AnyActiveUser => true,
            Requirement::AdminRole => self == Self::Role(Role::Admin),
            Requirement::Permission(needed) => match self {
                Self::Role(role) => role.permissions().contains(&needed),
                Self::Nothing => false,
                Self::Grant(granted) => granted == needed,
                Self::AllBut(left_out) => left_out != needed,
            },
        }
    }

    /// The invented e-mail address of the account, unique per subject.
    #[must_use]
    pub fn email(self) -> String {
        let slug = |name: &str| name.replace(' ', "-");
        match self {
            Self::Role(role) => format!("matrix-role-{}@example.test", role.name()),
            Self::Nothing => "matrix-nothing@example.test".to_owned(),
            Self::Grant(permission) => {
                format!("matrix-grant-{}@example.test", slug(permission.name()))
            }
            Self::AllBut(permission) => {
                format!("matrix-all-but-{}@example.test", slug(permission.name()))
            }
        }
    }
}

impl fmt::Display for MatrixSubject {
    /// Names the account in a finding, such as `the writer role` or
    /// `every permission but "edit users"`.
    ///
    /// - `formatter`: where to write.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Role(role) => write!(formatter, "the {role} role"),
            Self::Nothing => formatter.write_str("no role and no grant"),
            Self::Grant(permission) => {
                write!(formatter, "the single grant {:?}", permission.name())
            }
            Self::AllBut(permission) => {
                write!(formatter, "every permission but {:?}", permission.name())
            }
        }
    }
}

/// How the server answered one cell.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    /// `401`: on a route that needs a session, the server took the valid
    /// session for none; on a `Public` route, where the guard opens no
    /// session, the handler's own answer.
    Unauthorized,
    /// `403` with the `forbidden` envelope: the guard's refusal.
    Refused,
    /// Any other answer: the guard let the request through to the handler,
    /// which answered this status.
    Admitted(StatusCode),
}

impl Outcome {
    /// Reads an answer.
    ///
    /// - `status`: the answer's status.
    /// - `body`: the answer's body; only a `403` is read, for its `error`.
    ///
    /// Only the `forbidden` code is the guard's refusal. Any other `403`
    /// code reads as an admitted request, among them the guard's own
    /// `origin_mismatch`, which it answers before it opens the session
    /// (ADR 0006 §3, ADR 0044), and a layer's in front of the guard: a cell
    /// that should be refused then becomes a finding, so a request that
    /// never reached the requirement check cannot pass as its refusal.
    #[must_use]
    pub fn of(status: StatusCode, body: &[u8]) -> Self {
        if status == StatusCode::UNAUTHORIZED {
            return Self::Unauthorized;
        }
        if status == StatusCode::FORBIDDEN
            && serde_json::from_slice::<Value>(body)
                .is_ok_and(|envelope| envelope["error"] == "forbidden")
        {
            return Self::Refused;
        }
        Self::Admitted(status)
    }
}

impl fmt::Display for Outcome {
    /// Writes the outcome for a finding.
    ///
    /// - `formatter`: where to write.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unauthorized => formatter.write_str("401 unauthorized"),
            Self::Refused => formatter.write_str("403 forbidden"),
            Self::Admitted(status) => write!(formatter, "{status} from the handler"),
        }
    }
}

/// One request of the matrix and its answer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MatrixCell {
    /// The route's method.
    pub method: Method,
    /// The route's path as the table declares it, `/api/admin` included.
    pub path: String,
    /// The account the route was called as.
    pub subject: MatrixSubject,
    /// How the server answered.
    pub outcome: Outcome,
}

/// What [`run`] found.
#[derive(Debug, Default)]
pub struct MatrixReport {
    /// Every request, in the order it was sent: per account, every route
    /// in table order.
    pub cells: Vec<MatrixCell>,
    /// One line per answer that differs from what was expected; empty when
    /// the routes agree with the matrix.
    pub findings: Vec<String>,
}

impl MatrixReport {
    /// The outcome of calling `method path` as `subject`, if the walk sent
    /// that request.
    ///
    /// - `method`: the route's method.
    /// - `path`: the route's path, `/api/admin` included.
    /// - `subject`: the account.
    #[must_use]
    pub fn outcome(&self, method: &Method, path: &str, subject: MatrixSubject) -> Option<Outcome> {
        self.cells
            .iter()
            .find(|cell| cell.method == *method && cell.path == path && cell.subject == subject)
            .map(|cell| cell.outcome)
    }
}

/// `path` with every parameter filled: `{name}` with [`ABSENT_ID`], and a
/// catch-all `{*name}` with one segment of the same value.
///
/// - `path`: a route's path as the table declares it.
#[must_use]
pub fn fill_path(path: &str) -> String {
    path.split('/')
        .map(|segment| {
            if segment.starts_with('{') && segment.ends_with('}') {
                ABSENT_ID
            } else {
                segment
            }
        })
        .collect::<Vec<_>>()
        .join("/")
}

/// Whether a method writes: every method but the safe ones of RFC 9110
/// §9.2.1 (`GET`, `HEAD`, `OPTIONS`, `TRACE`).
///
/// - `method`: the request's method.
///
/// The guard's `Origin` check decides by the same rule
/// (`domain::origin::is_write_method`, ADR 0006 §3, ADR 0044), and this
/// asks it, so the matrix sends the admin origin on exactly the requests
/// that the check reads.
#[must_use]
pub fn is_write(method: &Method) -> bool {
    is_write_method(method.as_str())
}

/// Whether ADR 0007 §5's user-editing test applies to a route: a method
/// that writes ([`is_write`]) on [`USER_EDITOR_PATH`] or below it.
///
/// - `method`: the route's method.
/// - `path`: the route's path, `/api/admin` included.
///
/// A slice that mounts a user, role or extra-grant write outside
/// [`USER_EDITOR_PATH`] adds that route here, in the same PR, with a probe
/// test (see [`USER_EDITOR_PATH`]).
#[must_use]
pub fn is_user_editor_write(method: &Method, path: &str) -> bool {
    let under_users = path == USER_EDITOR_PATH
        || path
            .strip_prefix(USER_EDITOR_PATH)
            .is_some_and(|rest| rest.starts_with('/'));
    under_users && is_write(method)
}

/// What is wrong with one cell, if anything.
///
/// - `cell`: the request and its answer.
/// - `requirement`: what the route declares.
///
/// The findings, at most one per rule: a `401` on a route that needs a
/// session (a `Public` route's `401` is its handler's own, since the guard
/// opens no session there); the guard's refusal of an account the
/// requirement admits; any other answer to one it does not admit; a `5xx`
/// to an admitted account, which shows that the guard's path or the
/// handler failed for it instead of answering it; and ADR 0007 §5's rule
/// for the user editor.
#[must_use]
pub fn cell_findings(cell: &MatrixCell, requirement: Requirement) -> Vec<String> {
    let at = format!("{} {}", cell.method, cell.path);
    let subject = cell.subject;
    let outcome = cell.outcome;
    let admitted = subject.admits(requirement);
    let mut findings = Vec::new();
    if outcome == Outcome::Unauthorized && requirement.needs_session() {
        findings.push(format!(
            "{at}: the valid full session of {subject} was answered {outcome}"
        ));
    } else if admitted && outcome == Outcome::Refused {
        findings.push(format!(
            "{at}: {subject} meets {requirement}, which the route declares, but was answered \
             {outcome}"
        ));
    } else if !admitted && outcome != Outcome::Refused {
        findings.push(format!(
            "{at}: {subject} does not meet {requirement}, which the route declares, but was \
             answered {outcome} instead of 403 forbidden"
        ));
    } else if matches!(outcome, Outcome::Admitted(status) if status.is_server_error()) {
        findings.push(format!(
            "{at}: {subject} meets {requirement}, which the route declares, but was answered \
             {outcome}: a server error, so the walk has not shown that the route serves the \
             account"
        ));
    }
    if subject == MatrixSubject::AllBut(Permission::EditUsers)
        && is_user_editor_write(&cell.method, &cell.path)
        && outcome != Outcome::Refused
    {
        findings.push(format!(
            "{at}: ADR 0007 §5 refuses every change of a user, its roles or its extra grants \
             to an account with every permission but \"edit users\", which was answered \
             {outcome}; the route must ask for Requirement::Permission(Permission::EditUsers) \
             (or the admin role)"
        ));
    }
    findings
}

/// Sends one request of the matrix as the admin UI sends it: `route` with
/// its parameters filled ([`fill_path`]), no body, `token` as the session
/// cookie, and on a write ([`is_write`]) the admin origin
/// ([`ADMIN_ORIGIN`]) in the `Origin` header, as a browser adds it there
/// and not to a same-origin read.
///
/// - `server`: the server that mounts the route.
/// - `route`: the route.
/// - `token`: a full session of the account.
///
/// This is the one place where the matrix builds a request: a layer that
/// a later slice puts in front of the guard and that reads more than the
/// session and the origin adds here what a browser sends, so the matrix
/// still reaches the guard. Without the origin, the guard's `Origin`
/// check (ADR 0006 §3, ADR 0044) would answer every write
/// `403 origin_mismatch` before it opens the session, and no write route
/// would reach its requirement.
async fn send(server: &TestServer, route: &AdminRoute, token: &str) -> (StatusCode, Vec<u8>) {
    let request = server
        .method(route.method().clone(), &fill_path(&route.path()))
        .add_header(COOKIE, format!("{SESSION_COOKIE_NAME}={token}"));
    let request = if is_write(route.method()) {
        request.add_header(ORIGIN, ADMIN_ORIGIN)
    } else {
        request
    };
    let response = request.await;
    (response.status_code(), response.as_bytes().to_vec())
}

/// Calls every route of `routes` on `server` as each [`MatrixSubject`],
/// with a full session, and reports each answer that differs from what
/// the route's requirement says for that account, and from ADR 0007 §5 for
/// the user editor ([`cell_findings`]).
///
/// - `server`: the server that mounts the routes: the application's own
///   (`support::with_server`), or a probe table's (`admin_probe::probe_server`).
/// - `ctx`: the app context of the server's database; the 37 invented
///   accounts are inserted there, so it must not hold their addresses yet.
/// - `routes`: the route declarations to walk and read the requirements
///   from; `routes::table()` for the application.
///
/// # Panics
///
/// When an account cannot be inserted or read back.
pub async fn run(server: &TestServer, ctx: &AppContext, routes: &[AdminRoute]) -> MatrixReport {
    let mut report = MatrixReport::default();
    for subject in MatrixSubject::all() {
        let user = account_with(
            ctx,
            &subject.email(),
            &subject.roles(),
            &subject.extra(),
            |_| {},
        )
        .await;
        let mut token = session_token(ctx, &user, SessionScope::Admin);
        for route in routes {
            let (status, body) = send(server, route, &token).await;
            let cell = MatrixCell {
                method: route.method().clone(),
                path: route.path(),
                subject,
                outcome: Outcome::of(status, &body),
            };
            report
                .findings
                .extend(cell_findings(&cell, route.requirement()));
            if cell.outcome != Outcome::Refused {
                // A handler may have answered (anything but the guard's
                // refusal; a `Public` route's `401` is its handler's own)
                // and ended the account's sessions (the logout moves
                // `token_version`): sign a token for the account as it is
                // now. When the guard answered `401` instead, this costs
                // one token.
                let current = users::Entity::find_by_id(user.id)
                    .one(&ctx.db)
                    .await
                    .expect("read the matrix account")
                    .expect("the matrix account still exists");
                token = session_token(ctx, &current, SessionScope::Admin);
            }
            report.cells.push(cell);
        }
    }
    report
}
