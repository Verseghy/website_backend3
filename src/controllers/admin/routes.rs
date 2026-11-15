//! The admin route table: every `/api/admin` route, each declared with the
//! [`Requirement`] an account must meet to call it (ADR 0007 §4, §5, "Deny
//! by default").
//!
//! [`table`] is the one list of admin routes, and [`routes`] builds the
//! admin API's router from it and from nothing else; `App::routes` adds that
//! router once. Each entry is an [`AdminRoute`]: its method, its path below
//! `/api/admin`, its requirement (a permission, the `admin` role, any active
//! user, or public), the session scopes it accepts, and how large its body
//! may be. A requirement has no default, so a route cannot be added without
//! saying who may call it. Two tests hold the rest:
//!
//! - `tests/requests/admin_routes.rs` compares what `App::routes` mounts
//!   under `/api/admin` with the table. It fails on a route mounted there
//!   without being in the table (also one that Loco records no method for,
//!   such as `any(...)`), on a table route that is not mounted, on a route
//!   mounted or declared twice, on a `Public` route other than the sign-in
//!   steps, and on a route other than the own-account routes of ADR 0006 §4
//!   that accepts a `password_change` session.
//! - The other hooks that could add a route (`before_routes`,
//!   `after_routes`, `serve`, an initializer) never reach `App::routes`, so
//!   `tests/architecture/admin_mounts.rs` reads `src/` instead: outside this
//!   file and `domain::api_path`, no code builds a path under `/api/admin`,
//!   and only `App::routes` reaches [`routes`] or [`build`]. Its module
//!   documentation lists what a text check cannot see.
//!
//! The access map that `/auth/me` sends (ADR 0007 §7) is computed from a
//! second table, `domain::authz::RESOURCE_ACCESS`, one row per ra-core
//! resource. [`AdminRoute::serving`] marks the routes each row stands for,
//! and this module's tests hold the two tables together: every action of a
//! row has a route that serves it, a route serves only actions that a row
//! offers, and a row offers an action only to accounts that every route
//! serving it lets in. Without that, the sidebar could show an entry that
//! answers `403`, and the `/auth/me` refetch after the refusal would bring
//! the same map back.
//!
//! The table enforces each requirement itself, in a guard that runs before
//! the handler: the handler of every entry is wrapped so that its
//! requirement is checked whatever the handler takes. First, for every
//! requirement, `Public` included, the guard
//!
//! 0. refuses a write (`POST`, `PUT`, `PATCH`, `DELETE`) that carries the
//!    session cookie or the two-factor cookie but not the admin origin in
//!    its `Origin` header, with `403 origin_mismatch` and before it reads
//!    anything else (`super::origin`, ADR 0006 §3 as amended by ADR 0044);
//!    a Bearer request without a credential cookie is exempt.
//!
//! Then, for a requirement that needs a session, it
//!
//! 1. opens the session with the route's accepted scopes
//!    (`super::session::open_session`): no valid session, or a scope the
//!    route does not accept, answers `401 unauthorized`;
//! 2. checks the requirement against the roles and extra grants the session
//!    read for this request ([`Requirement::is_met_by`]): an account that
//!    does not meet it gets `403 forbidden`, with the Hungarian message in
//!    the admin (ADR 0021 §4);
//! 3. hands the opened session to the handler, whose
//!    [`super::session::AdminSession`] takes it from the request;
//! 4. adds the passive sliding refresh to the answer (ADR 0006 §1): a full
//!    `admin` session that the session cookie carries, with less than
//!    3,600 s of its token left, gets a renewed `__Host-admin_session`
//!    cookie, on the handler's answer or on the `403`, unless the handler
//!    set the session cookie itself (the logout's deletion, the explicit
//!    refresh). A Bearer token and a restricted session never get one. The
//!    maintainer kept the refresh on the `403` as built on 2026-11-15:
//!    "Keep as built: every guarded answer, the 403 included" (research:
//!    DECISIONS R35 (D156); ADR 0006 §1 quotes the question), so a
//!    handler's own record-level `403` carries it too.
//!
//! So the `Origin` check comes first, `401` is always decided before
//! `403 forbidden` (ADR 0007 §5's flowchart), and all of them before the
//! handler reads anything: a list that the account may not
//! see answers `403` and never an empty list, and a body that is not the
//! route's DTO, or is too large, is looked at only for an account that may
//! call the route. A `Public` route runs its handler without a session; a
//! handler there that takes `AdminSession` answers `500`.
//!
//! Every admin response carries `Cache-Control: no-store` (ADR 0023 §6):
//! successes, the guard's and the handlers' error envelopes, and the `405`
//! that axum answers for a method a path does not take. Admin responses
//! carry account data and session cookies, so no browser or proxy may keep
//! them. Unknown `/api/admin` paths get the same header from the router
//! fallback ([`crate::controllers::not_found`]).
//!
//! Request bodies: a route is a JSON route by default, whose body may be at
//! most [`JSON_BODY_LIMIT`] (4 MiB, ADR 0023 §5); [`AdminRoute::upload`]
//! marks an upload route, which keeps the 128 MiB of
//! `server.middlewares.limit_payload` (ADR 0015 §6) and reads its file
//! through [`super::media`].
//!
//! Loco and axum facts that shape [`build`]:
//!
//! - loco-rs 1.2.0's `Routes::layer` wraps only the handlers that the group
//!   already holds, so the JSON limit goes on the JSON group and the
//!   `no-store` layer on the merged group, after the last merge.
//! - `Routes::merge` drops the merged group's prefix, so the two groups have
//!   none and the merged group gets `/api/admin`.
//! - axum's `Json` extractor reads its limit from the request, which
//!   `DefaultBodyLimit` sets; the route-level layer sets it after Loco's
//!   global `limit_payload` layer did, so it wins.
//! - axum merges two method routers of one path, so a path may appear in
//!   the table once per method; a method twice on one path makes axum panic
//!   when the router is built ("Overlapping method route").
//! - The guard wraps the handler as an axum `Handler`, not as a layer: a
//!   handler receives the router's state (the `AppContext`) when it is
//!   called, while a layer would need it when the router is built, which
//!   `App::routes` does not give.
//!
//! Recipe 10 of `.claude/skills/backend3/SKILL.md` says how to add a
//! route.

