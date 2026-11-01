//! With `LOCO_ENV=production`, `backend3-cli db reset` (and `db truncate`,
//! `db seed --reset`) exits with an error before it connects
//! (ADR 0024 §4, "What must be true in code and tests"), and so does
//! `doctor --config`, which would print every secret of the configuration
//! (ADR 0039).

use std::time::Duration;

use loco_rs::environment::Environment;

use crate::support::{ConnectionProbe, cli_jwt_secret, cli_totp_key, run_cli};

const TIMEOUT: Duration = Duration::from_secs(60);

/// The environment of a production pod, with the database pointed at
/// `probe`. `HOST`, `JWT_SECRET` and `TOTP_ENCRYPTION_KEY` are required by
/// config/production.yaml; both keys are made when the test runs, because
/// production refuses a committed one (ADR 0034 §3).
fn production_env(database_url: &str) -> Vec<(&'static str, String)> {
    vec![
        ("LOCO_ENV", "production".to_owned()),
        ("DATABASE_URL", database_url.to_owned()),
        ("HOST", "http://localhost".to_owned()),
        ("JWT_SECRET", cli_jwt_secret()),
        ("TOTP_ENCRYPTION_KEY", cli_totp_key()),
        // Keep the positive control fast: the probe never answers the
        // PostgreSQL handshake, so the driver gives up after this.
        ("DB_CONNECT_TIMEOUT", "300".to_owned()),
    ]
}

