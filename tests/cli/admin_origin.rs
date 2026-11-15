//! Production takes the admin origin, `settings.admin_auth.origin`, from
//! `HOST`, the public URL of the app (ADR 0014; ADR 0006 §3 as amended by
//! ADR 0044). The `Origin` check compares it byte for byte with the header
//! browsers send, so every command stops before it connects anywhere when
//! `HOST` is not an origin in that spelling, with a message that names the
//! key and the variable and says what to write: such a value would refuse
//! every cookie-authenticated admin write.
//!
//! The variable is read by config/production.yaml through `get_env`, so
//! these tests run the built `backend3-cli` with an explicit environment
//! (tests/support `run_cli`); the database points at a [`ConnectionProbe`],
//! and the positive control shows that production's own value gets as far as
//! connecting.

use std::time::Duration;

use crate::support::{
    CLI_MEDIA_PUBLIC_BASE_URL, CliRun, ConnectionProbe, cli_jwt_secret, cli_totp_key, run_cli,
};

const TIMEOUT: Duration = Duration::from_secs(60);

/// Runs `db status` in production with every variable that
/// config/production.yaml requires and `HOST` set to `host`, and returns the
/// run with whether the database probe saw a connection.
fn db_status_with(host: &str) -> (CliRun, bool) {
    let probe = ConnectionProbe::bind("127.0.0.1");
    let url = format!(
        "postgres://loco:loco@127.0.0.1:{}/backend3_test",
        probe.port()
    );
    // Production refuses a committed key, so both keys are made now
    // (ADR 0034 §3).
    let jwt_secret = cli_jwt_secret();
    let totp_key = cli_totp_key();
    let env = [
        ("LOCO_ENV", "production"),
        ("DATABASE_URL", url.as_str()),
        ("HOST", host),
        ("JWT_SECRET", jwt_secret.as_str()),
        ("TOTP_ENCRYPTION_KEY", totp_key.as_str()),
        ("MEDIA_PUBLIC_BASE_URL", CLI_MEDIA_PUBLIC_BASE_URL),
        // The probe never answers the PostgreSQL handshake; give up fast.
        ("DB_CONNECT_TIMEOUT", "300"),
    ];
    let run = run_cli(&env, &["db", "status"], TIMEOUT);
    let connected = probe.was_connected();
    (run, connected)
}

#[test]
fn a_host_that_is_no_browser_origin_stops_production_and_names_the_key() {
    for (host, fix) in [
        (
            "backend3.microshift.verseghy-gimnazium.net",
            "is not an absolute URL",
        ),
        (
            "https://backend3.microshift.verseghy-gimnazium.net/",
            "write only the scheme, the host and the port, as \
             https://backend3.microshift.verseghy-gimnazium.net",
        ),
        (
            "https://backend3.microshift.verseghy-gimnazium.net/admin",
            "is not an origin",
        ),
        (
            "https://Backend3.microshift.verseghy-gimnazium.net:443",
            "write it as https://backend3.microshift.verseghy-gimnazium.net",
        ),
        // Quotes and backslashes reach the typed check through the escaping
        // filters of config/production.yaml.
        ("https://example.org/\"\\", "is not an origin"),
    ] {
        let (run, connected) = db_status_with(host);
        assert!(!run.status.success(), "{host}: {}", run.stderr);
        assert!(
            run.stderr
                .contains("invalid configuration at `settings.admin_auth.origin`"),
            "{host}: {}",
            run.stderr
        );
        assert!(
            run.stderr.contains("HOST in production"),
            "{host}: {}",
            run.stderr
        );
        assert!(run.stderr.contains(fix), "{host}: {}", run.stderr);
        assert!(
            !connected,
            "{host}: connected with an unusable admin origin"
        );
    }
}

#[test]
fn an_empty_host_stops_production_before_connecting() {
    // `server.host` reads the same variable, and Loco refuses its empty
    // value first, when it parses the file.
    let (run, connected) = db_status_with("");
    assert!(!run.status.success(), "{}", run.stderr);
    assert!(
        run.stderr.contains("cannot parse `config/production.yaml`"),
        "{}",
        run.stderr
    );
    assert!(!connected, "connected without a host");
}

#[test]
fn production_s_own_host_gets_as_far_as_connecting() {
    // The positive control: the value ADR 0014 gives production.
    let (run, connected) = db_status_with("https://backend3.microshift.verseghy-gimnazium.net");
    assert!(
        !run.status.success(),
        "the probe never completes a handshake"
    );
    assert!(!run.stderr.contains("admin_auth.origin"), "{}", run.stderr);
    assert!(connected, "{}", run.stderr);
}
