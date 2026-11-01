//! The committed configuration files load and say what the ADRs require.
//!
//! These tests read `config/<env>.yaml` the way Loco does, as committed: a
//! personal `config/<env>.local.yaml` that Loco would merge over it is left
//! out. Values that the files take from the environment through `get_env`
//! (the database URI, for example) are only checked through the guard,
//! because the CI job may set them. `config/production.yaml` needs
//! variables without defaults; the CLI tests in `tests/cli/` and
//! `totp_encryption_key.rs` load it in a child process. `published_keys.rs`
//! checks every key value these files hold against the production refusal.

use std::net::Ipv4Addr;

use backend3::{
    app::enforce_database_guard,
    domain::{
        image_optimise::PngEffort,
        session_jwt::SESSION_LIFETIME_SECS,
        totp_secret::{PUBLISHED_LOCAL_KEY, PUBLISHED_LOCAL_KEY_TEXT, TotpEncryptionKey},
    },
    settings::{Settings, SettingsError, check_session_jwt},
};
use loco_rs::{
    config::{CacheConfig, Config, QueueConfig, WorkerMode},
    environment::Environment,
};

use crate::support::{cli_totp_key, load_committed_config, repository_root};

/// The committed `config/<environment>.yaml`, without a developer's
/// git-ignored `<environment>.local.yaml` merged over it
/// ([`load_committed_config`]): these tests check what the repository
/// publishes.
///
/// - `environment`: the environment whose file is loaded.
fn load(environment: &Environment) -> Config {
    load_committed_config(environment)
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
        // The committed effort is the documented default of ADR 0033 §5.
        assert_eq!(
            settings.image_optimisation.png,
            PngEffort::DEFAULT,
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
        // public-api.md §9: the 304 of a public answer. Loco turns the
        // middleware on when the key is absent, but a key without
        // `enable: true` turns it off, so the files write it out.
        let etag = middlewares.etag.expect("etag configured");
        assert!(etag.enable, "{environment}: etag (public-api.md §9)");

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

#[test]
fn local_configs_use_the_published_throwaway_totp_key() {
    // ADR 0006 §6 and ADR 0034 §2 item 3: the local environments carry one
    // fixed value that is no secret and says so when decoded, so nobody
    // mistakes it for a real key. The local environments keep it, and
    // production refuses the same settings (ADR 0034 §3).
    let text = std::str::from_utf8(PUBLISHED_LOCAL_KEY_TEXT).expect("readable text");
    assert!(text.contains("public and throwaway"), "{text}");
    let expected = TotpEncryptionKey::from_base64(PUBLISHED_LOCAL_KEY).expect("a valid key");
    for environment in local_environments() {
        let settings = Settings::from_section(load(&environment).settings.as_ref())
            .unwrap_or_else(|error| panic!("{environment}: {error}"));
        assert_eq!(
            settings.admin_auth.totp_encryption_key, expected,
            "{environment}"
        );
        let config = load(&environment);
        let jwt_secret = check_session_jwt(config.auth.as_ref())
            .unwrap_or_else(|error| panic!("{environment}: {error}"));
        settings
            .refuse_published_keys(&environment, &jwt_secret)
            .unwrap_or_else(|error| panic!("{environment}: {error}"));
        assert!(
            matches!(
                settings.refuse_published_keys(&Environment::Production, &jwt_secret),
                Err(SettingsError::PublishedTotpKey)
            ),
            "{environment}: production must refuse this key"
        );
    }
}

#[test]
fn local_configs_pass_the_session_check_with_their_own_published_secret() {
    // ADR 0006 §1 and ADR 0034 §2 item 3: each local environment signs its
    // sessions with a throwaway secret of its own, published in its file,
    // which the local environment keeps and production refuses by name
    // (ADR 0034 §3). The lifetime and the locations are the ones of
    // ADR 0006 §1, or the check would fail.
    for environment in local_environments() {
        let config = load(&environment);
        let jwt = config.get_jwt_config().expect("auth.jwt");
        assert_eq!(jwt.expiration, SESSION_LIFETIME_SECS, "{environment}");
        let secret = check_session_jwt(config.auth.as_ref())
            .unwrap_or_else(|error| panic!("{environment}: {error}"));
        let file = format!("config/{environment}.yaml");
        assert_eq!(secret.published_in(), Some(file.as_str()), "{environment}");

        // With a TOTP key of its own, production still refuses the secret,
        // and names the file it comes from.
        let mut section = config.settings.clone().expect("settings");
        section["admin_auth"]["totp_encryption_key"] = cli_totp_key().into();
        let settings = Settings::from_section(Some(&section)).expect("settings");
        settings
            .refuse_published_keys(&environment, &secret)
            .unwrap_or_else(|error| panic!("{environment}: {error}"));
        match settings.refuse_published_keys(&Environment::Production, &secret) {
            Err(SettingsError::PublishedJwtSecret { published_in }) => {
                assert_eq!(published_in, file, "{environment}");
            }
            other => panic!("{environment}: production must refuse this secret, got {other:?}"),
        }
    }
}

#[test]
fn production_reads_the_jwt_secret_from_the_environment_without_a_default() {
    // As for the TOTP key below: no default (ADR 0023 §11), and the escaping
    // filters, without which a value with a quote fails as a YAML error that
    // does not name auth.jwt.secret (tests/cli/session_secret.rs runs it end
    // to end). The lifetime and the locations are fixed in the file.
    let source = std::fs::read_to_string(repository_root().join("config/production.yaml"))
        .expect("config/production.yaml");
    let lines: Vec<&str> = source
        .lines()
        .filter(|line| line.trim_start().starts_with("secret:"))
        .collect();
    assert_eq!(
        lines,
        [
            r#"    secret: "<%= get_env(name='JWT_SECRET') | replace(from='\\', to='\\\\') | replace(from='\"', to='\\\"') %>""#
        ]
    );
    assert!(
        source.contains(
            "auth:\n  jwt:\n"
        ) && source.contains(
            "    expiration: 7200\n"
        ) && source.contains(
            "    location:\n      - from: Cookie\n        name: __Host-admin_session\n      - from: Bearer\n"
        ),
        "config/production.yaml: auth.jwt as ADR 0006 §1 fixes it"
    );
}

#[test]
fn production_reads_the_totp_key_from_the_environment_without_a_default() {
    // A secret takes no default (ADR 0023 §11): an unset variable must stop
    // the process, never fall back to the local value. The escaping filters
    // are pinned too, because without them a value with a quote fails as a
    // YAML error that does not name the key. The file is read as text
    // because loading it needs the variable (tests/boot/totp_encryption_key.rs
    // runs it end to end).
    let source = std::fs::read_to_string(repository_root().join("config/production.yaml"))
        .expect("config/production.yaml");
    let lines: Vec<&str> = source
        .lines()
        .filter(|line| line.trim_start().starts_with("totp_encryption_key:"))
        .collect();
    assert_eq!(
        lines,
        [
            r#"    totp_encryption_key: "<%= get_env(name='TOTP_ENCRYPTION_KEY') | replace(from='\\', to='\\\\') | replace(from='\"', to='\\\"') %>""#
        ]
    );
}

#[test]
fn test_and_e2e_configure_no_import_source() {
    // data-migration.md §4.1: these files never read
    // IMPORT_SOURCE_DATABASE_URL, so a variable left in a shell cannot point
    // a test run or the E2E server at a source; the import task's tests set
    // the source in the context they run it with.
    for environment in [Environment::Test, Environment::Any("e2e".to_owned())] {
        let settings = Settings::from_section(load(&environment).settings.as_ref())
            .unwrap_or_else(|error| panic!("{environment}: {error}"));
        assert!(
            !settings.import.source_database_url.is_set(),
            "{environment}: {:?}",
            settings.import.source_database_url
        );
    }
}

#[test]
fn production_and_development_read_the_import_source_with_an_unset_default() {
    // data-migration.md §4.1: the source comes from IMPORT_SOURCE_DATABASE_URL
    // and nowhere else, its default is the empty string (unset), and it is
    // quoted and escaped like the TOTP key, so any value reaches the task's
    // own check. Read as text, because the variable decides the loaded value
    // (tests/cli/import_mysql.rs runs production end to end).
    for file in ["config/production.yaml", "config/development.yaml"] {
        let source = std::fs::read_to_string(repository_root().join(file)).expect(file);
        let lines: Vec<&str> = source
            .lines()
            .filter(|line| line.trim_start().starts_with("source_database_url:"))
            .collect();
        assert_eq!(
            lines,
            [
                r#"    source_database_url: "<%= get_env(name='IMPORT_SOURCE_DATABASE_URL', default='') | replace(from='\\', to='\\\\') | replace(from='\"', to='\\\"') %>""#
            ],
            "{file}"
        );
    }
}
