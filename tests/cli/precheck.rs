//! With `LOCO_ENV=production`, `backend3-cli db reset` (and `db truncate`,
//! `db seed --reset`) exits with an error before it connects
//! (ADR 0024 §4, "What must be true in code and tests").

use std::time::Duration;

use crate::support::{CLI_JWT_SECRET, ConnectionProbe, cli_totp_key, run_cli};

const TIMEOUT: Duration = Duration::from_secs(60);

/// The environment of a production pod, with the database pointed at
/// `probe`. `HOST`, `JWT_SECRET` and `TOTP_ENCRYPTION_KEY` are required by
/// config/production.yaml; the TOTP key is made when the test runs, because
/// production refuses a committed one (ADR 0034 §3).
fn production_env(database_url: &str) -> Vec<(&'static str, String)> {
    vec![
        ("LOCO_ENV", "production".to_owned()),
        ("DATABASE_URL", database_url.to_owned()),
        ("HOST", "http://localhost".to_owned()),
        ("JWT_SECRET", CLI_JWT_SECRET.to_owned()),
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
