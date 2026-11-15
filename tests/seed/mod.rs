//! The development, test and E2E seed, `Hooks::seed` (`src/domain/seed.rs`).
//!
//! The seed creates the invented account of `SEED_ACCOUNT` in development,
//! test and e2e, with the `admin` role, signs in through the browser sign-in
//! like any account,
//! leaves an existing account alone, and refuses every other environment
//! before it writes anything: production never holds the account
//! (ADR 0040). `tests/cli/precheck.rs` covers the CLI
//! refusal in production, which stops `db seed` before it connects.
//!
//! The seed also creates the invented labels of `SEED_LABELS` once, which the
//! E2E suite lists, and leaves a live label with one of their names alone.
//!
//! The E2E suite types the account into the sign-in screen and checks the
//! label list against the seed's labels, so its files
//! `admin/e2e/seed-account.ts` and `admin/e2e/seed-labels.ts` must hold the
//! same values; the two `the_e2e_suite_holds_*` tests check that.

use std::path::Path;

use axum::http::{StatusCode, header::COOKIE};
use backend3::{
    app::App,
    domain::{
        authz::{AccessAction, AccountGrants, Permission, RESOURCE_ACCESS, Role},
        password_hash::{self, PasswordHashKind},
        seed::{SEED_ACCOUNT, SEED_LABELS},
        session_cookie::SESSION_COOKIE_NAME,
    },
    models::{
        labels,
        users::{self, Entity, Model},
    },
};
use loco_rs::{
    app::{AppContext, Hooks},
    environment::Environment,
};
use sea_orm::{ConnectionTrait, EntityTrait, PaginatorTrait};
use serde_json::{Map, Value, json};

use crate::support::{repository_root, with_app, with_server};

/// The `--from` directory Loco passes by default; backend3's seed ignores it.
const DEFAULT_FROM: &str = "src/fixtures";

/// Runs `Hooks::seed` on `ctx`, as `cargo loco db seed` does.
async fn seed(ctx: &AppContext) -> loco_rs::Result<()> {
    App::seed(ctx, Path::new(DEFAULT_FROM)).await
}

/// `ctx` with its environment replaced, so the hook sees another
/// environment while it still writes to this test's own local database.
fn in_environment(ctx: &AppContext, environment: Environment) -> AppContext {
    let mut other = ctx.clone();
    other.environment = environment;
    other
}

/// How many accounts the database holds.
async fn account_count(ctx: &AppContext) -> u64 {
    Entity::find().count(&ctx.db).await.expect("count users")
}

/// How many labels the database holds, live or trashed.
async fn label_count(ctx: &AppContext) -> u64 {
    labels::Entity::find()
        .count(&ctx.db)
        .await
        .expect("count labels")
}

/// The live labels as `(id, name, color)`, by id.
async fn live_labels(ctx: &AppContext) -> Vec<(i64, String, String)> {
    let mut rows: Vec<_> = labels::Model::find_live()
        .all(&ctx.db)
        .await
        .expect("live labels")
        .into_iter()
        .map(|label| (label.id, label.name, label.color))
        .collect();
    rows.sort();
    rows
}

/// The seeded account, found as sign-in finds it.
async fn seeded_account(ctx: &AppContext) -> Model {
    Model::find_live_by_email(&ctx.db, SEED_ACCOUNT.email)
        .await
        .expect("the seed account exists")
}

#[tokio::test]
async fn the_seed_creates_the_invented_account_with_a_hash_made_at_seed_time() {
    with_app(|_router, ctx| async move {
        seed(&ctx).await.expect("the test environment seeds");

        assert_eq!(account_count(&ctx).await, 1);
        let user = seeded_account(&ctx).await;
        assert_eq!(user.name, SEED_ACCOUNT.name);
        assert_eq!(user.email.as_deref(), Some(SEED_ACCOUNT.email));
        let hash = user.password_hash.as_deref().expect("a password hash");
        assert_eq!(password_hash::classify(hash), PasswordHashKind::Argon2id);
        assert!(user.verify_password(SEED_ACCOUNT.password));
        // A full admin session from the password alone (ADR 0006 §4).
        assert!(user.active);
        assert!(!user.must_change_password);
        assert!(user.totp_enabled_at.is_none());
        assert!(user.deleted_at.is_none());
        assert!(user.anonymised_at.is_none());
        assert_eq!(user.created_by, None, "the system created it");
        // The admin role and no extra grant (ADR 0007 §3).
        assert_eq!(
            user.grants(&ctx.db).await.expect("grants"),
            AccountGrants {
                roles: [Role::Admin].into(),
                extra: [].into(),
            }
        );
        assert!(
            users::Model::authenticate(&ctx.db, SEED_ACCOUNT.email, SEED_ACCOUNT.password, true)
                .await
                .expect("authenticate")
                .accepted()
                .is_some()
        );
    })
    .await;
}