fn as_refs<'a>(env: &'a [(&'static str, String)]) -> Vec<(&'static str, &'a str)> {
    env.iter()
        .map(|(key, value)| (*key, value.as_str()))
        .collect()
}

#[test]
fn destructive_db_commands_are_refused_in_production_before_connecting() {
    for (args, refused) in [
        (&["db", "reset"][..], "db reset"),
        (&["db", "truncate"][..], "db truncate"),
        (&["db", "seed", "--reset"][..], "db seed --reset"),
        (&["db", "seed", "-r"][..], "db seed --reset"),
    ] {
        let probe = ConnectionProbe::bind("127.0.0.1");
        let url = format!(
            "postgres://loco:loco@127.0.0.1:{}/backend3_test",
            probe.port()
        );
        let env = production_env(&url);

        let run = run_cli(&as_refs(&env), args, TIMEOUT);

        assert_eq!(run.status.code(), Some(2), "{args:?}: {}", run.stderr);
        assert!(
            run.stderr.contains(&format!("Refusing to run `{refused}`")),
            "{args:?}: {}",
            run.stderr
        );
        assert!(
            run.stderr.contains("from LOCO_ENV"),
            "{args:?}: {}",
            run.stderr
        );
        assert!(!probe.was_connected(), "{args:?} connected to the database");
    }
}

#[test]
fn the_environment_option_selects_production_too() {
    let probe = ConnectionProbe::bind("127.0.0.1");
    let url = format!(
        "postgres://loco:loco@127.0.0.1:{}/backend3_test",
        probe.port()
    );
    let mut env = production_env(&url);
    env.retain(|(key, _)| *key != "LOCO_ENV");

    let run = run_cli(
        &as_refs(&env),
        &["--environment", "production", "db", "reset"],
        TIMEOUT,
    );

    assert_eq!(run.status.code(), Some(2), "{}", run.stderr);
    assert!(run.stderr.contains("from --environment"), "{}", run.stderr);
    assert!(!probe.was_connected());
}

/// Positive control: a production command that the pre-check allows does
/// load config/production.yaml and does try to connect, so the probe works.
#[test]
fn an_allowed_production_command_reaches_the_database() {
    let probe = ConnectionProbe::bind("127.0.0.1");
    let url = format!(
        "postgres://loco:loco@127.0.0.1:{}/backend3_test",
        probe.port()
    );
    let env = production_env(&url);

    let run = run_cli(&as_refs(&env), &["db", "status"], TIMEOUT);

    assert!(
        !run.status.success(),
        "the probe never answers, so the command fails"
    );
    assert!(!run.stderr.contains("Refusing"), "{}", run.stderr);
    assert!(
        probe.was_connected(),
        "db status should have connected: {}",
        run.stderr
    );
}

/// ADR 0039: in production `doctor --config` stops before anything is
/// loaded or connected, in each of its spellings, and prints nothing on
/// stdout, where the configuration would go.
#[test]
fn doctor_config_is_refused_in_production_before_connecting() {
    for args in [
        &["doctor", "--config"][..],
        &["doctor", "-c"][..],
        &["doctor", "-pc"][..],
    ] {
        let probe = ConnectionProbe::bind("127.0.0.1");
        let url = format!(
            "postgres://loco:loco@127.0.0.1:{}/backend3_test",
            probe.port()
        );
        let env = production_env(&url);

        let run = run_cli(&as_refs(&env), args, TIMEOUT);

        assert_eq!(run.status.code(), Some(2), "{args:?}: {}", run.stderr);
        assert!(
            run.stderr
                .contains("Refusing to run `doctor --config`: the environment is production"),
            "{args:?}: {}",
            run.stderr
        );
        assert!(run.stdout.is_empty(), "{args:?} printed: {}", run.stdout);
        for (key, value) in &env {
            if ["JWT_SECRET", "TOTP_ENCRYPTION_KEY"].contains(key) {
                assert!(
                    !run.stderr.contains(value.as_str()),
                    "{key}: {}",
                    run.stderr
                );
            }
        }
        assert!(!probe.was_connected(), "{args:?} connected to the database");
    }
}

/// loco-rs 1.2.0 reads `doctor --production` as `--environment production`,
/// so the pre-check refuses `doctor --config --production` even when the
/// process environment says development.
#[test]
fn doctor_production_selects_production_for_doctor_config() {
    let probe = ConnectionProbe::bind("127.0.0.1");
    let url = format!(
        "postgres://loco:loco@127.0.0.1:{}/backend3_test",
        probe.port()
    );
    let mut env = production_env(&url);
    env.retain(|(key, _)| *key != "LOCO_ENV");
    env.push(("LOCO_ENV", "development".to_owned()));

    let run = run_cli(
        &as_refs(&env),
        &["doctor", "--config", "--production"],
        TIMEOUT,
    );

    assert_eq!(run.status.code(), Some(2), "{}", run.stderr);
    assert!(
        run.stderr.contains("from doctor --production"),
        "{}",
        run.stderr
    );
    assert!(run.stdout.is_empty(), "printed: {}", run.stdout);
    assert!(!probe.was_connected());
}

/// Positive control: `doctor` without `--config` is allowed in production
/// and does load the configuration and try the database, so the refusal
/// above is the pre-check's doing.
#[test]
fn doctor_without_config_reaches_the_database_in_production() {
    let probe = ConnectionProbe::bind("127.0.0.1");
    let url = format!(
        "postgres://loco:loco@127.0.0.1:{}/backend3_test",
        probe.port()
    );
    let env = production_env(&url);

    let run = run_cli(&as_refs(&env), &["doctor"], TIMEOUT);

    assert!(!run.stderr.contains("Refusing"), "{}", run.stderr);
    assert!(
        probe.was_connected(),
        "doctor should have connected: {}",
        run.stderr
    );
}

/// Why production refuses it: where the pre-check allows `doctor --config`,
/// in the test environment against the local test database, loco-rs 1.2.0
/// prints the whole configuration, the session secret, the TOTP key and the
/// database password included. A Loco release that stops printing them
/// fails this test, and ADR 0039 can then be revisited.
#[test]
fn doctor_config_prints_the_secrets_where_it_is_allowed() {
    let config = Environment::Test.load().expect("config/test.yaml");
    let jwt_secret = config.get_jwt_config().expect("auth.jwt").secret.clone();
    let totp_key = config.settings.as_ref().expect("settings")["admin_auth"]["totp_encryption_key"]
        .as_str()
        .expect("a string")
        .to_owned();

    // The configured database of config/test.yaml, which exists in every
    // run of the tests (compose.yaml and the CI Test job create it).
    let run = run_cli(&[("LOCO_ENV", "test")], &["doctor", "--config"], TIMEOUT);

    assert!(run.status.success(), "{}", run.stderr);
    for (what, value) in [
        ("the session secret", jwt_secret.as_str()),
        ("the TOTP encryption key", totp_key.as_str()),
        ("the database password", "postgres://loco:loco@"),
    ] {
        assert!(
            run.stdout.contains(value),
            "{what} is printed: {}",
            run.stdout
        );
    }
}
