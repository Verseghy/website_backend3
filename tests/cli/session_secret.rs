//! The admin session's JWT configuration in production (ADR 0006 §1,
//! ADR 0023 §11, ADR 0034 §3; D92 and D75, research: DECISIONS R15).
//!
//! config/production.yaml takes the session secret from `JWT_SECRET` with
//! no default, and every command refuses to start, before any database
//! connection, when the variable is unset, when its value is not base64 of
//! at least 64 bytes, and when it is one of the public throwaway secrets
//! that this repository publishes. The lifetime and the token locations are
//! fixed in the file; a copy of it with either changed is refused the same
//! way. Every message names the key and never shows the value.
//!
//! Every secret production should accept here is made when the test runs
//! ([`cli_jwt_secret`]), and so is the TOTP key ([`cli_totp_key`]), because
//! production refuses a committed key (ADR 0034 §3). Each test points the
//! database at a [`ConnectionProbe`]; the positive controls show that a
//! valid configuration gets as far as connecting.

use std::{
    fs,
    path::{Path, PathBuf},
    time::Duration,
};

use backend3::domain::session_jwt::{JwtSecret, PUBLISHED_JWT_SECRETS};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use loco_rs::environment::Environment;
use sea_orm::prelude::Uuid;

use crate::support::{
    CliRun, ConnectionProbe, cli_jwt_secret, cli_totp_key, load_committed_config, run_cli,
};

const TIMEOUT: Duration = Duration::from_secs(60);

/// The message prefix of a session secret that is not usable.
const UNUSABLE: &str =
    "invalid configuration at `auth.jwt.secret`: the session secret (JWT_SECRET in production)";

/// Runs `db status` in production with `jwt_secret` as `JWT_SECRET` (unset
/// for `None`), every other variable that config/production.yaml requires,
/// and `config_folder` as `LOCO_CONFIG_FOLDER` when given. Returns the run
/// with whether the database probe saw a connection.
fn db_status_with(jwt_secret: Option<&str>, config_folder: Option<&Path>) -> (CliRun, bool) {
    let probe = ConnectionProbe::bind("127.0.0.1");
    let url = format!(
        "postgres://loco:loco@127.0.0.1:{}/backend3_test",
        probe.port()
    );
    let totp_key = cli_totp_key();
    let folder = config_folder.map(|folder| folder.to_str().expect("a UTF-8 path").to_owned());
    let mut env = vec![
        ("LOCO_ENV", "production"),
        ("DATABASE_URL", url.as_str()),
        ("HOST", "http://localhost"),
        ("TOTP_ENCRYPTION_KEY", totp_key.as_str()),
        // The probe never answers the PostgreSQL handshake; give up fast.
        ("DB_CONNECT_TIMEOUT", "300"),
    ];
    if let Some(jwt_secret) = jwt_secret {
        env.push(("JWT_SECRET", jwt_secret));
    }
    if let Some(folder) = &folder {
        // loco-rs 1.2.0 reads `<folder>/<env>.yaml` instead of `config/`
        // (`Environment::load`, `LOCO_CONFIG_FOLDER`).
        env.push(("LOCO_CONFIG_FOLDER", folder.as_str()));
    }
    let run = run_cli(&env, &["db", "status"], TIMEOUT);
    let connected = probe.was_connected();
    (run, connected)
}

/// Asserts that `run` stopped before connecting, with `message` on stderr
/// and nothing of `value` in its output.
fn assert_refused(run: &CliRun, connected: bool, message: &str, value: &str) {
    assert!(!run.status.success(), "{value:?}: {}", run.stderr);
    assert!(run.stderr.contains(message), "{value:?}: {}", run.stderr);
    for line in value.lines().filter(|line| !line.is_empty()) {
        assert!(
            !run.stderr.contains(line) && !run.stdout.contains(line),
            "the value is never printed: {}",
            run.stderr
        );
    }
    assert!(!connected, "{value:?}: connected: {}", run.stderr);
}

