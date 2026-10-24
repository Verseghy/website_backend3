//! In production the server refuses to start without the built admin UI, and
//! the error names the configured path and the setting key (ADR 0004 §4).
//! The check runs where the router is built (`Hooks::before_routes`).

use backend3::app::App;
use loco_rs::{app::Hooks, environment::Environment};
use serde_json::json;

use crate::support::with_app;

/// Boots the test app, then calls `before_routes` again with the given
/// environment and admin UI directory, as the boot of that environment would.
async fn build_router(environment: Environment, dist_dir: &str) -> Result<(), String> {
    let dist_dir = dist_dir.to_owned();
    with_app(|_router, mut ctx| async move {
        ctx.environment = environment;
        ctx.config.settings = Some(json!({ "admin_ui": { "dist_dir": dist_dir } }));
        App::before_routes(&ctx)
            .await
            .map(|_| ())
            .map_err(|error| error.to_string())
    })
    .await
}

#[tokio::test]
async fn production_refuses_a_missing_dist_directory() {
    let message = build_router(Environment::Production, "tests/fixtures/no-such-dist")
        .await
        .expect_err("production must refuse to start");
    assert!(
        message.starts_with("Refusing to start: the admin UI directory"),
        "{message}"
    );
    assert!(
        message.contains("'tests/fixtures/no-such-dist'"),
        "{message}"
    );
    assert!(message.contains("settings.admin_ui.dist_dir"), "{message}");
}

#[tokio::test]
async fn production_refuses_a_dist_directory_without_index_html() {
    let message = build_router(Environment::Production, "tests/fixtures/admin-dist/assets")
        .await
        .expect_err("production must refuse to start");
    assert!(message.contains("has no index.html"), "{message}");
    assert!(
        message.contains("'tests/fixtures/admin-dist/assets'"),
        "{message}"
    );
    assert!(message.contains("settings.admin_ui.dist_dir"), "{message}");
}

#[tokio::test]
async fn production_starts_with_a_complete_dist_directory() {
    build_router(Environment::Production, "tests/fixtures/admin-dist")
        .await
        .expect("a complete directory is accepted");
}

#[tokio::test]
async fn other_environments_start_without_the_admin_ui() {
    for environment in [
        Environment::Development,
        Environment::Test,
        Environment::Any("e2e".into()),
    ] {
        let name = environment.to_string();
        build_router(environment, "tests/fixtures/no-such-dist")
            .await
            .unwrap_or_else(|error| panic!("{name}: {error}"));
    }
}