#[tokio::test]
async fn seeding_twice_leaves_the_one_account_unchanged() {
    with_app(|_router, ctx| async move {
        seed(&ctx).await.expect("first seed");
        let first = seeded_account(&ctx).await;

        seed(&ctx).await.expect("second seed");

        assert_eq!(account_count(&ctx).await, 1);
        // Not even rehashed: the row is exactly the one the first run wrote.
        assert_eq!(seeded_account(&ctx).await, first);
    })
    .await;
}

#[tokio::test]
async fn the_seed_creates_the_invented_labels_in_order_and_only_once() {
    with_app(|_router, ctx| async move {
        seed(&ctx).await.expect("first seed");

        let first = live_labels(&ctx).await;
        // In the order of SEED_LABELS, so their ids grow down the list.
        assert_eq!(
            first
                .iter()
                .map(|(_, name, color)| (name.as_str(), color.as_str()))
                .collect::<Vec<_>>(),
            SEED_LABELS
                .iter()
                .map(|label| (label.name, label.color))
                .collect::<Vec<_>>()
        );
        let all = labels::Entity::find().all(&ctx.db).await.expect("labels");
        for label in &all {
            // Made by the system, at PostgreSQL's clock (D90), and live.
            assert_eq!(label.created_by, None, "{}", label.name);
            assert_eq!(label.updated_by, None, "{}", label.name);
            assert!(label.deleted_at.is_none(), "{}", label.name);
        }

        seed(&ctx).await.expect("second seed");

        // Not one more row, and the rows are the first run's.
        assert_eq!(label_count(&ctx).await, SEED_LABELS.len() as u64);
        assert_eq!(live_labels(&ctx).await, first);
    })
    .await;
}

#[tokio::test]
async fn the_seed_leaves_a_live_label_of_the_same_name_alone() {
    with_app(|_router, ctx| async move {
        // A live label that one seed label names but for case, and a
        // trashed one with another seed label's name, written in SQL as
        // the importer and the trash will write them. Invented names.
        let taken = SEED_LABELS[0];
        let trashed = SEED_LABELS[1];
        for statement in [
            format!(
                "INSERT INTO labels (id, name, color) VALUES (900, '{}', '#000000')",
                taken.name.to_uppercase()
            ),
            format!(
                "INSERT INTO labels (id, name, color, deleted_at) VALUES \
                 (901, '{}', '#000000', TIMESTAMPTZ '2026-11-15 08:00:00+00')",
                trashed.name
            ),
        ] {
            ctx.db
                .execute_unprepared(&statement)
                .await
                .unwrap_or_else(|error| panic!("{statement}: {error}"));
        }

        seed(&ctx).await.expect("seed");

        let live = live_labels(&ctx).await;
        // The live label stays as it was, and the seed did not add its name.
        assert!(live.contains(&(900, taken.name.to_uppercase(), "#000000".to_owned())));
        assert!(
            !live.iter().any(|(_, name, _)| name.as_str() == taken.name),
            "{live:?}"
        );
        // The trashed label does not hold its name: the seed adds it.
        assert!(
            live.iter()
                .any(|(_, name, _)| name.as_str() == trashed.name),
            "{live:?}"
        );
        assert_eq!(live.len(), SEED_LABELS.len());
        assert_eq!(label_count(&ctx).await, SEED_LABELS.len() as u64 + 1);
    })
    .await;
}

#[tokio::test]
async fn the_seed_runs_in_development_test_and_e2e() {
    with_app(|_router, ctx| async move {
        for environment in [
            Environment::Development,
            Environment::Test,
            Environment::Any("e2e".to_owned()),
        ] {
            let name = environment.to_string();
            seed(&in_environment(&ctx, environment))
                .await
                .unwrap_or_else(|error| panic!("{name}: {error}"));
        }
        assert_eq!(account_count(&ctx).await, 1);
        assert_eq!(label_count(&ctx).await, SEED_LABELS.len() as u64);
    })
    .await;
}

/// Production, staging (which runs the production configuration, but could
/// also be started under its own name) and every other name are refused
/// before the hook writes anything, with a message that says why and what
/// to do.
#[tokio::test]
async fn the_seed_refuses_every_other_environment_before_writing() {
    with_app(|_router, ctx| async move {
        for environment in [
            Environment::Production,
            Environment::Any("staging".to_owned()),
            Environment::Any("Production".to_owned()),
            Environment::Any("e2e-staging".to_owned()),
        ] {
            let name = environment.to_string();
            let error = seed(&in_environment(&ctx, environment))
                .await
                .expect_err(&name)
                .to_string();
            assert!(
                error.contains(&format!(
                    "Refusing to seed: the environment is {name:?}, and backend3 seeds only \
                     development, test, e2e."
                )),
                "{name}: {error}"
            );
            assert!(error.contains("cut-over import"), "{name}: {error}");
            assert_eq!(account_count(&ctx).await, 0, "{name} wrote an account");
            assert_eq!(label_count(&ctx).await, 0, "{name} wrote a label");
        }
    })
    .await;
}

