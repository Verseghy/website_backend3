//! Helpers for the tests of the admin route table's guard (ADR 0007 §5).
//!
//! The admin API has no route yet that asks for a permission or the `admin`
//! role, so the guard is tested on probe tables: routes that the tests
//! declare with each kind of requirement, built by the same
//! `controllers::admin::routes::build` as the application's table and served
//! through Loco's own middleware stack ([`probe_server`]). The accounts are
//! invented and get their roles and extra grants straight in the tables
//! ([`account_with`]), and their tokens are signed with the test
//! configuration's key ([`session_token`]).

use axum_test::TestServer;
use backend3::{
    app::App,
    domain::{
        authz::{Permission, Role},
        session::SessionScope,
    },
    models::users::{ActiveModel, Model},
};
use chrono::DateTime;
use loco_rs::{
    app::AppContext,
    auth::jwt::JWT,
    controller::{AppRoutes, Routes},
};
use sea_orm::{ActiveModelTrait, ActiveValue, ConnectionTrait};
use serde_json::{Map, Value, json};

use super::test_server;

/// Loco's test server in front of `probes` alone, behind the application's
/// whole middleware stack (`App::middlewares`, `server.middlewares` of
/// `config/test.yaml`, the 128 MiB request body limit included) and the
/// trailing-slash normalisation of `App::serve`, with `ctx` as the state.
///
/// - `ctx`: the app context the routes run with; a test may pass a changed
///   copy, such as one with another `auth.jwt` section.
/// - `probes`: the routes to serve, usually `build` over a probe table.
///
/// # Panics
///
/// When Loco cannot build the router or the test server cannot start.
pub fn probe_server(ctx: &AppContext, probes: Routes) -> TestServer {
    let router = AppRoutes::empty()
        .add_route(probes)
        .to_router::<App>(ctx.clone(), axum::Router::new())
        .expect("build the probe router");
    test_server(router)
}

/// Inserts a live, active, invented account with `email` and no password,
/// holding `roles` and the extra grants `extra`, after `change` has adjusted
/// it (for example to deactivate it).
///
/// - `ctx`: the app context of the test's own database.
/// - `email`: a unique invented address.
/// - `roles`: the roles to give (`user_roles`).
/// - `extra`: the extra grants to give (`user_permissions`).
/// - `change`: any other change to the new row.
///
/// # Panics
///
/// When an insert fails.
pub async fn account_with(
    ctx: &AppContext,
    email: &str,
    roles: &[Role],
    extra: &[Permission],
    change: impl FnOnce(&mut ActiveModel),
) -> Model {
    let mut new = ActiveModel {
        email: ActiveValue::Set(Some(email.to_owned())),
        name: ActiveValue::Set("Próba Szerkesztő".to_owned()),
        ..Default::default()
    };
    change(&mut new);
    let user = new.insert(&ctx.db).await.expect("insert the account");
    give(ctx, &user, roles, extra).await;
    user
}

/// Gives `user` the roles `roles` and the extra grants `extra`, as an
/// account editor would (the values are the enums' own, so the CHECK and
/// the foreign key accept them).
///
/// - `ctx`: the app context of the test's own database.
/// - `user`: the account.
/// - `roles`: the roles to add.
/// - `extra`: the extra grants to add.
///
/// # Panics
///
/// When an insert fails, for example for a role the account already holds.
pub async fn give(ctx: &AppContext, user: &Model, roles: &[Role], extra: &[Permission]) {
    for role in roles {
        ctx.db
            .execute_unprepared(&format!(
                "INSERT INTO user_roles (user_id, role_id) VALUES ({}, {})",
                user.id,
                role.id()
            ))
            .await
            .expect("give a role");
    }
    for permission in extra {
        ctx.db
            .execute_unprepared(&format!(
                "INSERT INTO user_permissions (user_id, permission) VALUES ({}, '{}')",
                user.id,
                permission.name()
            ))
            .await
            .expect("give an extra grant");
    }
}

/// Takes every role and extra grant away from `user`.
///
/// - `ctx`: the app context of the test's own database.
/// - `user`: the account.
///
/// # Panics
///
/// When a delete fails.
pub async fn take_all(ctx: &AppContext, user: &Model) {
    for table in ["user_roles", "user_permissions"] {
        ctx.db
            .execute_unprepared(&format!("DELETE FROM {table} WHERE user_id = {}", user.id))
            .await
            .expect("take the grants away");
    }
}

/// A session token for `user` with `scope`, signed with the test
/// configuration's `auth.jwt` key: what the sign-in issues for a full
/// session, or what the forced password change will issue for a
/// `password_change` one (ADR 0006 §4).
///
/// - `ctx`: the app context, for the key.
/// - `user`: the account; the token carries its current `token_version`.
/// - `scope`: the session's scope.
///
/// # Panics
///
/// When the configuration has no `auth.jwt` section or the token cannot be
/// signed.
pub fn session_token(ctx: &AppContext, user: &Model, scope: SessionScope) -> String {
    let secret = &ctx.config.get_jwt_config().expect("auth.jwt").secret;
    // A fixed `auth_time`: nothing the guard checks reads it, and tests
    // read the wall clock only where recipe 7 of the project skill says so.
    // Loco still sets `exp` from the system time (its step 4).
    let auth_time = DateTime::parse_from_rfc3339("2026-10-25T08:00:00Z").expect("RFC 3339");
    let claims = json!({
        "tv": user.token_version,
        "scope": scope.claim(),
        "auth_time": auth_time.timestamp(),
    });
    let Value::Object(claims) = claims else {
        unreachable!("json! of an object is an object");
    };
    let claims: Map<String, Value> = claims;
    JWT::new(secret)
        .generate_token(7200, user.pid.to_string(), claims)
        .expect("sign the token")
}
