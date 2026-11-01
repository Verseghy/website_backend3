//! Production takes the base of the public media URLs from
//! `MEDIA_PUBLIC_BASE_URL`, which has no default (public-api.md §2.3,
//! ADR 0015 §3): every command stops before it connects anywhere when the
//! variable is unset, and when its value is not an absolute http(s) URL in
//! canonical form without a trailing slash, with a message that names the
//! variable or the key and says what to write. `MEDIA_SERVE_STORAGE`, the
//! switch of the ADR 0015 §4 fallback, is checked the same way.
//!
//! The variables are read by config/production.yaml through `get_env`, so
//! these tests run the built `backend3-cli` with an explicit environment
//! (tests/support `run_cli`); the database points at a [`ConnectionProbe`],
//! and the positive controls show that a valid value gets as far as
//! connecting.

use std::time::Duration;

use crate::support::{
    CLI_MEDIA_PUBLIC_BASE_URL, CliRun, ConnectionProbe, cli_jwt_secret, cli_totp_key, run_cli,
};

const TIMEOUT: Duration = Duration::from_secs(60);

/// Runs `db status` in production with every other variable that
/// config/production.yaml requires, `MEDIA_PUBLIC_BASE_URL` set to
/// `base_url` (unset for `None`) and `extra` added, and returns the run with
/// whether the database probe saw a connection.
fn db_status_with(base_url: Option<&str>, extra: &[(&str, &str)]) -> (CliRun, bool) {
    let probe = ConnectionProbe::bind("127.0.0.1");
    let url = format!(
        "postgres://loco:loco@127.0.0.1:{}/backend3_test",
        probe.port()
    );
    // Production refuses a committed key, so both keys are made now
    // (ADR 0034 §3).
    let jwt_secret = cli_jwt_secret();
    let totp_key = cli_totp_key();
    let mut env = vec![
        ("LOCO_ENV", "production"),
        ("DATABASE_URL", url.as_str()),
        ("HOST", "http://localhost"),
        ("JWT_SECRET", jwt_secret.as_str()),
        ("TOTP_ENCRYPTION_KEY", totp_key.as_str()),
        // The probe never answers the PostgreSQL handshake; give up fast.
        ("DB_CONNECT_TIMEOUT", "300"),
    ];
    if let Some(base_url) = base_url {
        env.push(("MEDIA_PUBLIC_BASE_URL", base_url));
    }
    env.extend_from_slice(extra);
    let run = run_cli(&env, &["db", "status"], TIMEOUT);
    let connected = probe.was_connected();
    (run, connected)
}

#[test]
fn an_unset_base_url_stops_production_before_connecting() {
    let (run, connected) = db_status_with(None, &[]);
    assert!(!run.status.success(), "{}", run.stderr);
    assert!(
        run.stderr
            .contains("environment variable `MEDIA_PUBLIC_BASE_URL` not found"),
        "{}",
        run.stderr
    );
    assert!(!connected, "connected without a media base URL");
}

#[test]
fn an_unusable_base_url_stops_production_and_names_the_key() {
    for (value, fix) in [
        ("", "is empty"),
        (
            "backend.microshift.verseghy-gimnazium.net/storage",
            "is not an absolute URL",
        ),
        (
            "https://backend.microshift.verseghy-gimnazium.net/storage/",
            "remove the trailing `/`",
        ),
        (
            "https://Backend.microshift.verseghy-gimnazium.net/storage",
            "write it as https://backend.microshift.verseghy-gimnazium.net/storage",
        ),
        // Quotes and backslashes reach the typed check through the escaping
        // filters of config/production.yaml.
        (
            "https://example.org/\"storage\\",
            "write it as https://example.org/%22storage",
        ),
    ] {
        let (run, connected) = db_status_with(Some(value), &[]);
        assert!(!run.status.success(), "{value}: {}", run.stderr);
        assert!(
            run.stderr
                .contains("invalid configuration at `settings.media.public_base_url`"),
            "{value}: {}",
            run.stderr
        );
        assert!(
            run.stderr.contains("MEDIA_PUBLIC_BASE_URL in production"),
            "{}",
            run.stderr
        );
        assert!(run.stderr.contains(fix), "{value}: {}", run.stderr);
        assert!(!connected, "{value}: connected with an unusable base URL");
    }
}

#[test]
fn the_storage_switch_is_a_boolean() {
    let (run, connected) = db_status_with(
        Some(CLI_MEDIA_PUBLIC_BASE_URL),
        &[("MEDIA_SERVE_STORAGE", "yes")],
    );
    assert!(!run.status.success(), "{}", run.stderr);
    assert!(
        run.stderr
            .contains("invalid configuration at `settings.media.serve_storage`"),
        "{}",
        run.stderr
    );
    assert!(!connected, "connected with a switch that is no boolean");
}

#[test]
fn a_valid_base_url_gets_as_far_as_connecting() {
    // The positive controls: the production value, with the fallback off
    // (the default) and on.
    for extra in [&[][..], &[("MEDIA_SERVE_STORAGE", "true")][..]] {
        let (run, connected) = db_status_with(Some(CLI_MEDIA_PUBLIC_BASE_URL), extra);
        assert!(
            !run.status.success(),
            "the probe never completes a handshake"
        );
        assert!(
            !run.stderr.contains("settings.media"),
            "{extra:?}: {}",
            run.stderr
        );
        assert!(connected, "{extra:?}: {}", run.stderr);
    }
}
