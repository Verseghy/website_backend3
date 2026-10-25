//! The committed configuration files load and say what the ADRs require.
//!
//! These tests read `config/<env>.yaml` the way Loco does. Values that the
//! files take from the environment through `get_env` (the database URI, for
//! example) are only checked through the guard, because the CI job may set
//! them. `config/production.yaml` needs variables without defaults; the CLI
//! tests in `tests/cli/` load it in a child process.

use std::net::Ipv4Addr;

use backend3::{app::enforce_database_guard, settings::Settings};
use loco_rs::{
    config::{CacheConfig, Config, QueueConfig, WorkerMode},
    environment::Environment,
};

fn load(environment: &Environment) -> Config {
    environment
        .load()
        .unwrap_or_else(|error| panic!("config/{environment}.yaml: {error}"))
}

fn local_environments() -> [Environment; 3] {
    [
        Environment::Development,
        Environment::Test,
        Environment::Any("e2e".to_owned()),
    ]
}

#[test]
fn local_configs_pass_the_guard_and_have_typed_settings() {
    for environment in local_environments() {
        let config = load(&environment);
        enforce_database_guard(&environment, &config)
            .unwrap_or_else(|error| panic!("{environment}: {error}"));
        let settings = Settings::from_section(config.settings.as_ref())
            .unwrap_or_else(|error| panic!("{environment}: {error}"));
        // ADR 0023 §9: a port of its own, reachable off the host in
        // production only.
        settings
            .check_server_port(config.server.port)
            .unwrap_or_else(|error| panic!("{environment}: {error}"));
        assert_eq!(
            settings.metrics.binding,
            Ipv4Addr::LOCALHOST,
            "{environment}"
        );
        assert!(!config.database.dangerously_truncate, "{environment}");
        assert!(!config.database.dangerously_recreate, "{environment}");
    }
}

#[test]
fn static_and_fallback_middlewares_are_disabled_and_headers_enabled() {
    for environment in local_environments() {
        let middlewares = load(&environment).server.middlewares;
        let static_assets = middlewares.static_assets.expect("static configured");
        assert!(!static_assets.enable, "{environment}: static (ADR 0004 §3)");
        let fallback = middlewares.fallback.expect("fallback configured");
        assert!(!fallback.enable, "{environment}: fallback (ADR 0004 §3)");
        let compression = middlewares.compression.expect("compression configured");
        assert!(
            compression.enable,
            "{environment}: compression (ADR 0004 §3)"
        );

        let secure = middlewares
            .secure_headers
            .expect("secure_headers configured");
        assert!(secure.enable, "{environment}: secure_headers (ADR 0023 §6)");
        assert_eq!(secure.preset, "github", "{environment}");
        let overrides = secure.overrides.expect("overrides");
        assert_eq!(
            overrides["Content-Security-Policy"],
            "default-src 'none'; frame-ancestors 'none'; base-uri 'none'; form-action 'none'",
            "{environment}"
        );
        assert_eq!(
            overrides["Referrer-Policy"],
            "strict-origin-when-cross-origin"
        );
    }
}

#[test]
fn test_config_is_safe_and_deterministic() {
    let config = load(&Environment::Test);
    assert_eq!(config.workers.mode, WorkerMode::ForegroundBlocking);
    assert!(config.queue.is_none());
    assert!(matches!(config.cache, CacheConfig::Null));
    assert!(config.database.auto_migrate);
    assert!(
        config.database.uri.starts_with("postgres://"),
        "Loco's per-test databases only work with the postgres:// scheme"
    );
    let settings = Settings::from_section(config.settings.as_ref()).expect("settings");
    assert_eq!(
        settings.admin_ui.dist_dir.to_str(),
        Some("tests/fixtures/admin-dist")
    );
}

#[test]
fn development_runs_the_production_queue_setup() {
    let config = load(&Environment::Development);
    assert_eq!(config.workers.mode, WorkerMode::BackgroundQueue);
    match config.queue {
        Some(QueueConfig::Postgres(queue)) => {
            assert_eq!(queue.num_workers, 1);
            assert_eq!(queue.max_connections, 2);
            assert_eq!(queue.poll_interval_sec, 5);
            assert!(!queue.dangerously_flush);
            let reaper = queue.reaper.expect("reaper");
            assert_eq!(reaper.age_minutes, 30);
            assert_eq!(reaper.interval_seconds, 300);
        }
        other => panic!("queue must be Postgres (ADR 0016), got {other:?}"),
    }
    match config.cache {
        CacheConfig::InMem(cache) => assert_eq!(cache.max_capacity, 1000),
        other => panic!("cache must be InMem with 1000 entries, got {other:?}"),
    }
}

#[test]
fn e2e_serves_the_admin_build_on_port_5150() {
    let config = load(&Environment::Any("e2e".to_owned()));
    assert_eq!(config.server.port, 5150);
    assert!(config.database.auto_migrate);
    let settings = Settings::from_section(config.settings.as_ref()).expect("settings");
    assert_eq!(settings.admin_ui.dist_dir.to_str(), Some("admin/dist"));
}