use std::{fmt, future::Future, pin::Pin};

use axum::{
    extract::{DefaultBodyLimit, Request},
    handler::Handler,
    http::{HeaderValue, Method, header::CACHE_CONTROL},
    response::{IntoResponse, Response},
    routing::{MethodFilter, MethodRouter, on},
};
use loco_rs::{app::AppContext, controller::Routes};
use tower_http::set_header::SetResponseHeaderLayer;

use super::{
    JSON_BODY_LIMIT, auth, error_response, labels, origin,
    session::{
        AcceptedScopes, AdminOrPasswordChange, AdminScope, accepted_scopes, open_session,
        passive_refresh_cookie, session_refusal_response, with_passive_refresh,
    },
};
use crate::{
    domain::{
        api_path::ADMIN_API_PREFIX,
        authz::{AccessAction, Requirement},
        session::SessionScope,
    },
    dtos::admin::error::ErrorCode,
};

/// Every route of the admin API, in the order they are listed for review.
///
/// A new admin route is one more entry here, with its requirement from
/// ADR 0007 §4; the module documentation says what the guard does with it.
#[must_use]
pub fn table() -> Vec<AdminRoute> {
    vec![
        // The sign-in issues the session, so it cannot ask for one
        // (ADR 0006 §4).
        AdminRoute::post("/auth/login", Requirement::Public, auth::login),
        // A script's sign-in: it issues the Bearer token, so it cannot ask
        // for one either (ADR 0006 §3).
        AdminRoute::post("/auth/token", Requirement::Public, auth::token),
        // The account's own data: any signed-in account, also one that must
        // first set a new password (ADR 0006 §4, ADR 0007 §7).
        AdminRoute::get("/auth/me", Requirement::AnyActiveUser, auth::me)
            .accepting::<AdminOrPasswordChange>(),
        // POST only (ADR 0006 §2): axum answers every other method with 405.
        AdminRoute::post("/auth/logout", Requirement::AnyActiveUser, auth::logout)
            .accepting::<AdminOrPasswordChange>(),
        // The explicit sliding refresh (ADR 0006 §1): a full session only,
        // so a `password_change` session cannot extend itself (§4).
        AdminRoute::post("/auth/refresh", Requirement::AnyActiveUser, auth::refresh),
        // Labels (LBL-01): the list and the record, which `edit labels`
        // reads and `edit posts` reads for the post form's label picker
        // (ADR 0007 §4). The access map offers the screens to `edit labels`
        // only (`domain::authz::RESOURCE_ACCESS`).
        AdminRoute::get("/labels", labels::READ, labels::list)
            .serving("labels", &[AccessAction::List]),
        AdminRoute::get("/labels/{id}", labels::READ, labels::show)
            .serving("labels", &[AccessAction::Show]),
    ]
}

/// The admin API's routes, for `App::routes`: [`build`] over [`table`].
#[must_use]
pub fn routes() -> Routes {
    build(table())
}

/// Builds the admin API's Loco route group from `table`, and from nothing
/// else: each route behind its requirement's guard, the JSON routes under
/// the 4 MiB body limit, and every route under the `no-store` layer.
///
/// - `table`: the routes; [`table`] in the application, a probe table in
///   the tests of the guard.
///
/// # Panics
///
/// axum panics, when the router is built, if `table` lists one method of
/// one path twice.
#[must_use]
pub fn build(table: Vec<AdminRoute>) -> Routes {
    let mut json = Routes::new();
    let mut uploads = Routes::new();
    for route in table {
        let group = match route.body {
            RequestBody::Json => &mut json,
            RequestBody::Upload => &mut uploads,
        };
        let path = route.path;
        *group = std::mem::take(group).add(path, route.into_method_router());
    }
    Routes::new()
        .prefix(ADMIN_API_PREFIX)
        .merge(json.layer(DefaultBodyLimit::max(JSON_BODY_LIMIT)))
        .merge(uploads)
        // Last, so that it wraps every route above. `overriding`: no handler
        // may make an admin response cacheable.
        .layer(SetResponseHeaderLayer::overriding(
            CACHE_CONTROL,
            HeaderValue::from_static("no-store"),
        ))
}

/// How large a route's request body may be.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RequestBody {
    /// At most [`JSON_BODY_LIMIT`] (4 MiB, ADR 0023 §5): every route that
    /// reads a JSON body or none.
    Json,
    /// At most `server.middlewares.limit_payload` (128 MiB, ADR 0015 §6): a
    /// route that receives a file.
    Upload,
}

/// What builds a route's method router once the guard is known.
type Mount = Box<dyn FnOnce(Guard) -> MethodRouter<AppContext>>;