#[test]
fn production_refuses_to_start_without_a_jwt_secret() {
    let (run, connected) = db_status_with(None, None);
    assert_refused(
        &run,
        connected,
        "environment variable `JWT_SECRET` not found",
        "",
    );
}

/// Every unusable value, including the ones YAML treats specially inside
/// the double-quoted string of config/production.yaml (quotes, backslashes,
/// line breaks), reaches the typed check and gets its message, instead of a
/// `500` at the first correct sign-in.
#[test]
fn an_unusable_secret_stops_production_and_names_the_key() {
    let short = STANDARD.encode([0x5a_u8; 40]);
    // The broken forms of a valid secret start from one made now, so this
    // file holds no secret that production would accept once repaired.
    let valid = cli_jwt_secret();
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
        let (run, connected) = db_status_with(Some(value), None);
        assert_refused(&run, connected, UNUSABLE, value);
        assert!(run.stderr.contains(problem), "{value:?}: {}", run.stderr);
    }
}

/// The limit that the `auth.jwt` comment of config/production.yaml states:
/// the escaping lets every printable value reach the typed check (a tab
/// too), while a control character other than tab, CR or LF stops the boot
/// earlier. That error comes from the YAML parser: it names the file and a
/// position, not `auth.jwt.secret`, and shows nothing of the value. Either
/// way production stops before connecting.
#[test]
fn a_control_character_stops_production_before_the_typed_check() {
    let tab = "Pasted\tTab\tSecret";
    let (run, connected) = db_status_with(Some(tab), None);
    assert_refused(&run, connected, UNUSABLE, tab);

    let control = "Pasted\u{1}Control\u{1}Secret";
    let (run, connected) = db_status_with(Some(control), None);
    assert_refused(&run, connected, "production.yaml", control);
    assert!(
        run.stderr.contains("control characters are not allowed"),
        "{}",
        run.stderr
    );
    assert!(!run.stderr.contains("auth.jwt"), "{}", run.stderr);
    for part in ["Pasted", "Control", "Secret"] {
        assert!(!run.stderr.contains(part), "{part}: {}", run.stderr);
    }
}

/// ADR 0034 §3: each JWT secret this repository publishes is valid base64
/// of more than 64 bytes, so only the published-key refusal stops it. The
/// values come from the files that publish them: the three local
/// configurations, and, for the constant that left `tests/support`, the
/// refusal's own list. The message names the key, the variable and where
/// the value is published, says where the real secret comes from, and does
/// not repeat the value.
#[test]
fn every_published_secret_stops_production_before_connecting() {
    let mut cases: Vec<(String, String)> = [
        Environment::Development,
        Environment::Test,
        Environment::Any("e2e".to_owned()),
    ]
    .into_iter()
    .map(|environment| {
        // The committed file alone: a developer's own `<env>.local.yaml`
        // may set a secret of their own, which no file publishes.
        let config = load_committed_config(&environment);
        let secret = config.get_jwt_config().expect("auth.jwt").secret.clone();
        (secret, format!("config/{environment}.yaml"))
    })
    .collect();
    let former_constant = PUBLISHED_JWT_SECRETS
        .iter()
        .find(|published| published.source.contains("CLI_JWT_SECRET"))
        .expect("the former CLI_JWT_SECRET stays published in the history, so stays refused");
    cases.push((
        STANDARD.encode(former_constant.text),
        former_constant.source.to_owned(),
    ));
    for (secret, source) in cases {
        // Valid as a secret: only the refusal can stop it.
        JwtSecret::from_base64(&secret).unwrap_or_else(|error| panic!("{source}: {error}"));
        let (run, connected) = db_status_with(Some(&secret), None);
        let message = format!(
            "refusing to start: auth.jwt.secret (JWT_SECRET) is the public throwaway secret of \
             {source}, which production refuses; set JWT_SECRET to the secret from the secret \
             store (the backend-secrets key backend3-jwt-secret)"
        );
        assert_refused(&run, connected, &message, &secret);
    }
}

