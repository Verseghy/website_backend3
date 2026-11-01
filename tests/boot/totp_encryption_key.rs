//! Production refuses to start without a usable TOTP encryption key
//! (ADR 0006 §6): every command fails before it connects anywhere when
//! `TOTP_ENCRYPTION_KEY` is unset, empty, not base64 or shorter than 64
//! bytes, with a message that names the key and never shows its value. It
//! refuses the public throwaway key of the local configuration files in the
//! same way (ADR 0034 §3).
//!
//! The variable is read by `config/production.yaml` through `get_env`, so
//! these tests run the built `backend3-cli` with an explicit environment
//! (tests/support `run_cli`) instead of changing their own. The database
//! points at a [`ConnectionProbe`]; the positive control shows that a valid
//! key gets as far as connecting. Every valid TOTP key here is made when the
//! test runs ([`cli_totp_key`]), because production refuses a committed key
//! (ADR 0034 §3). So is the session secret ([`cli_jwt_secret`]), for the
//! same reason.

use std::time::Duration;

use backend3::domain::totp_secret::{PUBLISHED_LOCAL_KEY, TotpEncryptionKey};
use base64::{Engine as _, engine::general_purpose::STANDARD};

use crate::support::{CliRun, ConnectionProbe, cli_jwt_secret, cli_totp_key, run_cli};

const TIMEOUT: Duration = Duration::from_secs(60);

/// Runs `db status` in production with `totp_key` as `TOTP_ENCRYPTION_KEY`
/// (unset for `None`) and every other variable that config/production.yaml
/// requires, and returns the run with whether the database probe saw a
/// connection.
fn db_status_with(totp_key: Option<&str>) -> (CliRun, bool) {
    let probe = ConnectionProbe::bind("127.0.0.1");
    let url = format!(
        "postgres://loco:loco@127.0.0.1:{}/backend3_test",
        probe.port()
    );
    let jwt_secret = cli_jwt_secret();
    let mut env = vec![
        ("LOCO_ENV", "production"),
        ("DATABASE_URL", url.as_str()),
        ("HOST", "http://localhost"),
        // The session secret that config/production.yaml also requires
        // (ADR 0006 §1), so that only the TOTP key decides the outcome.
        ("JWT_SECRET", jwt_secret.as_str()),
        // The probe never answers the PostgreSQL handshake; give up fast.
        ("DB_CONNECT_TIMEOUT", "300"),
    ];
    if let Some(totp_key) = totp_key {
        env.push(("TOTP_ENCRYPTION_KEY", totp_key));
    }
    let run = run_cli(&env, &["db", "status"], TIMEOUT);
    let connected = probe.was_connected();
    (run, connected)
}

#[test]
fn an_unset_key_stops_production_before_connecting() {
    let (run, connected) = db_status_with(None);
    assert!(!run.status.success(), "{}", run.stderr);
    assert!(
        run.stderr
            .contains("environment variable `TOTP_ENCRYPTION_KEY` not found"),
        "{}",
        run.stderr
    );
    assert!(!connected, "connected without a TOTP key");
}

/// Every unusable value, including the ones YAML treats specially inside
/// the double-quoted string of config/production.yaml (quotes, backslashes,
/// line breaks), reaches the typed check and gets its message.
#[test]
fn an_unusable_key_stops_production_and_names_the_key() {
    let short = STANDARD.encode([0x5a_u8; 40]);
    // The broken forms of a valid key start from one made now, so this file
    // holds no key that production would accept once repaired.
    let valid = cli_totp_key();
    // How `openssl rand -base64 64` prints it: wrapped after 64 characters.
    let wrapped = format!("{}\n{}", &valid[..64], &valid[64..]);
    let trailing_newline = format!("{valid}\n");
    let pasted = "Wrong-Secret-Pasted-Here-0123456789-0123456789-0123456789-0123";
    for (value, problem) in [
        ("", "is empty"),
        (pasted, "is not valid base64"),
        (short.as_str(), "decodes to 40 bytes"),
        ("Pasted'Single'Quote", "is not valid base64"),
        ("Pasted\"Double\"Quote", "is not valid base64"),
        ("Pasted\\Back\\Slash\\", "is not valid base64"),
        (wrapped.as_str(), "unexpected character at position 64"),
        (
            trailing_newline.as_str(),
            "unexpected character at position 88",
        ),
    ] {
        let (run, connected) = db_status_with(Some(value));
        assert!(!run.status.success(), "{value:?}: {}", run.stderr);
        assert!(
            run.stderr.contains(
                "invalid configuration at `settings.admin_auth.totp_encryption_key`: the TOTP \
                 encryption key (TOTP_ENCRYPTION_KEY in production)"
            ),
            "{value:?}: {}",
            run.stderr
        );
        assert!(run.stderr.contains(problem), "{value:?}: {}", run.stderr);
        for line in value.lines() {
            assert!(
                !run.stderr.contains(line),
                "the value is never printed: {}",
                run.stderr
            );
        }
        assert!(!connected, "{value:?}: connected with an unusable key");
    }
}

/// ADR 0034 §3: the key that config/development.yaml, config/test.yaml and
/// config/e2e.yaml publish is valid base64 of more than 64 bytes, so only
/// the published-key check stops it. The message names the key and the
/// variable, says why and where the key comes from, and does not repeat
/// the value.
#[test]
fn the_published_local_key_stops_production_before_connecting() {
    let (run, connected) = db_status_with(Some(PUBLISHED_LOCAL_KEY));
    assert!(!run.status.success(), "{}", run.stderr);
    assert!(
        run.stderr.contains(
            "refusing to start: settings.admin_auth.totp_encryption_key (TOTP_ENCRYPTION_KEY) \
             is the public throwaway key of config/development.yaml, config/test.yaml and \
             config/e2e.yaml, which production refuses; set TOTP_ENCRYPTION_KEY to the key \
             from the secret store (the backend-secrets key backend3-totp-key)"
        ),
        "{}",
        run.stderr
    );
    assert!(
        !run.stderr.contains(PUBLISHED_LOCAL_KEY),
        "the value is never printed: {}",
        run.stderr
    );
    assert!(!connected, "connected with the published key");
}

/// Positive control: with a valid key made when the test runs, the same
/// command gets past the configuration and tries the database, so the
/// refusals above are the key check's doing (ADR 0034 §3).
#[test]
fn a_valid_key_lets_production_reach_the_database() {
    let key = cli_totp_key();
    let (run, connected) = db_status_with(Some(&key));
    assert!(
        !run.status.success(),
        "the probe never answers, so the command fails"
    );
    assert!(
        !run.stderr.contains("totp_encryption_key"),
        "{}",
        run.stderr
    );
    assert!(connected, "db status should have connected: {}", run.stderr);
}

/// The key that the production tests use ([`cli_totp_key`]) is one that
/// production accepts and no committed file holds (ADR 0034 §3): canonical
/// base64 of exactly 64 bytes, not the published key, and new on every
/// call, so it cannot be a fixed value copied into the tests.
#[test]
fn the_run_time_key_is_valid_unpublished_and_new_each_time() {
    let first = cli_totp_key();
    let decoded = STANDARD.decode(&first).expect("canonical base64");
    assert_eq!(decoded.len(), 64, "{first}");
    let key = TotpEncryptionKey::from_base64(&first).expect("a valid configured key");
    assert!(!key.is_published_local_key());
    assert_ne!(first, PUBLISHED_LOCAL_KEY);
    // Two keys from one process differ; a fixed value fails here.
    assert_ne!(first, cli_totp_key());
}