/// One route of the admin route table.
///
/// Built with [`AdminRoute::get`], [`AdminRoute::post`], [`AdminRoute::put`],
/// [`AdminRoute::patch`] or [`AdminRoute::delete`], each of which takes the
/// path, the requirement and the handler, and then, where the route needs
/// it, [`AdminRoute::accepting`] and [`AdminRoute::upload`].
pub struct AdminRoute {
    /// The method the route answers (and `HEAD` with `GET`, as axum does).
    method: Method,
    /// The path below `/api/admin`, such as `/auth/login`; a parameter is
    /// written `{id}`, as axum 0.8 writes it.
    path: &'static str,
    /// What the account must meet.
    requirement: Requirement,
    /// The session scopes the guard accepts.
    scopes: &'static [SessionScope],
    /// How large the request body may be.
    body: RequestBody,
    /// The actions of the access table that the route serves, by resource
    /// ([`AdminRoute::serving`]); empty for a route that no row describes.
    serves: Vec<ServedAccess>,
    /// Builds the guarded method router.
    mount: Mount,
}

/// What an admin route serves of one row of the access table
/// (`domain::authz::RESOURCE_ACCESS`, ADR 0007 §4, §7): the ra-core
/// resource and the actions of it that the admin takes through the route.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ServedAccess {
    /// The ra-core resource name, as the access table's row names it.
    pub resource: &'static str,
    /// The actions of that row the route serves.
    pub actions: &'static [AccessAction],
}