/// Positive control: with a secret made when the test runs, the same
/// command gets past the configuration and tries the database, so the
/// refusals above are the secret check's doing (ADR 0034 §3).
#[test]
fn a_run_time_secret_lets_production_reach_the_database() {
    let secret = cli_jwt_secret();
    let (run, connected) = db_status_with(Some(&secret), None);
    assert!(
        !run.status.success(),
        "the probe never answers, so the command fails"
    );
    assert!(!run.stderr.contains("auth.jwt"), "{}", run.stderr);
    assert!(connected, "db status should have connected: {}", run.stderr);
}

/// The secret that the production tests use ([`cli_jwt_secret`]) is one
/// that production accepts and no committed file holds (ADR 0034 §3):
/// canonical base64 of exactly 64 bytes, not a published secret, and new on
/// every call, so it cannot be a fixed value copied into the tests.
#[test]
fn the_run_time_secret_is_valid_unpublished_and_new_each_time() {
    let first = cli_jwt_secret();
    let decoded = STANDARD.decode(&first).expect("canonical base64");
    assert_eq!(decoded.len(), 64, "{first}");
    let secret = JwtSecret::from_base64(&first).expect("a valid configured secret");
    assert_eq!(secret.published_in(), None);
    // Two secrets from one process differ; a fixed value fails here.
    assert_ne!(first, cli_jwt_secret());
}

/// A copy of config/production.yaml in a directory of its own under the
/// cargo target directory, removed when it is dropped, also when the test
/// fails.
struct ProductionCopy(PathBuf);

impl ProductionCopy {
    /// Copies config/production.yaml with `from` replaced by `to`, and
    /// fails when `from` is not in the file, so a reworded file cannot turn
    /// the test into a copy of the original.
    fn with(from: &str, to: &str) -> Self {
        let source =
            fs::read_to_string(crate::support::repository_root().join("config/production.yaml"))
                .expect("config/production.yaml");
        assert!(
            source.contains(from),
            "config/production.yaml has no {from:?}"
        );
        // CARGO_TARGET_TMPDIR is cargo's scratch directory for integration
        // tests, inside the target directory.
        let folder =
            Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("production-{}", Uuid::new_v4()));
        fs::create_dir_all(&folder).expect("create the configuration folder");
        fs::write(folder.join("production.yaml"), source.replace(from, to))
            .expect("write the copy");
        Self(folder)
    }
}

impl Drop for ProductionCopy {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// D92's boot check also covers the two values that config/production.yaml
/// fixes: a lifetime other than two hours, or a token read from anywhere
/// but the session cookie, then the Bearer header, stops every command with
/// the key named. The unchanged copy is the positive control: the folder
/// itself is not what stops the others.
#[test]
fn a_changed_lifetime_or_token_location_stops_production() {
    let cookie_then_bearer =
        "      - from: Cookie\n        name: __Host-admin_session\n      - from: Bearer\n";
    let cases = [
        (
            "    expiration: 7200\n",
            "    expiration: 604800\n",
            Some(
                "invalid configuration at `auth.jwt.expiration`: is 604800 seconds; the admin \
                 session lasts 7200 seconds",
            ),
        ),
        (
            cookie_then_bearer,
            "      - from: Bearer\n",
            Some(
                "invalid configuration at `auth.jwt.location`: reads the token from the Bearer \
                 header; it must list the cookie __Host-admin_session first, then Bearer",
            ),
        ),
        (
            cookie_then_bearer,
            "      - from: Cookie\n        name: __Host-admin_session\n      - from: Bearer\n      \
             - from: Query\n        name: token\n",
            Some("then the query parameter token; it must list"),
        ),
        ("    expiration: 7200\n", "    expiration: 7200\n", None),
    ];
    let secret = cli_jwt_secret();
    for (from, to, refused) in cases {
        let copy = ProductionCopy::with(from, to);
        let (run, connected) = db_status_with(Some(&secret), Some(&copy.0));
        assert!(!run.status.success(), "{}", run.stderr);
        if let Some(message) = refused {
            assert_refused(&run, connected, message, &secret);
        } else {
            assert!(!run.stderr.contains("auth.jwt"), "{}", run.stderr);
            assert!(connected, "the unchanged copy connects: {}", run.stderr);
        }
    }
}
