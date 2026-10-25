//! config/production.yaml takes the admin session secret from `JWT_SECRET`
//! with no default (ADR 0006 §1, ADR 0023 §11), so production refuses to
//! start without it, before any database connection. [`super::precheck`]'s
//! positive control shows that the same environment with
//! [`crate::support::CLI_JWT_SECRET`] does connect.

use std::time::Duration;

use crate::support::{ConnectionProbe, cli_totp_key, run_cli};

const TIMEOUT: Duration = Duration::from_secs(60);

#[test]
fn production_refuses_to_start_without_a_jwt_secret() {
    let probe = ConnectionProbe::bind("127.0.0.1");
    let url = format!(
        "postgres://loco:loco@127.0.0.1:{}/backend3_test",
        probe.port()
    );
    // Every other variable that config/production.yaml requires is set, so
    // the missing secret alone stops the command. The TOTP key is made when
    // the test runs, because production refuses a committed one
    // (ADR 0034 §3).
    let totp_key = cli_totp_key();
    let env = [
        ("LOCO_ENV", "production"),
        ("DATABASE_URL", url.as_str()),
        ("HOST", "http://localhost"),
        ("TOTP_ENCRYPTION_KEY", totp_key.as_str()),
    ];

    let run = run_cli(&env, &["db", "status"], TIMEOUT);

    assert!(!run.status.success());
    assert!(
        run.stderr
            .contains("environment variable `JWT_SECRET` not found"),
        "{}",
        run.stderr
    );
    assert!(!probe.was_connected(), "connected to the database");
}