impl AdminRoute {
    /// A `GET` route (axum also answers `HEAD` with it).
    ///
    /// - `path`: the path below `/api/admin`, starting with `/`.
    /// - `requirement`: what the calling account must meet (ADR 0007 §4).
    /// - `handler`: the handler; its own extractors run after the guard.
    #[must_use]
    pub fn get<H, T>(path: &'static str, requirement: Requirement, handler: H) -> Self
    where
        H: Handler<T, AppContext>,
        T: 'static,
    {
        Self::new(Method::GET, MethodFilter::GET, path, requirement, handler)
    }

    /// A `POST` route.
    ///
    /// - `path`: the path below `/api/admin`, starting with `/`.
    /// - `requirement`: what the calling account must meet (ADR 0007 §4).
    /// - `handler`: the handler; its own extractors run after the guard.
    #[must_use]
    pub fn post<H, T>(path: &'static str, requirement: Requirement, handler: H) -> Self
    where
        H: Handler<T, AppContext>,
        T: 'static,
    {
        Self::new(Method::POST, MethodFilter::POST, path, requirement, handler)
    }

    /// A `PUT` route.
    ///
    /// - `path`: the path below `/api/admin`, starting with `/`.
    /// - `requirement`: what the calling account must meet (ADR 0007 §4).
    /// - `handler`: the handler; its own extractors run after the guard.
    #[must_use]
    pub fn put<H, T>(path: &'static str, requirement: Requirement, handler: H) -> Self
    where
        H: Handler<T, AppContext>,
        T: 'static,
    {
        Self::new(Method::PUT, MethodFilter::PUT, path, requirement, handler)
    }

    /// A `PATCH` route.
    ///
    /// - `path`: the path below `/api/admin`, starting with `/`.
    /// - `requirement`: what the calling account must meet (ADR 0007 §4).
    /// - `handler`: the handler; its own extractors run after the guard.
    #[must_use]
    pub fn patch<H, T>(path: &'static str, requirement: Requirement, handler: H) -> Self
    where
        H: Handler<T, AppContext>,
        T: 'static,
    {
        Self::new(
            Method::PATCH,
            MethodFilter::PATCH,
            path,
            requirement,
            handler,
        )
    }

    /// A `DELETE` route.
    ///
    /// - `path`: the path below `/api/admin`, starting with `/`.
    /// - `requirement`: what the calling account must meet (ADR 0007 §4).
    /// - `handler`: the handler; its own extractors run after the guard.
    #[must_use]
    pub fn delete<H, T>(path: &'static str, requirement: Requirement, handler: H) -> Self
    where
        H: Handler<T, AppContext>,
        T: 'static,
    {
        Self::new(
            Method::DELETE,
            MethodFilter::DELETE,
            path,
            requirement,
            handler,
        )
    }

    /// The route with `handler` behind the guard, accepting a full `admin`
    /// session and a JSON body until the builder methods say otherwise.
    fn new<H, T>(
        method: Method,
        filter: MethodFilter,
        path: &'static str,
        requirement: Requirement,
        handler: H,
    ) -> Self
    where
        H: Handler<T, AppContext>,
        T: 'static,
    {
        Self {
            method,
            path,
            requirement,
            scopes: accepted_scopes::<AdminScope>(),
            body: RequestBody::Json,
            serves: Vec::new(),
            mount: Box::new(move |guard| on(filter, Guarded { handler, guard })),
        }
    }

    /// The route, accepting the session scopes of the marker `A` instead of
    /// a full `admin` session only.
    ///
    /// - `A`: [`AdminOrPasswordChange`] for the three own-account routes
    ///   that ADR 0006 §4 opens to a `password_change` session; no other
    ///   route may take it, and the route table's test lists them. The
    ///   handler's `AdminSession` names the same marker.
    #[must_use]
    pub fn accepting<A: AcceptedScopes>(mut self) -> Self {
        self.scopes = accepted_scopes::<A>();
        self
    }

    /// The route as an upload route, whose body may be as large as
    /// `server.middlewares.limit_payload` allows (128 MiB, ADR 0015 §6)
    /// instead of [`JSON_BODY_LIMIT`].
    #[must_use]
    pub fn upload(mut self) -> Self {
        self.body = RequestBody::Upload;
        self
    }

    /// The route, marked as the one the admin calls to take `actions` on
    /// the ra-core resource `resource` (ADR 0007 §7).
    ///
    /// - `resource`: the ra-core resource name, as its row of the access
    ///   table (`domain::authz::RESOURCE_ACCESS`) names it.
    /// - `actions`: the actions of that row the route serves, such as
    ///   `&[AccessAction::List]` for the list route.
    ///
    /// The mark changes nothing in what the guard checks. It ties the
    /// access map that `/auth/me` sends to the routes it stands for: this
    /// module's tests fail when an action of a row has no route that
    /// serves it, when a route serves an action that no row offers, and
    /// when a row offers an action to an account that a route serving it
    /// refuses, so the sidebar never shows an entry that answers `403`.
    /// Call it once per resource the route serves.
    #[must_use]
    pub fn serving(mut self, resource: &'static str, actions: &'static [AccessAction]) -> Self {
        self.serves.push(ServedAccess { resource, actions });
        self
    }

    /// The route's method.
    #[must_use]
    pub fn method(&self) -> &Method {
        &self.method
    }

    /// The route's whole path, `/api/admin` included, as Loco's route table
    /// (`AppRoutes::collect()`) writes it.
    #[must_use]
    pub fn path(&self) -> String {
        format!("{ADMIN_API_PREFIX}{}", self.path)
    }

    /// What an account must meet to call the route.
    #[must_use]
    pub fn requirement(&self) -> Requirement {
        self.requirement
    }

    /// The session scopes the route accepts; never `mfa`.
    #[must_use]
    pub fn scopes(&self) -> &'static [SessionScope] {
        self.scopes
    }

    /// How large the route's request body may be.
    #[must_use]
    pub fn body(&self) -> RequestBody {
        self.body
    }

    /// The actions of the access table the route serves, by resource, in
    /// the order [`AdminRoute::serving`] added them.
    #[must_use]
    pub fn serves(&self) -> &[ServedAccess] {
        &self.serves
    }

    /// The route's method router, its handler behind the guard of its
    /// requirement and scopes.
    fn into_method_router(self) -> MethodRouter<AppContext> {
        (self.mount)(Guard {
            requirement: self.requirement,
            scopes: self.scopes,
        })
    }
}

impl fmt::Debug for AdminRoute {
    /// Writes the route's declaration; the handler has no `Debug`.
    ///
    /// - `formatter`: where to write.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AdminRoute")
            .field("method", &self.method)
            .field("path", &self.path())
            .field("requirement", &self.requirement)
            .field("scopes", &self.scopes)
            .field("body", &self.body)
            .field("serves", &self.serves)
            .finish_non_exhaustive()
    }
}

/// What the guard of one route checks.
#[derive(Debug, Clone, Copy)]
struct Guard {
    /// The route's requirement.
    requirement: Requirement,
    /// The session scopes the route accepts.
    scopes: &'static [SessionScope],
}

/// What the guard decided about a request.
enum Admission {
    /// Call the handler with this request, which carries the opened session
    /// when the requirement needs one, and add the passive refresh's
    /// `Set-Cookie` value, if any, to its answer.
    Admitted(Request, Option<HeaderValue>),
    /// Send this answer instead of calling the handler.
    Refused(Response),
}

impl Guard {
    /// Lets `request` through to the handler, or answers it.
    ///
    /// - `request`: the request, untouched unless a session is opened, which
    ///   goes into its extensions for the handler's `AdminSession`.
    /// - `ctx`: the router's state.
    ///
    /// The answer of a refused request is `403 origin_mismatch` for a
    /// cookie-authenticated write without the admin origin
    /// ([`origin::refusal`], for every requirement), the session's
    /// `401 unauthorized` or `500` ([`open_session`],
    /// [`session_refusal_response`]), or `403 forbidden` for an account that
    /// does not meet the requirement, in that order.
    ///
    /// Once the session holds, the guard also decides its passive refresh
    /// (ADR 0006 §1, [`passive_refresh_cookie`]): a full `admin` session in
    /// the session cookie with less than 3,600 s left gets a renewed cookie
    /// on its answer, the handler's or the `403 forbidden`, unless the
    /// handler sets the session cookie itself ([`with_passive_refresh`]). A
    /// failure to build it (no app clock, no `auth.jwt`) answers Loco's
    /// `500`. The `403 origin_mismatch` comes before the session is opened,
    /// so it never carries a renewed cookie.
    async fn admit(self, request: Request, ctx: &AppContext) -> Admission {
        let (mut parts, body) = request.into_parts();
        // First, for every requirement, `Public` included: a write with a
        // credential cookie but without the admin origin is answered before
        // anything is read, so its session is never even opened.
        if let Some(answer) = origin::refusal(&parts, ctx) {
            return Admission::Refused(answer);
        }
        if !self.requirement.needs_session() {
            return Admission::Admitted(Request::from_parts(parts, body), None);
        }
        let session = match open_session(&mut parts, ctx, self.scopes).await {
            Ok(session) => session,
            Err(error) => return Admission::Refused(session_refusal_response(error)),
        };
        let refresh = match passive_refresh_cookie(&session, ctx) {
            Ok(refresh) => refresh,
            Err(error) => return Admission::Refused(error.into_response()),
        };
        if !self.requirement.is_met_by(&session.grants) {
            // DEBUG, like the 401: the admin hides what an account may not
            // use, so a 403 comes from a stale tab or a script, and the
            // access log records its status anyway.
            tracing::debug!(
                user_id = session.user.id,
                requirement = %self.requirement,
                method = %parts.method,
                path = parts.uri.path(),
                "admin request refused: the account does not meet the route's requirement"
            );
            return Admission::Refused(with_passive_refresh(
                error_response(ErrorCode::Forbidden),
                refresh,
            ));
        }
        session.hand_over(&mut parts);
        Admission::Admitted(Request::from_parts(parts, body), refresh)
    }
}

/// A handler behind its route's guard: an axum [`Handler`] that runs the
/// guard and then, if it lets the request through, the handler.
#[derive(Clone)]
struct Guarded<H> {
    /// The route's handler.
    handler: H,
    /// The route's guard.
    guard: Guard,
}

impl<H, T> Handler<T, AppContext> for Guarded<H>
where
    H: Handler<T, AppContext>,
    T: 'static,
{
    type Future = Pin<Box<dyn Future<Output = Response> + Send>>;

    /// Runs the guard, then the handler with the same state, and adds the
    /// passive refresh's cookie to the handler's answer.
    ///
    /// - `request`: the request.
    /// - `state`: the router's state, which the guard reads the database and
    ///   the JWT configuration from.
    fn call(self, request: Request, state: AppContext) -> Self::Future {
        Box::pin(async move {
            match self.guard.admit(request, &state).await {
                Admission::Admitted(request, refresh) => {
                    with_passive_refresh(self.handler.call(request, state).await, refresh)
                }
                Admission::Refused(answer) => answer,
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_table_declares_the_auth_routes_with_their_requirements() {
        let declared: Vec<(Method, String, Requirement, &[SessionScope], RequestBody)> = table()
            .iter()
            .map(|route| {
                (
                    route.method().clone(),
                    route.path(),
                    route.requirement(),
                    route.scopes(),
                    route.body(),
                )
            })
            .collect();
        let own_account: &[SessionScope] = &[SessionScope::Admin, SessionScope::PasswordChange];
        // The routes of the auth slices; later slices add theirs beside them
        // (`tests/requests/admin_routes.rs` checks the whole table).
        for expected in [
            (
                Method::POST,
                "/api/admin/auth/login".to_owned(),
                Requirement::Public,
                &[SessionScope::Admin][..],
                RequestBody::Json,
            ),
            (
                Method::GET,
                "/api/admin/auth/me".to_owned(),
                Requirement::AnyActiveUser,
                own_account,
                RequestBody::Json,
            ),
            (
                Method::POST,
                "/api/admin/auth/logout".to_owned(),
                Requirement::AnyActiveUser,
                own_account,
                RequestBody::Json,
            ),
            (
                Method::POST,
                "/api/admin/auth/refresh".to_owned(),
                Requirement::AnyActiveUser,
                &[SessionScope::Admin][..],
                RequestBody::Json,
            ),
            // ADR 0007 §4: labels are read with `edit labels`, or with
            // `edit posts` for the post form's picker.
            (
                Method::GET,
                "/api/admin/labels".to_owned(),
                Requirement::AnyPermission(&[
                    crate::domain::authz::Permission::EditLabels,
                    crate::domain::authz::Permission::EditPosts,
                ]),
                &[SessionScope::Admin][..],
                RequestBody::Json,
            ),
            (
                Method::GET,
                "/api/admin/labels/{id}".to_owned(),
                Requirement::AnyPermission(&[
                    crate::domain::authz::Permission::EditLabels,
                    crate::domain::authz::Permission::EditPosts,
                ]),
                &[SessionScope::Admin][..],
                RequestBody::Json,
            ),
        ] {
            assert!(declared.contains(&expected), "{expected:?} in {declared:?}");
        }
        // Every path is under the prefix, written as Loco writes it back.
        for (method, path, ..) in &declared {
            assert!(path.starts_with("/api/admin/"), "{method} {path}");
            assert!(
                !path.ends_with('/') && !path.contains("//"),
                "{method} {path}"
            );
        }
    }

    #[test]
    fn the_builder_methods_set_what_they_name() {
        async fn probe() {}
        for (route, method) in [
            (
                AdminRoute::get("/a", Requirement::AdminRole, probe),
                Method::GET,
            ),
            (
                AdminRoute::post("/a", Requirement::AdminRole, probe),
                Method::POST,
            ),
            (
                AdminRoute::put("/a", Requirement::AdminRole, probe),
                Method::PUT,
            ),
            (
                AdminRoute::patch("/a", Requirement::AdminRole, probe),
                Method::PATCH,
            ),
            (
                AdminRoute::delete("/a", Requirement::AdminRole, probe),
                Method::DELETE,
            ),
        ] {
            assert_eq!(route.method(), &method);
            assert_eq!(route.path(), "/api/admin/a");
            assert_eq!(route.requirement(), Requirement::AdminRole);
            // A full session and a JSON body unless the route says otherwise.
            assert_eq!(route.scopes(), &[SessionScope::Admin]);
            assert_eq!(route.body(), RequestBody::Json);
            let route = route.accepting::<AdminOrPasswordChange>().upload();
            assert_eq!(
                route.scopes(),
                &[SessionScope::Admin, SessionScope::PasswordChange]
            );
            assert_eq!(route.body(), RequestBody::Upload);
            let shown = format!("{route:?}");
            assert!(shown.contains(r#"path: "/api/admin/a""#), "{shown}");
            assert!(shown.contains("requirement: AdminRole"), "{shown}");
        }
    }

    #[test]
    fn build_mounts_every_route_of_the_table_under_the_prefix() {
        async fn probe() {}
        let routes = build(vec![
            AdminRoute::get("/x", Requirement::AnyActiveUser, probe),
            AdminRoute::post(
                "/x",
                Requirement::Permission(crate::domain::authz::Permission::EditLabels),
                probe,
            ),
            AdminRoute::post("/y/{id}", Requirement::Public, probe).upload(),
        ]);
        assert_eq!(routes.prefix.as_deref(), Some(ADMIN_API_PREFIX));
        let mounted: Vec<(String, Vec<Method>)> = routes
            .handlers
            .iter()
            .map(|handler| (handler.uri.clone(), handler.actions.clone()))
            .collect();
        // The JSON group first, then the uploads; each with the method that
        // Loco reads back from the method router.
        assert_eq!(
            mounted,
            [
                ("/x".to_owned(), vec![Method::GET]),
                ("/x".to_owned(), vec![Method::POST]),
                ("/y/{id}".to_owned(), vec![Method::POST]),
            ]
        );
    }

    // The access table against the route table (ADR 0007 §4, §7).

    use std::collections::BTreeSet;

    use crate::domain::authz::{
        AccountGrants, ActionRule, Permission, RESOURCE_ACCESS, ResourceAccess, Role,
    };

    /// The accounts the access check tries: every set of roles, each with
    /// no extra grant, with every single grant and with every pair of
    /// grants. That finds a stricter route for every rule of at most two
    /// permissions, which is every rule ADR 0007 §4 reads; the weakest
    /// account of each rule ([`weakest_account`]) covers longer ones.
    fn probe_accounts() -> Vec<AccountGrants> {
        let mut grant_sets: Vec<BTreeSet<Permission>> = vec![BTreeSet::new()];
        for (index, first) in Permission::ALL.iter().enumerate() {
            grant_sets.push(BTreeSet::from([*first]));
            for second in &Permission::ALL[index + 1..] {
                grant_sets.push(BTreeSet::from([*first, *second]));
            }
        }
        let mut accounts = Vec::new();
        for mask in 0..(1_u32 << Role::ALL.len()) {
            let roles: BTreeSet<Role> = Role::ALL
                .iter()
                .enumerate()
                .filter(|(bit, _)| mask & (1 << bit) != 0)
                .map(|(_, role)| *role)
                .collect();
            for extra in &grant_sets {
                accounts.push(AccountGrants {
                    roles: roles.clone(),
                    extra: extra.clone(),
                });
            }
        }
        accounts
    }

    /// The accounts that hold exactly what `rule` names: its permissions as
    /// extra grants, one permission of each "or" list
    /// ([`Requirement::AnyPermission`], one account per choice), and the
    /// `admin` role when the rule asks for it. An empty "or" list, which no
    /// account meets, leaves no account.
    fn weakest_accounts(rule: &ActionRule) -> Vec<AccountGrants> {
        let mut accounts = vec![AccountGrants {
            roles: if rule.requires.contains(&Requirement::AdminRole) {
                BTreeSet::from([Role::Admin])
            } else {
                BTreeSet::new()
            },
            extra: BTreeSet::new(),
        }];
        for requirement in rule.requires {
            match requirement {
                Requirement::Permission(permission) => {
                    for account in &mut accounts {
                        account.extra.insert(*permission);
                    }
                }
                Requirement::AnyPermission(permissions) => {
                    accounts = accounts
                        .into_iter()
                        .flat_map(|account| {
                            permissions.iter().map(move |permission| {
                                let mut chosen = account.clone();
                                chosen.extra.insert(*permission);
                                chosen
                            })
                        })
                        .collect();
                }
                Requirement::Public | Requirement::AnyActiveUser | Requirement::AdminRole => {}
            }
        }
        accounts
    }

    /// What is wrong between the access table `access` and the route table
    /// `routes`, one line per defect; empty when they agree.
    ///
    /// - `access`: the access rows; `RESOURCE_ACCESS`, or a probe table.
    /// - `routes`: the admin routes; [`table`], or probe routes.
    ///
    /// Only one direction of the requirements is checked: a row may offer
    /// an action to fewer accounts than its routes let in (a picker that
    /// another permission may read, ADR 0007 §4), never to more.
    fn access_route_problems(access: &[ResourceAccess], routes: &[AdminRoute]) -> Vec<String> {
        let accounts = probe_accounts();
        let mut problems = Vec::new();
        for route in routes {
            let at = format!("{} {}", route.method(), route.path());
            for served in route.serves() {
                let resource = served.resource;
                if served.actions.is_empty() {
                    problems.push(format!("{at}: serves no action of {resource:?}"));
                }
                for action in served.actions {
                    let Some(rule) = access
                        .iter()
                        .filter(|row| row.resource == resource)
                        .flat_map(|row| row.rules)
                        .find(|rule| rule.action == *action)
                    else {
                        problems.push(format!(
                            "{at}: serves {resource:?}/{action}, which no row of the access \
                             table offers"
                        ));
                        continue;
                    };
                    let requirement = route.requirement();
                    if let Some(account) = accounts
                        .iter()
                        .cloned()
                        .chain(weakest_accounts(rule))
                        .find(|account| rule.is_met_by(account) && !requirement.is_met_by(account))
                    {
                        problems.push(format!(
                            "{resource:?}/{action}: the access map offers it to {account:?}, but \
                             {at}, which serves it, asks for {requirement}"
                        ));
                    }
                }
            }
        }
        for row in access {
            for rule in row.rules {
                let served = routes.iter().any(|route| {
                    route.serves().iter().any(|served| {
                        served.resource == row.resource && served.actions.contains(&rule.action)
                    })
                });
                if !served {
                    problems.push(format!(
                        "{:?}/{}: no route of the admin route table serves it; mark the route \
                         the admin calls for it with .serving({:?}, &[...])",
                        row.resource, rule.action, row.resource
                    ));
                }
            }
        }
        problems
    }

    #[test]
    fn every_access_row_agrees_with_the_routes_it_stands_for() {
        assert_eq!(
            access_route_problems(RESOURCE_ACCESS, &table()),
            Vec::<String>::new()
        );
    }

    const EDIT_POSTS: Requirement = Requirement::Permission(Permission::EditPosts);
    const PUBLISH_POSTS: Requirement = Requirement::Permission(Permission::PublishPosts);
    const EDIT_LABELS: Requirement = Requirement::Permission(Permission::EditLabels);

    /// A probe access table: invented names that no resource slice owns.
    const PROBE_ACCESS: &[ResourceAccess] = &[
        ResourceAccess {
            resource: "probe-posts",
            rules: &[
                ActionRule::new(AccessAction::List, &[EDIT_POSTS]),
                ActionRule::new(AccessAction::Publish, &[EDIT_POSTS, PUBLISH_POSTS]),
                ActionRule::new(AccessAction::Purge, &[Requirement::AdminRole]),
            ],
        },
        ResourceAccess {
            resource: "probe-labels",
            rules: &[ActionRule::new(AccessAction::List, &[EDIT_LABELS])],
        },
    ];

    /// Probe routes that serve every action of [`PROBE_ACCESS`] with the
    /// requirements its rules ask for, the labels' list for `edit posts`
    /// too, as a stand-in for a requirement that also lets the picker in.
    fn agreeing_routes() -> Vec<AdminRoute> {
        async fn probe() {}
        vec![
            AdminRoute::get("/probe-posts", EDIT_POSTS, probe)
                .serving("probe-posts", &[AccessAction::List]),
            AdminRoute::post("/probe-posts/{id}/publish", PUBLISH_POSTS, probe)
                .serving("probe-posts", &[AccessAction::Publish]),
            AdminRoute::delete("/probe-posts/{id}/purge", Requirement::AdminRole, probe)
                .serving("probe-posts", &[AccessAction::Purge]),
            AdminRoute::get("/probe-labels", Requirement::AnyActiveUser, probe)
                .serving("probe-labels", &[AccessAction::List]),
            // A route that no row describes needs no mark.
            AdminRoute::get("/probe-other", Requirement::AdminRole, probe),
        ]
    }

    #[test]
    fn the_access_check_accepts_routes_that_let_in_at_least_the_row_s_accounts() {
        assert_eq!(
            access_route_problems(PROBE_ACCESS, &agreeing_routes()),
            Vec::<String>::new()
        );
        // Nothing to check on either side.
        assert_eq!(access_route_problems(&[], &[]), Vec::<String>::new());
    }

    #[test]
    fn the_access_check_names_every_disagreement() {
        async fn probe() {}
        let cases: [(Vec<AdminRoute>, &str); 5] = [
            // The purge route is gone: the map would offer an action that
            // no route serves.
            (
                agreeing_routes()
                    .into_iter()
                    .filter(|route| !route.path().ends_with("/purge"))
                    .collect(),
                r#""probe-posts"/purge: no route of the admin route table serves it"#,
            ),
            // A route claims an action the row does not offer.
            (
                vec![
                    AdminRoute::delete("/probe-posts/{id}", EDIT_POSTS, probe)
                        .serving("probe-posts", &[AccessAction::Delete]),
                ],
                r#"DELETE /api/admin/probe-posts/{id}: serves "probe-posts"/delete, which no row"#,
            ),
            // A route claims a resource without a row.
            (
                vec![
                    AdminRoute::get("/probe-none", EDIT_POSTS, probe)
                        .serving("probe-none", &[AccessAction::List]),
                ],
                r#"serves "probe-none"/list, which no row"#,
            ),
            // A mark without an action.
            (
                vec![
                    AdminRoute::get("/probe-posts", EDIT_POSTS, probe).serving("probe-posts", &[]),
                ],
                r#"GET /api/admin/probe-posts: serves no action of "probe-posts""#,
            ),
            // The list route asks for more than the row: a writer would see
            // the entry and get 403.
            (
                vec![
                    AdminRoute::get("/probe-posts", Requirement::AdminRole, probe)
                        .serving("probe-posts", &[AccessAction::List]),
                ],
                r#""probe-posts"/list: the access map offers it to"#,
            ),
        ];
        for (routes, expected) in cases {
            let problems = access_route_problems(PROBE_ACCESS, &routes);
            assert!(
                problems.iter().any(|problem| problem.contains(expected)),
                "{expected}: {problems:?}"
            );
        }
        // Publishing asks for two permissions; a route that wants a third
        // is found too (a supervisor holds both and not `edit users`).
        let stricter_publish = vec![
            AdminRoute::post(
                "/probe-posts/{id}/publish",
                Requirement::Permission(Permission::EditUsers),
                probe,
            )
            .serving("probe-posts", &[AccessAction::Publish]),
        ];
        assert!(
            access_route_problems(PROBE_ACCESS, &stricter_publish)
                .iter()
                .any(|problem| problem.contains(r#""probe-posts"/publish: the access map offers"#)),
        );
    }

    #[test]
    fn the_weakest_account_finds_a_stricter_route_behind_a_long_rule() {
        async fn probe() {}
        // Four permissions that only the admin role holds together, so no
        // account of the matrix but an admin meets the rule, and the admin
        // role meets every permission route as well. Only the account with
        // exactly the four as grants shows the route's `edit notices`.
        const LONG: &[ResourceAccess] = &[ResourceAccess {
            resource: "probe-long",
            rules: &[ActionRule::new(
                AccessAction::List,
                &[
                    Requirement::Permission(Permission::EditUsers),
                    Requirement::Permission(Permission::ViewLogs),
                    Requirement::Permission(Permission::EditPages),
                    Requirement::Permission(Permission::EditMenu),
                ],
            )],
        }];
        let routes = vec![
            AdminRoute::get(
                "/probe-long",
                Requirement::Permission(Permission::EditNotices),
                probe,
            )
            .serving("probe-long", &[AccessAction::List]),
        ];
        let rule = &LONG[0].rules[0];
        assert!(
            probe_accounts()
                .iter()
                .all(|account| !rule.is_met_by(account)
                    || routes[0].requirement().is_met_by(account)),
            "the matrix alone would miss it"
        );
        let problems = access_route_problems(LONG, &routes);
        assert_eq!(problems.len(), 1, "{problems:?}");
        assert!(problems[0].contains("asks for the permission \"edit notices\""));
    }

    #[test]
    fn serving_records_the_resource_and_actions_in_order() {
        async fn probe() {}
        let route = AdminRoute::get("/probe", EDIT_POSTS, probe);
        assert_eq!(route.serves(), &[] as &[ServedAccess]);
        let route = route
            .serving("probe-a", &[AccessAction::List, AccessAction::Show])
            .serving("probe-b", &[AccessAction::History]);
        assert_eq!(
            route.serves(),
            &[
                ServedAccess {
                    resource: "probe-a",
                    actions: &[AccessAction::List, AccessAction::Show],
                },
                ServedAccess {
                    resource: "probe-b",
                    actions: &[AccessAction::History],
                },
            ]
        );
        let shown = format!("{route:?}");
        assert!(shown.contains(r#"resource: "probe-a""#), "{shown}");
        // The mark leaves the guard's facts alone.
        assert_eq!(route.requirement(), EDIT_POSTS);
        assert_eq!(route.scopes(), &[SessionScope::Admin]);
    }

    #[test]
    fn the_probe_accounts_hold_every_role_set_with_up_to_two_grants() {
        let accounts = probe_accounts();
        // 16 role sets, each with no grant, 16 single grants and 120 pairs.
        assert_eq!(accounts.len(), 16 * (1 + 16 + 120));
        assert!(accounts.contains(&AccountGrants::default()));
        assert!(accounts.iter().any(|account| account.roles.len() == 4
            && account.extra == BTreeSet::from([Permission::EditPosts, Permission::EditDiaktoll])));
        let weakest = weakest_accounts(&ActionRule::new(
            AccessAction::Purge,
            &[
                Requirement::AdminRole,
                EDIT_POSTS,
                Requirement::AnyActiveUser,
            ],
        ));
        assert_eq!(weakest.len(), 1);
        assert_eq!(weakest[0].roles, BTreeSet::from([Role::Admin]));
        assert_eq!(weakest[0].extra, BTreeSet::from([Permission::EditPosts]));
        // An "or" gives one account per choice, each with the other
        // requirements too; an empty one gives none.
        let choices = weakest_accounts(&ActionRule::new(
            AccessAction::List,
            &[
                Requirement::AnyPermission(&[Permission::EditLabels, Permission::EditPosts]),
                Requirement::Permission(Permission::ManageMedia),
            ],
        ));
        assert_eq!(
            choices
                .iter()
                .map(|account| account.extra.clone())
                .collect::<Vec<_>>(),
            [
                BTreeSet::from([Permission::EditLabels, Permission::ManageMedia]),
                BTreeSet::from([Permission::EditPosts, Permission::ManageMedia]),
            ]
        );
        assert_eq!(
            weakest_accounts(&ActionRule::new(
                AccessAction::List,
                &[Requirement::AnyPermission(&[])]
            )),
            Vec::<AccountGrants>::new()
        );
    }

    #[test]
    fn the_access_check_finds_a_picker_route_that_lets_in_fewer_than_its_row() {
        async fn probe() {}
        // The row offers the list to either permission; a route that asks
        // for one of them alone would answer the other one's sidebar entry
        // `403`.
        const EITHER: &[ResourceAccess] = &[ResourceAccess {
            resource: "probe-either",
            rules: &[ActionRule::new(
                AccessAction::List,
                &[Requirement::AnyPermission(&[
                    Permission::EditLabels,
                    Permission::EditPosts,
                ])],
            )],
        }];
        let narrower = vec![
            AdminRoute::get("/probe-either", EDIT_LABELS, probe)
                .serving("probe-either", &[AccessAction::List]),
        ];
        let problems = access_route_problems(EITHER, &narrower);
        assert!(
            problems
                .iter()
                .any(|problem| problem.contains(r#""probe-either"/list: the access map offers"#)),
            "{problems:?}"
        );
        // A route with the same "or", or a wider one, agrees.
        for requirement in [
            Requirement::AnyPermission(&[Permission::EditPosts, Permission::EditLabels]),
            Requirement::AnyActiveUser,
        ] {
            let routes = vec![
                AdminRoute::get("/probe-either", requirement, probe)
                    .serving("probe-either", &[AccessAction::List]),
            ];
            assert_eq!(
                access_route_problems(EITHER, &routes),
                Vec::<String>::new(),
                "{requirement}"
            );
        }
    }
}