/// The seeded account signs in through the browser sign-in, and `/auth/me`
/// answers it: the flow the E2E suite drives through the admin.
#[tokio::test]
async fn the_seed_account_signs_in_through_the_admin_api() {
    with_server(|server, ctx| async move {
        seed(&ctx).await.expect("seed");
        let user = seeded_account(&ctx).await;

        let login = server
            .post("/api/admin/auth/login")
            .json(&json!({ "email": SEED_ACCOUNT.email, "password": SEED_ACCOUNT.password }))
            .await;

        assert_eq!(login.status_code(), StatusCode::OK);
        assert_eq!(login.json::<Value>(), json!({ "step": "done" }));
        let set_cookie = login.header("set-cookie");
        let token = set_cookie
            .to_str()
            .expect("ASCII")
            .split_once(';')
            .and_then(|(pair, _)| pair.strip_prefix(&format!("{SESSION_COOKIE_NAME}=")))
            .expect("the session cookie")
            .to_owned();
        let me = server
            .get("/api/admin/auth/me")
            .add_header(COOKIE, format!("{SESSION_COOKIE_NAME}={token}"))
            .await;
        assert_eq!(me.status_code(), StatusCode::OK);
        assert_eq!(
            me.json::<Value>(),
            json!({
                "pid": user.pid.to_string(),
                "name": SEED_ACCOUNT.name,
                "email": SEED_ACCOUNT.email,
                "scope": "admin",
                // The admin role, and with it every permission (ADR 0007
                // §2), so the E2E suite reaches every screen.
                "roles": ["admin"],
                "permissions": Permission::ALL.map(Permission::name),
                "is_admin": true,
                // And so every action of every row of the access table
                // (ADR 0007 §7), in the ADR's order.
                "access": every_action_of_every_resource(),
            })
        );
    })
    .await;
}

/// `admin/e2e/seed-account.ts` holds the values the E2E suite types into the
/// sign-in screen. They must be the seed's, or every signed-in E2E test would
/// fail with a wrong password instead of naming the cause.
#[test]
fn the_e2e_suite_holds_the_seed_account() {
    let path = repository_root().join("admin/e2e/seed-account.ts");
    let source = std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("{}: {error}", path.display()));
    for (field, value) in [
        ("name", SEED_ACCOUNT.name),
        ("email", SEED_ACCOUNT.email),
        ("password", SEED_ACCOUNT.password),
    ] {
        let line = format!("  {field}: {value:?},");
        assert!(
            source.lines().any(|candidate| candidate == line),
            "{} must contain the line `{line}`, the {field} of SEED_ACCOUNT in \
             src/domain/seed.rs",
            path.display()
        );
    }
}

/// `admin/e2e/seed-labels.ts` holds the labels the E2E suite expects the
/// label list to show. They must be the seed's, or the list tests would
/// fail on a page size or an order instead of naming the cause.
#[test]
fn the_e2e_suite_holds_the_seed_labels() {
    let path = repository_root().join("admin/e2e/seed-labels.ts");
    let source = std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("{}: {error}", path.display()));
    let lines: Vec<&str> = source
        .lines()
        .filter(|line| line.starts_with("  { name: "))
        .collect();
    let expected: Vec<String> = SEED_LABELS
        .iter()
        .map(|label| format!("  {{ name: {:?}, color: {:?} }},", label.name, label.color))
        .collect();
    assert_eq!(
        lines,
        expected,
        "{} must list the labels of SEED_LABELS in src/domain/seed.rs, one line each, in \
         the same order",
        path.display()
    );
}

/// Every action of every row of the access table, as `/auth/me` sends it to
/// an account with the `admin` role: read from the table itself, not
/// through the access map, so the seed test checks the map independently.
fn every_action_of_every_resource() -> Value {
    RESOURCE_ACCESS
        .iter()
        .map(|row| {
            let mut actions: Vec<_> = row.rules.iter().map(|rule| rule.action).collect();
            actions.sort();
            let names: Vec<&str> = actions.into_iter().map(AccessAction::name).collect();
            (row.resource.to_owned(), json!(names))
        })
        .collect::<Map<String, Value>>()
        .into()
}
